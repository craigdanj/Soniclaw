//! Control-side reversible pre-mixer stem rendering, validated cache reuse and bounce.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub type FreezeError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frozen {
    pub schema: u32,
    pub track: String,
    pub original: Document,
    pub removed_parameters: Vec<String>,
    pub fingerprint: String,
    pub wav_sha256: String,
    pub directory: PathBuf,
    pub start_frame: u64,
    pub frames: u64,
}
fn file_hash(path: &Path, hash: &mut Sha256) -> Result<(), FreezeError> {
    let mut file = std::fs::File::open(path)?;
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(())
}
fn wav_hash(path: &Path) -> Result<String, FreezeError> {
    let mut h = Sha256::new();
    file_hash(path, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}
fn fingerprint(
    d: &Document,
    id: &str,
    assets: &Assets,
    disk: &DiskAssets,
) -> Result<String, FreezeError> {
    let track = d
        .tracks
        .iter()
        .find(|t| t.id == id)
        .ok_or("track missing")?;
    // Mixer edits remain live. Conservatively invalidate for any non-mixer automation/state edit.
    let lanes: Vec<_> = d
        .automation
        .lanes
        .iter()
        .filter(|l| !l.parameter.starts_with("mix."))
        .collect();
    let values: BTreeMap<_, _> = d
        .parameter_values
        .iter()
        .filter(|(id, _)| !id.starts_with("mix."))
        .collect();
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(&(
        d.sample_rate,
        d.max_block_frames,
        &d.timeline,
        track,
        lanes,
        values,
    ))?);
    for (id, a) in assets {
        h.update((id.len() as u64).to_le_bytes());
        h.update(id);
        h.update(a.sample_rate.to_le_bytes());
        h.update(a.channels.to_le_bytes());
        h.update((a.samples.len() as u64).to_le_bytes());
        for s in a.samples.iter() {
            h.update(s.to_bits().to_le_bytes());
        }
    }
    for (id, path) in disk {
        h.update((id.len() as u64).to_le_bytes());
        h.update(id);
        file_hash(path, &mut h)?;
    }
    Ok(format!("{:x}", h.finalize()))
}
/// Render a new immutable cache. `length` includes the musical selection; `tail` extends it.
/// All preparation, disk access and rendering happen on the calling control/offline thread.
#[allow(clippy::too_many_arguments)]
pub fn render(
    d: &Document,
    id: &str,
    assets: &Assets,
    disk: &DiskAssets,
    factory: &mut impl Factory,
    directory: &Path,
    start: u64,
    length: u64,
    tail: u64,
) -> Result<Frozen, FreezeError> {
    render_monitored(
        d,
        id,
        assets,
        disk,
        factory,
        directory,
        start,
        length,
        tail,
        &Monitor::default(),
    )
}
/// Same render with cancellation and atomic progress. All work remains off callback.
#[allow(clippy::too_many_arguments)]
pub fn render_monitored(
    d: &Document,
    id: &str,
    assets: &Assets,
    disk: &DiskAssets,
    factory: &mut impl Factory,
    directory: &Path,
    start: u64,
    length: u64,
    tail: u64,
    monitor: &Monitor,
) -> Result<Frozen, FreezeError> {
    monitor.check()?;
    d.validate()?;
    let frames = length.checked_add(tail).ok_or("range overflow")?;
    let end = start.checked_add(frames).ok_or("range overflow")?;
    if length == 0 || frames > u32::MAX as u64 / 2 || end > d.sample_rate as u64 * 60 * 60 * 4 {
        return Err("freeze range exceeds four-hour render/RIFF bound".into());
    }
    let index = d
        .tracks
        .iter()
        .position(|t| t.id == id)
        .ok_or("track missing")?;
    if d.tracks[index].input_gain != 0. || d.tracks[index].midi_input {
        return Err("disable live input before freezing an offline track".into());
    }
    if d.mix_routing
        .sidechains
        .iter()
        .any(|s| s.source == id || d.tracks[index].inserts.iter().any(|p| p.id == s.target))
    {
        return Err("disconnect sidechains before freezing a source or detector target".into());
    }
    let key = fingerprint(d, id, assets, disk)?;
    let target_ids: Vec<_> = d.tracks[index]
        .instrument
        .iter()
        .chain(&d.tracks[index].inserts)
        .map(|p| p.id.clone())
        .collect();
    let mut removed = Vec::new();
    let mut wrapped = |desc: &Descriptor| {
        let built = factory.create(desc)?;
        if target_ids.contains(&desc.id) {
            removed.extend(built.parameters.iter().map(|p| p.info().id.to_string()));
        }
        Ok(built)
    };
    let mut offline = d.compile_offline(assets, disk, &mut wrapped)?;
    let prepared = &mut offline.inner;
    prepared.capture.enabled.store(true, Ordering::Release);
    let latency = prepared.capture.track_latency as u64;
    std::fs::create_dir(directory)?;
    let result = (|| -> Result<Frozen, FreezeError> {
        let mut writer = hound::WavWriter::create(
            directory.join("stem.wav"),
            hound::WavSpec {
                channels: 2,
                sample_rate: d.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        let mut mix = vec![0.; d.max_block_frames * 2];
        let mut stem = mix.clone();
        let mut clock = 0;
        monitor.inner.total.store(end + latency, Ordering::Release);
        while clock < end + latency {
            monitor.check()?;
            let count = ((end + latency - clock) as usize).min(d.max_block_frames);
            mix[..count * 2].fill(0.);
            prepared.process(
                &mut AudioBuffer::new(&mut mix[..count * 2], 2)?,
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: clock,
                        playing: true,
                        ..Default::default()
                    },
                },
            );
            if prepared.state.failures.load(Ordering::Acquire) != 0 {
                return Err("offline disk/processor render failed".into());
            }
            prepared.capture.read(index, &mut stem[..count * 2]);
            for f in 0..count {
                let pos = clock + f as u64;
                if pos >= start + latency && pos < end + latency {
                    for s in &stem[f * 2..f * 2 + 2] {
                        writer.write_sample(*s)?;
                    }
                }
            }
            clock += count as u64;
            monitor.inner.completed.store(clock, Ordering::Release);
        }
        writer.finalize()?;
        if prepared.state.failures.load(Ordering::Acquire) != 0 {
            return Err("render failed".into());
        }
        monitor.check()?;
        if fingerprint(d, id, assets, disk)? != key {
            return Err("sources changed during freeze".into());
        }
        monitor.check()?;
        let frozen = Frozen {
            schema: 1,
            track: id.into(),
            original: d.clone(),
            removed_parameters: removed,
            fingerprint: key,
            wav_sha256: wav_hash(&directory.join("stem.wav"))?,
            directory: directory.into(),
            start_frame: start,
            frames,
        };
        std::fs::write(
            directory.join("freeze.json"),
            serde_json::to_vec_pretty(&frozen)?,
        )?;
        Ok(frozen)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(directory);
    }
    result
}
impl Frozen {
    pub fn read(directory: &Path) -> Result<Self, FreezeError> {
        let bytes = std::fs::read(directory.join("freeze.json"))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("freeze manifest too large".into());
        }
        let mut s: Self = serde_json::from_slice(&bytes)?;
        s.directory = directory.into();
        s.verify()?;
        Ok(s)
    }
    pub fn verify(&self) -> Result<(), FreezeError> {
        self.original.validate()?;
        if self.schema != 1
            || self.frames == 0
            || wav_hash(&self.directory.join("stem.wav"))? != self.wav_sha256
        {
            return Err("invalid or modified frozen cache".into());
        }
        let r = hound::WavReader::open(self.directory.join("stem.wav"))?;
        if r.spec().channels != 2
            || r.spec().sample_rate != self.original.sample_rate
            || r.duration() as u64 != self.frames
        {
            return Err("freeze header mismatch".into());
        }
        Ok(())
    }
    /// Return a playback document/catalog, retaining the original project in this manifest.
    pub fn apply(
        &self,
        d: &Document,
        assets: &Assets,
        disk: &DiskAssets,
    ) -> Result<(Document, DiskAssets), FreezeError> {
        self.verify()?;
        if fingerprint(d, &self.track, assets, disk)? != self.fingerprint {
            return Err("stale freeze: source configuration or assets changed".into());
        }
        let mut result = d.clone();
        let t = result
            .tracks
            .iter_mut()
            .find(|t| t.id == self.track)
            .ok_or("track missing")?;
        let asset = format!("freeze.{}", self.track);
        if disk.contains_key(&asset) {
            return Err("freeze asset identity collision".into());
        }
        *t = Track {
            id: self.track.clone(),
            audio: vec![],
            midi: vec![],
            instrument: None,
            inserts: vec![],
            input_gain: 0.,
            midi_input: false,
            streamed: vec![soniclaw_stream::clips::DiskClip {
                edits: None,
                id: asset.clone(),
                asset: asset.clone(),
                start_frame: self.start_frame,
                length_frames: self.frames,
                source_offset: 0,
                source_length: self.frames,
                looping: false,
                gain: 1.,
                fade_in: 0,
                fade_out: 0,
                enabled: true,
            }],
        };
        result
            .automation
            .lanes
            .retain(|l| !self.removed_parameters.contains(&l.parameter));
        result
            .parameter_values
            .retain(|p, _| !self.removed_parameters.contains(p));
        let mut catalog = disk.clone();
        catalog.insert(asset, self.directory.join("stem.wav"));
        result.validate()?;
        Ok((result, catalog))
    }
    /// Restore processors, clips and their parameter lanes while preserving live mixer edits.
    pub fn unfreeze(&self, d: &Document) -> Result<Document, FreezeError> {
        let mut result = d.clone();
        let target = result
            .tracks
            .iter_mut()
            .find(|t| t.id == self.track)
            .ok_or("track missing")?;
        if target.streamed.len() != 1
            || target.streamed[0].asset != format!("freeze.{}", self.track)
        {
            return Err("track no longer refers to this freeze".into());
        }
        *target = self
            .original
            .tracks
            .iter()
            .find(|t| t.id == self.track)
            .ok_or("original track missing")?
            .clone();
        result
            .automation
            .lanes
            .retain(|l| !self.removed_parameters.contains(&l.parameter));
        result.automation.lanes.extend(
            self.original
                .automation
                .lanes
                .iter()
                .filter(|l| self.removed_parameters.contains(&l.parameter))
                .cloned(),
        );
        for id in &self.removed_parameters {
            result.parameter_values.remove(id);
            if let Some(v) = self.original.parameter_values.get(id) {
                result.parameter_values.insert(id.clone(), *v);
            }
        }
        result.validate()?;
        Ok(result)
    }
    /// Destructive processor replacement in the returned document only; original manifest remains.
    pub fn bounce(
        &self,
        d: &Document,
        assets: &Assets,
        disk: &DiskAssets,
    ) -> Result<(Document, DiskAssets), FreezeError> {
        self.apply(d, assets, disk)
    }
}
#[derive(Default)]
struct ProgressState {
    cancelled: AtomicBool,
    completed: AtomicU64,
    total: AtomicU64,
}
#[derive(Clone, Default)]
pub struct Monitor {
    inner: Arc<ProgressState>,
    external_cancel: Option<Arc<AtomicBool>>,
}
impl Monitor {
    /// Share progress while observing a scheduler cancellation token on the worker.
    pub fn with_cancel(&self, token: Arc<AtomicBool>) -> Self {
        Self {
            inner: self.inner.clone(),
            external_cancel: Some(token),
        }
    }
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }
    pub fn progress(&self) -> (u64, u64) {
        (
            self.inner.completed.load(Ordering::Acquire),
            self.inner.total.load(Ordering::Acquire),
        )
    }
    fn check(&self) -> Result<(), FreezeError> {
        if self.inner.cancelled.load(Ordering::Acquire)
            || self
                .external_cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Acquire))
        {
            Err("freeze cancelled".into())
        } else {
            Ok(())
        }
    }
}
/// One bounded background task; factories must be explicitly worker-owned and Send.
/// Polling joins only an already-finished worker. Dropping cancels and joins on the caller.
pub struct Job {
    pub monitor: Monitor,
    worker: Option<std::thread::JoinHandle<Result<Frozen, FreezeError>>>,
}
impl Job {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn<F: Factory + Send + 'static>(
        d: Document,
        track: String,
        assets: Assets,
        disk: DiskAssets,
        mut factory: F,
        directory: PathBuf,
        start: u64,
        length: u64,
        tail: u64,
    ) -> Result<Self, FreezeError> {
        d.validate()?;
        if directory.exists() {
            return Err("freeze destination already exists".into());
        }
        let monitor = Monitor::default();
        let progress = monitor.clone();
        let worker = std::thread::Builder::new()
            .name("soniclaw-freeze".into())
            .spawn(move || {
                render_monitored(
                    &d,
                    &track,
                    &assets,
                    &disk,
                    &mut factory,
                    &directory,
                    start,
                    length,
                    tail,
                    &progress,
                )
            })?;
        Ok(Self {
            monitor,
            worker: Some(worker),
        })
    }
    pub fn poll(&mut self) -> Option<Result<Frozen, FreezeError>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        Some(match self.worker.take().unwrap().join() {
            Ok(result) => result,
            Err(_) => Err("freeze worker panicked".into()),
        })
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.monitor.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_offline_job_waveform_cancel_and_stale_result() {
        let root =
            std::env::temp_dir().join(format!("soniclaw-freeze-streamed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.wav");
        let mut w = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..48000 {
            w.write_sample((i % 997) as f32 / 997. * 0.2).unwrap();
        }
        w.finalize().unwrap();
        let (mut d, a) = crate::tests::fixture();
        d.tracks[0].audio.clear();
        d.tracks[0].streamed = vec![DiskClip {
            edits: None,
            id: "disk".into(),
            asset: "source".into(),
            start_frame: 0,
            length_frames: 48000,
            source_offset: 0,
            source_length: 48000,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
        }];
        let disk = DiskAssets::from([("source".into(), source)]);
        let mut job = Job::spawn(
            d.clone(),
            "audio".into(),
            a.clone(),
            disk.clone(),
            Builtins,
            root.join("frozen"),
            0,
            48000,
            0,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let f = loop {
            if let Some(r) = job.poll() {
                break r.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        assert_eq!(job.monitor.progress(), (48000, 48000));
        let pcm = hound::WavReader::open(f.directory.join("stem.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for (i, frame) in pcm.as_chunks::<2>().0.iter().enumerate() {
            assert!(
                (frame[0] - (i % 997) as f32 / 997. * 0.1).abs() < 1e-7,
                "frame {i}: got {}",
                frame[0]
            );
            assert_eq!(frame[0], frame[1]);
        }
        let mut changed = d.clone();
        changed.tracks[0].streamed[0].gain = 0.5;
        assert!(f.apply(&changed, &a, &disk).is_err());
        let mut p =
            crate::project::Project::new("Frozen disk".into(), d.clone(), a.clone(), disk.clone())
                .unwrap();
        p.apply_frozen(f).unwrap();
        p.save_new(&root.join("portable")).unwrap();
        let mut loaded = crate::project::Project::open(&root.join("portable")).unwrap();
        loaded.unfreeze("audio").unwrap();
        assert_eq!(loaded.document.tracks[0].streamed[0].asset, "source");
        let monitor = Monitor::default();
        monitor.cancel();
        assert!(render_monitored(
            &d,
            "audio",
            &a,
            &disk,
            &mut Builtins,
            &root.join("cancelled"),
            0,
            48000,
            0,
            &monitor
        )
        .is_err());
        assert!(!root.join("cancelled").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_reuse_bypass_restore_and_invalidation() {
        let (d, a) = crate::tests::fixture();
        let path = std::env::temp_dir().join(format!("soniclaw-freeze-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let frozen = render(
            &d,
            "audio",
            &a,
            &DiskAssets::new(),
            &mut Builtins,
            &path,
            0,
            512,
            64,
        )
        .unwrap();
        let loaded = Frozen::read(&path).unwrap();
        let (mut playback, catalog) = loaded.apply(&d, &a, &DiskAssets::new()).unwrap();
        assert!(playback.tracks[0].inserts.is_empty());
        let mut calls = 0;
        let mut reject = |_: &Descriptor| {
            calls += 1;
            Err(Error::InvalidParameter)
        };
        let _p = playback
            .compile_streamed(&a, &catalog, &mut reject)
            .unwrap();
        assert_eq!(calls, 0);
        playback.mixer.strips[0].gain = 0.3;
        let restored = loaded.unfreeze(&playback).unwrap();
        assert_eq!(restored.mixer.strips[0].gain, 0.3);
        assert_eq!(restored.tracks[0].inserts.len(), 1);
        let pcm = hound::WavReader::open(path.join("stem.wav"))
            .unwrap()
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!((pcm[100] - 0.1).abs() < 1e-6);
        let mut changed = d.clone();
        changed.tracks[0].inserts[0].state = serde_json::json!({"value":0.8});
        assert!(frozen.apply(&changed, &a, &DiskAssets::new()).is_err());
        std::fs::write(path.join("stem.wav"), b"tampered").unwrap();
        assert!(loaded.verify().is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
