//! Transactional waveform/warp drafts; preparation belongs on a worker, application on the stopped control thread.
use crate::{
    project::{Project, ProjectError},
    tempo_cache::{self, Binding, Marker, Mode, Recipe},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
};
#[derive(Clone, Debug)]
pub struct Draft {
    pub source: String,
    pub track: String,
    pub region: String,
    pub asset: String,
    pub recipe: Recipe,
}
impl Draft {
    pub fn new(
        p: &Project,
        source: &str,
        track: &str,
        region: &str,
        asset: &str,
    ) -> Result<Self, ProjectError> {
        let r = hound::WavReader::open(
            p.disk
                .get(source)
                .ok_or("select an original disk WAV source")?,
        )?;
        let frames = r.duration() as u64;
        if frames < 2 {
            return Err("warp source is too short".into());
        }
        let map = crate::TempoMap::compile(&p.document.timeline)?;
        let beat = map.beat_at_sample(
            (frames as f64 * p.document.sample_rate as f64 / r.spec().sample_rate as f64).round()
                as u64,
            p.document.sample_rate as f64,
        )?;
        let draft = Self {
            source: source.into(),
            track: track.into(),
            region: region.into(),
            asset: asset.into(),
            recipe: Recipe {
                schema: 1,
                mode: Mode::PreservePitch,
                markers: vec![
                    Marker {
                        source_frame: 0,
                        beat: 0.,
                    },
                    Marker {
                        source_frame: frames,
                        beat,
                    },
                ],
            },
        };
        draft.validate(p)?;
        Ok(draft)
    }
    pub fn validate(&self, p: &Project) -> Result<(), ProjectError> {
        if !crate::valid_id(&self.asset)
            || !crate::valid_id(&self.region)
            || self.asset == self.source
            || !p.document.tracks.iter().any(|t| t.id == self.track)
            || p.warps.contains_key(&self.source)
        {
            return Err("invalid draft identity or generated cache selected as original".into());
        }
        let r = hound::WavReader::open(p.disk.get(&self.source).ok_or("missing source")?)?;
        self.recipe.validate(
            r.duration() as u64,
            &p.document.timeline,
            p.document.sample_rate,
        )
    }
    pub fn insert(
        &mut self,
        source_frame: u64,
        beat: f64,
        p: &Project,
    ) -> Result<(), ProjectError> {
        let mut next = self.clone();
        next.recipe.markers.push(Marker { source_frame, beat });
        next.recipe.markers.sort_by_key(|m| m.source_frame);
        next.validate(p)?;
        *self = next;
        Ok(())
    }
    pub fn build(
        &self,
        p: &Project,
        directory: &Path,
        cancel: &AtomicBool,
    ) -> Result<Ready, ProjectError> {
        self.validate(p)?;
        let binding = tempo_cache::prepare(
            &p.disk[&self.source],
            &self.source,
            &self.recipe,
            &p.document.timeline,
            p.document.sample_rate,
            directory,
            cancel,
        )?;
        Ok(Ready {
            project_id: p.project_id.clone(),
            source_path: p.disk[&self.source].clone(),
            draft: self.clone(),
            binding,
            wav: directory.join("audio.wav"),
        })
    }
}
pub struct Ready {
    project_id: String,
    source_path: PathBuf,
    pub draft: Draft,
    pub binding: Binding,
    pub wav: PathBuf,
}
impl Ready {
    pub fn apply(self, p: &mut Project) -> Result<(), ProjectError> {
        if p.project_id != self.project_id
            || p.disk.get(&self.draft.source) != Some(&self.source_path)
        {
            return Err("stale waveform job; project/source changed".into());
        }
        let mut next = p.clone();
        next.apply_warp(
            &self.draft.track,
            &self.draft.region,
            &self.draft.asset,
            self.binding,
            &self.wav,
        )?;
        next.validate()?;
        *p = next;
        Ok(())
    }
}
pub struct Job {
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<Ready, ProjectError>>>,
}
impl Job {
    pub fn start(draft: Draft, project: Project, directory: PathBuf) -> Result<Self, ProjectError> {
        draft.validate(&project)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("soniclaw-warp-editor".into())
            .spawn(move || draft.build(&project, &directory, &c))?;
        Ok(Self {
            cancel,
            worker: Some(worker),
        })
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn poll(&mut self) -> Option<Result<Ready, ProjectError>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        Some(
            self.worker
                .take()?
                .join()
                .unwrap_or_else(|_| Err("warp worker panicked".into())),
        )
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
