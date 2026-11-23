//! Latest-request-wins resource construction on a bounded worker. No audio-thread I/O.
//! Native CLAP owners are deliberately outside this loader's Send-only factory.
use crate::{
    core::ProcessSpec,
    plugin::{PluginDescriptor, PluginInstance},
    state::{ProjectState, StateError},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

pub enum LoadRequest {
    Sfz {
        path: PathBuf,
        product: String,
    },
    File(PathBuf),
    Snapshot {
        state: Box<ProjectState>,
        root: PathBuf,
    },
    Bundle {
        path: PathBuf,
        destination: PathBuf,
    },
    Recovery {
        directory: PathBuf,
        product: String,
    },
}
pub struct LoadedProject {
    state: ProjectState,
    root: PathBuf,
    plugin: PluginInstance,
    prepared: Option<ProcessSpec>,
}
impl LoadedProject {
    pub fn state(&self) -> &ProjectState {
        &self.state
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn prepared_spec(&self) -> Option<ProcessSpec> {
        self.prepared
    }
    pub fn into_parts(self) -> (ProjectState, PathBuf, PluginInstance) {
        (self.state, self.root, self.plugin)
    }
    /// Preserves preparation when a graph uses the exact worker-prepared spec.
    pub fn into_processor(self) -> crate::project::ProjectProcessor {
        crate::project::ProjectProcessor::from_prepared(self.plugin, self.prepared)
    }
}
pub struct LoadOutcome {
    pub generation: u64,
    pub result: Result<LoadedProject, String>,
}
struct Shared {
    generation: u64,
    ready: Option<LoadOutcome>,
}
struct Job {
    generation: u64,
    request: LoadRequest,
    spec: Option<ProcessSpec>,
}
pub struct AsyncProjectLoader {
    sender: Option<SyncSender<Job>>,
    shared: Arc<Mutex<Shared>>,
    worker: Option<JoinHandle<()>>,
}
impl AsyncProjectLoader {
    pub fn new(descriptor: PluginDescriptor) -> Self {
        Self::with_factory(move |s, root| {
            crate::project::create_plugin(descriptor, s.clone(), root)
        })
    }
    /// Factory and its resource destruction execute on the worker. Keep it free
    /// of main-thread-only native plug-in/window objects.
    pub fn with_factory<F>(factory: F) -> Self
    where
        F: Fn(&ProjectState, &Path) -> Result<PluginInstance, StateError> + Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel::<Job>(2);
        let shared = Arc::new(Mutex::new(Shared {
            generation: 0,
            ready: None,
        }));
        let worker_shared = shared.clone();
        let worker = thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                if worker_shared.lock().unwrap().generation != job.generation {
                    continue;
                }
                let result = (|| -> Result<LoadedProject, StateError> {
                    let (state, root) = match job.request {
                        LoadRequest::Sfz { path, product } => {
                            crate::sampling::import_sfz(&path, &product)?
                        }
                        LoadRequest::Bundle { path, destination } => {
                            let state = crate::project_storage::unpack_bundle(&path, &destination)?;
                            (state, destination.canonicalize()?)
                        }
                        LoadRequest::Recovery { directory, product } => {
                            let s = crate::project_storage::RecoveryStore::new(directory, 5)?
                                .latest(&product)?
                                .ok_or("no valid recovery snapshot")?;
                            (s.project, s.asset_root.canonicalize()?)
                        }
                        LoadRequest::Snapshot { state, root } => (*state, root.canonicalize()?),
                        LoadRequest::File(path) => {
                            let root = path
                                .parent()
                                .filter(|p| !p.as_os_str().is_empty())
                                .unwrap_or(Path::new("."))
                                .canonicalize()?;
                            let f = std::fs::File::open(path)?;
                            use std::io::Read;
                            let mut bytes = Vec::new();
                            f.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
                            if bytes.len() > 4 * 1024 * 1024 {
                                return Err("project exceeds 4 MiB".into());
                            }
                            (ProjectState::from_json(std::str::from_utf8(&bytes)?)?, root)
                        }
                    };
                    state.validate(&state.product_id)?;
                    let mut plugin = factory(&state, &root)?;
                    let state = plugin.project_state().cloned().unwrap_or(state);
                    if let Some(spec) = job.spec {
                        plugin.activate(spec)?;
                    }
                    Ok(LoadedProject {
                        state,
                        root,
                        plugin,
                        prepared: job.spec,
                    })
                })()
                .map_err(|e| e.to_string());
                let mut shared = worker_shared.lock().unwrap();
                if shared.generation == job.generation {
                    shared.ready = Some(LoadOutcome {
                        generation: job.generation,
                        result,
                    });
                }
                // Stale results and replaced resources are destroyed here, never in a callback.
            }
        });
        Self {
            sender: Some(tx),
            shared,
            worker: Some(worker),
        }
    }
    /// Nonblocking queue submission; a full queue leaves the previous request valid.
    pub fn submit(
        &mut self,
        request: LoadRequest,
        spec: Option<ProcessSpec>,
    ) -> Result<u64, &'static str> {
        let mut shared = self.shared.lock().unwrap();
        let generation = shared
            .generation
            .checked_add(1)
            .ok_or("loader generation exhausted")?;
        match self.sender.as_ref().ok_or("loader stopped")?.try_send(Job {
            generation,
            request,
            spec,
        }) {
            Ok(()) => {
                shared.generation = generation;
                shared.ready = None;
                Ok(generation)
            }
            Err(TrySendError::Full(_)) => Err("project loader busy"),
            Err(TrySendError::Disconnected(_)) => Err("project loader stopped"),
        }
    }
    pub fn try_poll(&mut self) -> Option<LoadOutcome> {
        self.shared.lock().unwrap().ready.take()
    }
    /// Invalidates pending results; does not interrupt filesystem I/O in progress.
    pub fn cancel(&mut self) {
        let mut shared = self.shared.lock().unwrap();
        shared.generation = shared.generation.saturating_add(1);
        shared.ready = None;
    }
}
impl Drop for AsyncProjectLoader {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Bounded optional autosave worker. Submission, polling and shutdown belong to
/// the editor/control thread. Shutdown waits for outstanding disk operations.
pub struct AutosaveWorker {
    sender: Option<SyncSender<crate::project_storage::RecoverySnapshot>>,
    result: mpsc::Receiver<Result<PathBuf, String>>,
    worker: Option<JoinHandle<()>>,
}
impl AutosaveWorker {
    pub fn new(store: crate::project_storage::RecoveryStore) -> Self {
        let (tx, rx) = mpsc::sync_channel::<crate::project_storage::RecoverySnapshot>(1);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            while let Ok(s) = rx.recv() {
                let result = store
                    .checkpoint(&s.project, &s.asset_root)
                    .map_err(|e| e.to_string());
                let _ = done_tx.try_send(result);
            }
        });
        Self {
            sender: Some(tx),
            result: done_rx,
            worker: Some(worker),
        }
    }
    pub fn try_checkpoint(&self, state: ProjectState, root: PathBuf) -> Result<(), &'static str> {
        self.sender
            .as_ref()
            .ok_or("autosave stopped")?
            .try_send(crate::project_storage::RecoverySnapshot {
                project: state,
                asset_root: root,
            })
            .map_err(|_| "autosave busy or stopped")
    }
    pub fn try_poll(&self) -> Option<Result<PathBuf, String>> {
        self.result.try_recv().ok()
    }
}
impl Drop for AutosaveWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
