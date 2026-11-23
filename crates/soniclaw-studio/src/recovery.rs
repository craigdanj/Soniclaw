//! Verified portable recovery snapshots and bounded, opt-in background autosave. No native code is loaded.
use crate::{
    project::{Project, ProjectError},
    *,
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const COMMIT: &str = "recovery.json";
const MARKER: &str = "store.json";
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub snapshots: usize,
    pub retained_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            snapshots: 5,
            retained_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    project_id: String,
    limits: Limits,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreMarker {
    schema: u32,
    project_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedFile {
    file: String,
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    schema: u32,
    project_id: String,
    generation: u64,
    files: Vec<OwnedFile>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub directory: PathBuf,
    pub generation: u64,
    pub bytes: u64,
}
pub struct Inventory {
    pub snapshots: Vec<Snapshot>,
    pub warnings: Vec<String>,
}
pub struct Restored {
    pub project: Project,
    pub snapshot: Snapshot,
    pub warnings: Vec<String>,
}
pub struct Saved {
    pub snapshot: Snapshot,
    pub warnings: Vec<String>,
}
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, limit: u64) -> Result<T, ProjectError> {
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err("recovery metadata too large".into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn hash(path: &Path) -> Result<String, ProjectError> {
    let mut file = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        h.update(&bytes[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn collect(
    root: &Path,
    at: &Path,
    out: &mut Vec<OwnedFile>,
    depth: usize,
) -> Result<(), ProjectError> {
    if depth > 32 {
        return Err("recovery tree too deep".into());
    }
    for e in std::fs::read_dir(at)? {
        let e = e?;
        let ty = e.file_type()?;
        if ty.is_symlink() {
            return Err("recovery trees cannot contain symlinks".into());
        }
        if ty.is_dir() {
            collect(root, &e.path(), out, depth + 1)?;
        } else if ty.is_file() {
            if out.len() >= 20000 {
                return Err("recovery file-count limit".into());
            }
            let path = e.path();
            let file = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("non-UTF8 recovery path")?
                .replace('\\', "/");
            if file == COMMIT {
                continue;
            }
            out.push(OwnedFile {
                file,
                bytes: e.metadata()?.len(),
                sha256: hash(&path)?,
            });
        } else {
            return Err("unsupported recovery file type".into());
        }
    }
    Ok(())
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
impl Store {
    pub fn open(directory: &Path, project_id: &str, limits: Limits) -> Result<Self, ProjectError> {
        if !valid_id(project_id)
            || !(1..=32).contains(&limits.snapshots)
            || !(1024 * 1024..=8 * 1024 * 1024 * 1024).contains(&limits.retained_bytes)
        {
            return Err("invalid recovery identity/limits".into());
        }
        match std::fs::create_dir(directory) {
            Ok(()) => {
                let marker = StoreMarker {
                    schema: 1,
                    project_id: project_id.into(),
                };
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(directory.join(MARKER))?;
                file.write_all(&serde_json::to_vec(&marker)?)?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let root = directory.canonicalize()?;
        let marker: StoreMarker = read_json(&root.join(MARKER), 16384)?;
        if marker.schema != 1 || marker.project_id != project_id {
            return Err("recovery store belongs to another project or is not recognized".into());
        }
        Ok(Self {
            root,
            project_id: project_id.into(),
            limits,
        })
    }
    fn lock(&self) -> Result<std::fs::File, ProjectError> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("writer.lock"))?;
        file.try_lock()
            .map_err(|e| format!("recovery writer is busy: {e}"))?;
        Ok(file)
    }
    fn commit(&self, snapshot: &Snapshot) -> Result<Commit, ProjectError> {
        let c: Commit = read_json(&snapshot.directory.join(COMMIT), 8 * 1024 * 1024)?;
        if c.schema != 1
            || c.project_id != self.project_id
            || c.generation != snapshot.generation
            || c.files.is_empty()
            || c.files.len() > 20000
        {
            return Err("invalid recovery commit".into());
        }
        let mut total = 0u64;
        let mut names = BTreeSet::new();
        for f in &c.files {
            total = total.checked_add(f.bytes).ok_or("recovery size overflow")?;
            if total > 8 * 1024 * 1024 * 1024 {
                return Err("recovery size limit exceeded".into());
            }
            if !safe_relative(&f.file)
                || f.file == COMMIT
                || !names.insert(&f.file)
                || f.sha256.len() != 64
                || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("invalid recovery inventory".into());
            }
        }
        Ok(c)
    }
    pub fn inventory(&self) -> Result<Inventory, ProjectError> {
        let mut snapshots = vec![];
        let mut warnings = vec![];
        let mut count = 0;
        for e in std::fs::read_dir(&self.root)? {
            count += 1;
            if count > 4096 {
                return Err("recovery store entry limit exceeded".into());
            }
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(number) = name
                .strip_prefix("snapshot-")
                .filter(|n| n.len() == 20 && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse::<u64>().ok())
            else {
                continue;
            };
            if !e.file_type()?.is_dir() {
                warnings.push(format!("Ignored non-directory {name}"));
                continue;
            }
            let mut snapshot = Snapshot {
                directory: e.path(),
                generation: number,
                bytes: 0,
            };
            match self.commit(&snapshot) {
                Ok(c) => {
                    let bytes = c.files.iter().try_fold(0u64, |sum, f| {
                        sum.checked_add(f.bytes).ok_or("snapshot size overflow")
                    })?;
                    snapshot.bytes =
                        bytes + std::fs::metadata(snapshot.directory.join(COMMIT))?.len();
                    snapshots.push(snapshot)
                }
                Err(error) => warnings.push(format!("Ignored {name}: {error}")),
            }
        }
        snapshots.sort_by_key(|s| std::cmp::Reverse(s.generation));
        Ok(Inventory {
            snapshots,
            warnings,
        })
    }
    fn verify(&self, s: &Snapshot) -> Result<(), ProjectError> {
        let c = self.commit(s)?;
        let mut actual = vec![];
        collect(&s.directory, &s.directory, &mut actual, 0)?;
        actual.sort_by(|a, b| a.file.cmp(&b.file));
        let mut expected = c.files;
        expected.sort_by(|a, b| a.file.cmp(&b.file));
        if actual.len() != expected.len()
            || actual
                .iter()
                .zip(&expected)
                .any(|(a, b)| a.file != b.file || a.bytes != b.bytes || a.sha256 != b.sha256)
        {
            return Err("recovery file inventory changed".into());
        }
        Ok(())
    }
    pub fn restore_latest(&self, destination: &Path) -> Result<Restored, ProjectError> {
        let parent = destination
            .parent()
            .ok_or("recovery destination needs a parent")?
            .canonicalize()?;
        if parent.starts_with(&self.root) || destination.exists() {
            return Err("restore into a new folder outside the managed recovery store".into());
        }
        let _lock = self.lock()?;
        let mut inventory = self.inventory()?;
        for snapshot in inventory.snapshots {
            let result = (|| -> Result<Project, ProjectError> {
                self.verify(&snapshot)?;
                let p = Project::open(&snapshot.directory)?;
                if p.project_id != self.project_id {
                    return Err("snapshot identity mismatch".into());
                }
                Ok(p)
            })();
            match result {
                Ok(project) => {
                    project.save_new(destination)?;
                    let project = Project::open(destination)?;
                    return Ok(Restored {
                        project,
                        snapshot,
                        warnings: inventory.warnings,
                    });
                }
                Err(e) => inventory
                    .warnings
                    .push(format!("Rejected generation {}: {e}", snapshot.generation)),
            }
        }
        Err(format!(
            "no valid recovery snapshot: {}",
            inventory.warnings.join("; ")
        )
        .into())
    }
    pub fn checkpoint(&self, project: &Project) -> Result<Saved, ProjectError> {
        project.validate()?;
        if project.project_id != self.project_id {
            return Err("project does not match recovery store".into());
        }
        for path in project
            .disk
            .values()
            .chain(&project.takes)
            .chain(&project.midi_takes)
            .chain(project.freezes.values().map(|f| &f.directory))
        {
            if path.canonicalize()?.starts_with(&self.root) {
                return Err(
                    "restore into an independent working folder before checkpointing".into(),
                );
            }
        }
        let _lock = self.lock()?;
        let mut old = self.inventory()?;
        if old.snapshots.len() > self.limits.snapshots
            || old.snapshots.iter().map(|s| s.bytes).sum::<u64>() > self.limits.retained_bytes
        {
            return Err(
                "recovery retention is blocked; inspect existing snapshots before adding another"
                    .into(),
            );
        }
        let mut generation = 0u64;
        for e in std::fs::read_dir(&self.root)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if let Some(n) = name
                .strip_prefix("snapshot-")
                .filter(|n| n.len() == 20)
                .and_then(|n| n.parse::<u64>().ok())
            {
                generation = generation.max(n);
            }
        }
        generation = generation
            .checked_add(1)
            .ok_or("recovery generation exhausted")?;
        let directory = self.root.join(format!("snapshot-{generation:020}"));
        project.save_new(&directory)?;
        let result = (|| -> Result<Snapshot, ProjectError> {
            let mut files = vec![];
            collect(&directory, &directory, &mut files, 0)?;
            let commit = Commit {
                schema: 1,
                project_id: self.project_id.clone(),
                generation,
                files,
            };
            let bytes = serde_json::to_vec_pretty(&commit)?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("recovery inventory too large".into());
            }
            let size = commit.files.iter().try_fold(bytes.len() as u64, |sum, f| {
                sum.checked_add(f.bytes).ok_or("recovery size overflow")
            })?;
            if size > self.limits.retained_bytes {
                return Err("snapshot exceeds recovery retained-byte limit".into());
            }
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(directory.join(COMMIT))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(Snapshot {
                directory: directory.clone(),
                generation,
                bytes: size,
            })
        })();
        let snapshot = match result {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(e);
            }
        };
        let mut total = old.snapshots.iter().map(|s| s.bytes).sum::<u64>() + snapshot.bytes;
        while old.snapshots.len() + 1 > self.limits.snapshots || total > self.limits.retained_bytes
        {
            let Some(stale) = old.snapshots.pop() else {
                break;
            };
            match self.verify(&stale).and_then(|()| {
                std::fs::remove_dir_all(&stale.directory)?;
                Ok(())
            }) {
                Ok(()) => total -= stale.bytes,
                Err(e) => {
                    old.warnings.push(format!(
                        "Snapshot saved, but retention could not remove {}: {e}",
                        stale.generation
                    ));
                    break;
                }
            }
        }
        Ok(Saved {
            snapshot,
            warnings: old.warnings,
        })
    }
}
/// One worker, no queued backlog. Time is application-provided; capture the applied project before tick.
pub struct Autosave {
    store: Store,
    interval: Duration,
    next: Duration,
    worker: Option<std::thread::JoinHandle<Result<Saved, ProjectError>>>,
}
impl Autosave {
    pub fn new(store: Store, interval: Duration) -> Result<Self, ProjectError> {
        if !(Duration::from_secs(1)..=Duration::from_secs(3600)).contains(&interval) {
            return Err("autosave interval must be 1..3600 seconds".into());
        }
        Ok(Self {
            store,
            interval,
            next: interval,
            worker: None,
        })
    }
    pub fn tick(&mut self, elapsed: Duration, project: &Project) -> Result<bool, ProjectError> {
        if elapsed < self.next || self.worker.is_some() {
            return Ok(false);
        }
        if project.project_id != self.store.project_id {
            return Err("autosave project identity changed; select another store".into());
        }
        let store = self.store.clone();
        let project = project.clone();
        let worker = std::thread::Builder::new()
            .name("soniclaw-autosave".into())
            .spawn(move || store.checkpoint(&project))?;
        self.worker = Some(worker);
        self.next = elapsed.saturating_add(self.interval);
        Ok(true)
    }
    pub fn poll(&mut self) -> Option<Result<Saved, ProjectError>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        Some(
            self.worker
                .take()
                .unwrap()
                .join()
                .unwrap_or_else(|_| Err("autosave worker panicked".into())),
        )
    }
    pub fn checkpoint_now(
        &mut self,
        elapsed: Duration,
        project: &Project,
    ) -> Result<bool, ProjectError> {
        if self.worker.is_some() {
            return Ok(false);
        }
        self.next = elapsed;
        self.tick(elapsed, project)
    }
    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }
}
impl Drop for Autosave {
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        let (d, a) = crate::tests::fixture();
        Project::new("Original".into(), d, a, Default::default()).unwrap()
    }
    #[test]
    fn retained_snapshots_skip_incomplete_corrupt_latest_and_preserve_working_copy() {
        let root = std::env::temp_dir().join(format!("soniclaw-recovery61-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let mut p = project();
        let store = Store::open(
            &root.join("recovery"),
            &p.project_id,
            Limits {
                snapshots: 2,
                retained_bytes: 1024 * 1024,
            },
        )
        .unwrap();
        let first = store.checkpoint(&p).unwrap();
        p.name = "Second".into();
        let second = store.checkpoint(&p).unwrap();
        std::fs::create_dir(root.join("recovery/snapshot-00000000000000000003")).unwrap();
        p.name = "Third".into();
        let third = store.checkpoint(&p).unwrap();
        assert_eq!(third.snapshot.generation, 4);
        assert!(!first.snapshot.directory.exists());
        assert!(second.snapshot.directory.exists());
        std::fs::write(
            third.snapshot.directory.join("assets/0.wav"),
            b"interrupted or corrupt resource",
        )
        .unwrap();
        let restored = store.restore_latest(&root.join("working")).unwrap();
        assert_eq!(restored.project.name, "Second");
        assert!(!restored.warnings.is_empty());
        assert!(store.restore_latest(&root.join("working")).is_err());
        assert!(store
            .restore_latest(&root.join("recovery/new-work"))
            .is_err());
        let before = restored.project.assets["a"].samples.clone();
        let saved = store.checkpoint(&restored.project).unwrap();
        assert!(saved.snapshot.generation > third.snapshot.generation);
        assert_eq!(
            Project::open(&root.join("working")).unwrap().assets["a"]
                .samples
                .as_ref(),
            before.as_ref()
        );
        assert!(Store::open(
            &root.join("recovery"),
            "different-project",
            Limits::default()
        )
        .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn lock_and_background_tick_bound_concurrency() {
        let root = std::env::temp_dir().join(format!("soniclaw-autosave61-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = project();
        let store = Store::open(&root, &p.project_id, Limits::default()).unwrap();
        let guard = store.lock().unwrap();
        assert!(store.checkpoint(&p).is_err());
        drop(guard);
        let mut worker = Autosave::new(store, Duration::from_secs(1)).unwrap();
        assert!(!worker.tick(Duration::ZERO, &p).unwrap());
        assert!(worker.tick(Duration::from_secs(1), &p).unwrap());
        assert!(!worker.tick(Duration::from_secs(2), &p).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(result) = worker.poll() {
                assert_eq!(result.unwrap().snapshot.generation, 1);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!worker.busy());
        drop(worker);
        std::fs::remove_dir_all(root).unwrap();
    }
}
