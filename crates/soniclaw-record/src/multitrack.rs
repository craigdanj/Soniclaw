//! Synchronized float32 multitrack takes. Callback copies only; one worker owns history and all files.
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
const FRAMES: usize = 128;
const CHANNELS: usize = 64;
pub type RecordError = Box<dyn std::error::Error + Send + Sync>;
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackSpec {
    pub id: String,
    pub channels: usize,
    pub latency_frames: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub sample_rate: u32,
    pub tracks: Vec<TrackSpec>,
    pub pre_roll_frames: usize,
    pub queue_packets: usize,
    pub max_block_frames: usize,
}
impl Config {
    pub fn validate(&self) -> Result<usize, Error> {
        if !(8000..=192000).contains(&self.sample_rate)
            || self.tracks.is_empty()
            || self.tracks.len() > 32
            || self.pre_roll_frames > self.sample_rate as usize * 10
            || !(4..=1024).contains(&self.queue_packets)
            || !(1..=1024).contains(&self.max_block_frames)
        {
            return Err(Error::InvalidSpec);
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut channels = 0;
        for t in &self.tracks {
            if !valid_id(&t.id)
                || !ids.insert(&t.id)
                || !(1..=8).contains(&t.channels)
                || t.latency_frames > 262144
            {
                return Err(Error::InvalidParameter);
            }
            channels += t.channels;
        }
        if channels > CHANNELS
            || (self.pre_roll_frames + self.max_block_frames * 2)
                .checked_mul(channels)
                .is_none_or(|v| v > 16 * 1024 * 1024)
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(channels)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakePlan {
    pub id: String,
    pub label: String,
    pub punch: Option<[u64; 2]>,
}
impl TakePlan {
    fn validate(&self) -> Result<(), Error> {
        if !valid_id(&self.id)
            || self.label.len() > 256
            || self.punch.is_some_and(|[a, b]| a >= b || b > 1 << 48)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedTrack {
    pub id: String,
    pub file: String,
    pub channels: usize,
    pub latency_frames: usize,
    pub frames: u64,
    pub peak: f32,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoundaryKind {
    Loop,
    Transport,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopBoundary {
    pub sample: u64,
    pub source_frame: u64,
    pub kind: BoundaryKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loop_boundaries: Vec<LoopBoundary>,
    pub schema: u32,
    pub id: String,
    pub label: String,
    pub sample_rate: u32,
    pub acquisition_start: u64,
    pub file_start: u64,
    pub frames: u64,
    pub pre_roll_frames: usize,
    pub punch: Option<[u64; 2]>,
    pub tracks: Vec<RecordedTrack>,
    pub missing_frames: u64,
    pub queue_drops: u64,
    #[serde(default)]
    pub input_discontinuities: u64,
    pub complete: bool,
}
impl Take {
    pub fn read(directory: &Path) -> Result<Self, RecordError> {
        let mut bytes = Vec::new();
        File::open(directory.join("take.json"))?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err("take manifest too large".into());
        }
        let take: Self = serde_json::from_slice(&bytes)?;
        if take.schema != 1
            || !take.complete
            || !valid_id(&take.id)
            || !(8000..=192000).contains(&take.sample_rate)
            || take.tracks.is_empty()
            || take.tracks.len() > 32
        {
            return Err("invalid/incomplete take".into());
        }
        if take.loop_boundaries.len() > 4096
            || take
                .loop_boundaries
                .windows(2)
                .any(|w| w[0].sample >= w[1].sample)
            || take.loop_boundaries.iter().any(|b| {
                b.sample < take.file_start
                    || b.sample >= take.file_start.saturating_add(take.frames)
                    || b.source_frame > 1 << 48
            })
        {
            return Err("invalid loop annotations".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for t in &take.tracks {
            if !valid_id(&t.id)
                || !ids.insert(&t.id)
                || t.file != format!("{}.wav", t.id)
                || !(1..=8).contains(&t.channels)
                || t.frames != take.frames
                || !t.peak.is_finite()
                || t.error.is_some()
            {
                return Err("invalid take track".into());
            }
            let w = hound::WavReader::open(directory.join(&t.file))?;
            let s = w.spec();
            if s.sample_format != hound::SampleFormat::Float
                || s.bits_per_sample != 32
                || s.channels as usize != t.channels
                || s.sample_rate != take.sample_rate
                || w.duration() as u64 != t.frames
            {
                return Err("take file does not match manifest".into());
            }
        }
        Ok(take)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeSelection {
    pub schema: u32,
    pub directories: Vec<PathBuf>,
    pub selected: String,
}
impl TakeSelection {
    pub fn resolve(&self) -> Result<(PathBuf, Take), RecordError> {
        if self.schema != 1
            || self.directories.is_empty()
            || self.directories.len() > 256
            || !valid_id(&self.selected)
        {
            return Err("invalid take selection".into());
        }
        let mut result = None;
        for dir in &self.directories {
            let take = Take::read(dir)?;
            if take.id == self.selected {
                if result.is_some() {
                    return Err("duplicate selected take identity".into());
                }
                result = Some((dir.clone(), take));
            }
        }
        result.ok_or_else(|| "selected take is missing".into())
    }
    pub fn to_json(&self) -> Result<String, RecordError> {
        self.resolve()?;
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(text: &str) -> Result<Self, RecordError> {
        if text.len() > 65536 {
            return Err("selection too large".into());
        }
        let s: Self = serde_json::from_str(text)?;
        s.resolve()?;
        Ok(s)
    }
}
#[derive(Clone)]
struct Packet {
    sample: u64,
    frames: usize,
    data: [f32; FRAMES * CHANNELS],
    loop_sources: [u64; FRAMES],
    loop_kinds: [u8; FRAMES],
}
#[derive(Default)]
struct Shared {
    latest: AtomicU64,
    drops: AtomicU64,
    invalid: AtomicU64,
    io: AtomicU64,
    stop: AtomicBool,
    armed: Vec<AtomicBool>,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Stats {
    pub latest_sample: u64,
    pub dropped_frames: u64,
    pub invalid_blocks: u64,
    pub io_errors: u64,
}
impl Shared {
    fn stats(&self) -> Stats {
        Stats {
            latest_sample: self.latest.load(Ordering::Acquire),
            dropped_frames: self.drops.load(Ordering::Relaxed),
            invalid_blocks: self.invalid.load(Ordering::Relaxed),
            io_errors: self.io.load(Ordering::Relaxed),
        }
    }
}
pub struct Tap {
    queue: rtrb::Producer<Packet>,
    shared: Arc<Shared>,
    config: Arc<Config>,
    channels: usize,
    last: Option<u64>,
}
impl Tap {
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn capture(&mut self, sample: u64, inputs: &[&[f32]]) -> Result<(), Error> {
        self.capture_with_loops(sample, inputs, &[])
    }
    /// Annotations and PCM publish in the same bounded packet; no UI-clock estimation.
    pub fn capture_with_loops(
        &mut self,
        sample: u64,
        inputs: &[&[f32]],
        boundaries: &[LoopBoundary],
    ) -> Result<(), Error> {
        let result = (|| {
            if self.shared.stop.load(Ordering::Acquire) {
                return Err(Error::NotPrepared);
            }
            if inputs.len() != self.config.tracks.len() {
                return Err(Error::InvalidBuffer);
            }
            let frames = inputs[0].len() / self.config.tracks[0].channels;
            if frames > self.config.max_block_frames
                || sample
                    .checked_add(frames as u64)
                    .is_none_or(|v| v > 1 << 48)
                || self.last.is_some_and(|last| {
                    sample < last || sample - last > self.config.sample_rate as u64 * 10
                })
            {
                return Err(Error::InvalidEvents);
            }
            if boundaries.len() > frames
                || boundaries.windows(2).any(|w| w[0].sample >= w[1].sample)
                || boundaries.iter().any(|b| {
                    b.sample < sample
                        || b.sample >= sample + frames as u64
                        || b.source_frame > 1 << 48
                })
            {
                return Err(Error::InvalidEvents);
            }
            for (t, input) in self.config.tracks.iter().zip(inputs) {
                if input.len() != frames * t.channels
                    || input.iter().any(|v| !v.is_finite() || v.abs() > 1e6)
                {
                    return Err(Error::InvalidBuffer);
                }
            }
            let mut offset = 0;
            while offset < frames {
                let count = (frames - offset).min(FRAMES);
                let mut packet = Packet {
                    sample: sample + offset as u64,
                    frames: count,
                    data: [0.; FRAMES * CHANNELS],
                    loop_sources: [u64::MAX; FRAMES],
                    loop_kinds: [0; FRAMES],
                };
                for boundary in boundaries {
                    if boundary.sample >= packet.sample
                        && boundary.sample < packet.sample + count as u64
                    {
                        let i = (boundary.sample - packet.sample) as usize;
                        packet.loop_sources[i] = boundary.source_frame;
                        packet.loop_kinds[i] = if boundary.kind == BoundaryKind::Loop {
                            1
                        } else {
                            2
                        };
                    }
                }
                let mut channel = 0;
                for (track_index, (t, input)) in self.config.tracks.iter().zip(inputs).enumerate() {
                    for frame in 0..count {
                        if self.shared.armed[track_index].load(Ordering::Relaxed) {
                            packet.data[frame * self.channels + channel
                                ..frame * self.channels + channel + t.channels]
                                .copy_from_slice(
                                    &input[(offset + frame) * t.channels
                                        ..(offset + frame + 1) * t.channels],
                                );
                        }
                    }
                    channel += t.channels;
                }
                if self.queue.push(packet).is_err() {
                    self.shared.drops.fetch_add(count as u64, Ordering::Relaxed);
                }
                offset += count;
            }
            self.last = Some(sample + frames as u64);
            self.shared
                .latest
                .store(sample + frames as u64, Ordering::Release);
            Ok(())
        })();
        if result.is_err() {
            self.shared.invalid.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
    pub fn stats(&self) -> Stats {
        self.shared.stats()
    }
}
enum Command {
    Start(PathBuf, TakePlan, u64, mpsc::Sender<Result<(), String>>),
    Stop(u64, mpsc::Sender<Result<Take, String>>),
    Quit,
}
pub struct Recorder {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    active: bool,
    takes: Vec<Take>,
    config: Arc<Config>,
}
pub fn pair(config: Config) -> Result<(Recorder, Tap), RecordError> {
    let channels = config.validate()?;
    let (queue, receiver) = rtrb::RingBuffer::new(config.queue_packets);
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        armed: (0..config.tracks.len())
            .map(|_| AtomicBool::new(true))
            .collect(),
        ..Default::default()
    });
    let config = Arc::new(config);
    let worker_shared = shared.clone();
    let worker_config = config.clone();
    let worker = thread::Builder::new()
        .name("soniclaw-takes".into())
        .spawn(move || worker(receiver, rx, worker_shared, worker_config, channels))?;
    Ok((
        Recorder {
            commands: tx,
            shared: shared.clone(),
            worker: Some(worker),
            active: false,
            takes: vec![],
            config: config.clone(),
        },
        Tap {
            queue,
            shared,
            config,
            channels,
            last: None,
        },
    ))
}
impl Recorder {
    pub fn set_armed(&self, index: usize, armed: bool) -> Result<(), Error> {
        let flag = self
            .shared
            .armed
            .get(index)
            .ok_or(Error::InvalidParameter)?;
        flag.store(armed, Ordering::Relaxed);
        Ok(())
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
    pub fn stats(&self) -> Stats {
        self.shared.stats()
    }
    pub fn takes(&self) -> &[Take] {
        &self.takes
    }
    pub fn start(
        &mut self,
        directory: impl AsRef<Path>,
        plan: TakePlan,
    ) -> Result<(), RecordError> {
        plan.validate()?;
        if self.active {
            return Err("take already active".into());
        }
        let (tx, rx) = mpsc::channel();
        let boundary = self.shared.latest.load(Ordering::Acquire);
        if plan.punch.is_some_and(|p| {
            p[0] < boundary && boundary - p[0] > self.config.sample_rate as u64 * 10
        }) {
            return Err("punch begins outside supported history window".into());
        }
        self.commands.send(Command::Start(
            directory.as_ref().into(),
            plan,
            boundary,
            tx,
        ))?;
        rx.recv()?.map_err(RecordError::from)?;
        self.active = true;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<Take, RecordError> {
        if !self.active {
            return Err("no active take".into());
        }
        let (tx, rx) = mpsc::channel();
        self.commands.send(Command::Stop(
            self.shared.latest.load(Ordering::Acquire),
            tx,
        ))?;
        let result = rx.recv()?.map_err(RecordError::from);
        self.active = false;
        if let Ok(t) = &result {
            self.takes.push(t.clone());
        }
        result
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        if self.active {
            let _ = self.stop();
        }
        let _ = self.commands.send(Command::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
struct History {
    pcm: Vec<f32>,
    tags: Vec<u64>,
    loop_sources: Vec<u64>,
    loop_kinds: Vec<u8>,
    channels: usize,
    end: u64,
}
impl History {
    fn new(frames: usize, channels: usize) -> Self {
        Self {
            pcm: vec![0.; frames * channels],
            tags: vec![u64::MAX; frames],
            loop_sources: vec![u64::MAX; frames],
            loop_kinds: vec![0; frames],
            channels,
            end: 0,
        }
    }
    fn consume(&mut self, p: &Packet) {
        for i in 0..p.frames {
            let sample = p.sample + i as u64;
            let slot = (sample % self.tags.len() as u64) as usize;
            self.tags[slot] = sample;
            self.loop_sources[slot] = p.loop_sources[i];
            self.loop_kinds[slot] = p.loop_kinds[i];
            self.pcm[slot * self.channels..(slot + 1) * self.channels]
                .copy_from_slice(&p.data[i * self.channels..(i + 1) * self.channels]);
        }
        self.end = self.end.max(p.sample + p.frames as u64);
    }
    fn boundary(&self, sample: u64) -> Option<LoopBoundary> {
        let slot = (sample % self.tags.len() as u64) as usize;
        (self.tags[slot] == sample && self.loop_sources[slot] != u64::MAX).then(|| LoopBoundary {
            sample,
            source_frame: self.loop_sources[slot],
            kind: if self.loop_kinds[slot] == 1 {
                BoundaryKind::Loop
            } else {
                BoundaryKind::Transport
            },
        })
    }
    fn frame(&self, sample: u64) -> Option<&[f32]> {
        let slot = (sample % self.tags.len() as u64) as usize;
        (self.tags[slot] == sample)
            .then(|| &self.pcm[slot * self.channels..(slot + 1) * self.channels])
    }
}
struct Wave {
    file: BufWriter<File>,
    channels: usize,
    frames: u64,
    peak: f32,
    error: Option<String>,
}
impl Wave {
    fn new(path: &Path, rate: u32, channels: usize) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)?;
        let mut file = BufWriter::new(f);
        let mut h = [0u8; 44];
        h[..4].copy_from_slice(b"RIFF");
        h[8..16].copy_from_slice(b"WAVEfmt ");
        h[16..20].copy_from_slice(&16u32.to_le_bytes());
        h[20..22].copy_from_slice(&3u16.to_le_bytes());
        h[22..24].copy_from_slice(&(channels as u16).to_le_bytes());
        h[24..28].copy_from_slice(&rate.to_le_bytes());
        h[28..32].copy_from_slice(&(rate * channels as u32 * 4).to_le_bytes());
        h[32..34].copy_from_slice(&(channels as u16 * 4).to_le_bytes());
        h[34..36].copy_from_slice(&32u16.to_le_bytes());
        h[36..40].copy_from_slice(b"data");
        file.write_all(&h)?;
        Ok(Self {
            file,
            channels,
            frames: 0,
            peak: 0.,
            error: None,
        })
    }
    fn frame(&mut self, input: &[f32]) {
        if self.error.is_some() {
            return;
        }
        if (self.frames + 1)
            .checked_mul(self.channels as u64 * 4)
            .is_none_or(|b| b > u32::MAX as u64 - 36)
        {
            self.error = Some("RIFF size limit".into());
            return;
        }
        for v in input {
            self.peak = self.peak.max(v.abs());
            if let Err(e) = self.file.write_all(&v.to_le_bytes()) {
                self.error = Some(e.to_string());
                return;
            }
        }
        self.frames += 1;
    }
    fn finish(&mut self, frames: u64) {
        if self.error.is_some() {
            let _ = self.file.flush();
            return;
        }
        while self.frames < frames {
            self.frame(&[0.; 8][..self.channels]);
            if self.error.is_some() {
                return;
            }
        }
        let bytes = frames * self.channels as u64 * 4;
        let result = (|| {
            self.file.flush()?;
            self.file.get_mut().set_len(44 + bytes)?;
            self.file.seek(SeekFrom::Start(4))?;
            self.file.write_all(&((bytes + 36) as u32).to_le_bytes())?;
            self.file.seek(SeekFrom::Start(40))?;
            self.file.write_all(&(bytes as u32).to_le_bytes())?;
            self.file.flush()
        })();
        if let Err(e) = result {
            let e: std::io::Error = e;
            self.error = Some(e.to_string());
        }
        self.frames = frames;
    }
}
struct Active {
    directory: PathBuf,
    plan: TakePlan,
    boundary: u64,
    origin: u64,
    end: u64,
    waves: Vec<Wave>,
    missing: Vec<[u64; 2]>,
    boundaries: Vec<LoopBoundary>,
    drops: u64,
}
impl Active {
    fn new(
        directory: PathBuf,
        plan: TakePlan,
        boundary: u64,
        c: &Config,
        history: &History,
        drops: u64,
    ) -> Result<Self, RecordError> {
        let origin = plan
            .punch
            .map_or(boundary, |p| p[0])
            .saturating_sub(c.pre_roll_frames as u64);
        std::fs::create_dir(&directory)?;
        let waves = c
            .tracks
            .iter()
            .map(|t| {
                Wave::new(
                    &directory.join(format!("{}.wav", t.id)),
                    c.sample_rate,
                    t.channels,
                )
            })
            .collect::<Result<Vec<_>, _>>();
        let waves = match waves {
            Ok(w) => w,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(e.into());
            }
        };
        let mut a = Self {
            directory,
            plan,
            boundary,
            origin,
            end: origin,
            waves,
            missing: vec![],
            boundaries: Vec::new(),
            drops,
        };
        let until = history
            .end
            .max(boundary)
            .min(a.plan.punch.map_or(u64::MAX, |p| p[1]));
        if until > origin {
            while a.end < until {
                let sample = a.end;
                let data = history.frame(sample);
                a.frame(data, history.boundary(sample), c);
            }
        }
        Ok(a)
    }
    fn gap(&mut self, start: u64, end: u64) {
        if let Some(last) = self.missing.last_mut() {
            if last[1] == start {
                last[1] = end;
                return;
            }
        }
        if self.missing.len() < 4096 {
            self.missing.push([start, end]);
        } else {
            for w in &mut self.waves {
                w.error = Some("recording gap budget exhausted".into());
            }
        }
    }
    fn frame(&mut self, input: Option<&[f32]>, boundary: Option<LoopBoundary>, c: &Config) {
        if let Some(boundary) = boundary {
            if self.boundaries.len() < 4096 {
                self.boundaries.push(boundary);
            } else {
                for w in &mut self.waves {
                    w.error = Some("loop annotation budget exhausted".into());
                }
            }
        }
        let zero = [0.; CHANNELS];
        let data = input.unwrap_or(&zero);
        if input.is_none() {
            self.gap(self.end, self.end + 1);
        }
        let mut offset = 0;
        for (w, t) in self.waves.iter_mut().zip(&c.tracks) {
            w.frame(&data[offset..offset + t.channels]);
            offset += t.channels;
        }
        self.end += 1;
    }
    fn consume(&mut self, p: &Packet, c: &Config, channels: usize) {
        let limit = self.plan.punch.map_or(u64::MAX, |p| p[1]);
        let end = (p.sample + p.frames as u64).min(limit);
        if end <= self.end || end <= self.origin {
            return;
        }
        let start = p.sample.max(self.end).max(self.origin);
        if start - self.end > c.sample_rate as u64 * 10 {
            for w in &mut self.waves {
                w.error = Some("acquisition gap exceeds ten seconds".into());
            }
            self.end = end;
            return;
        }
        while self.end < start {
            self.frame(None, None, c);
        }
        for sample in start..end {
            let i = (sample - p.sample) as usize;
            let boundary = (p.loop_sources[i] != u64::MAX).then(|| LoopBoundary {
                sample,
                source_frame: p.loop_sources[i],
                kind: if p.loop_kinds[i] == 1 {
                    BoundaryKind::Loop
                } else {
                    BoundaryKind::Transport
                },
            });
            self.frame(Some(&p.data[i * channels..(i + 1) * channels]), boundary, c);
        }
    }
    fn finish(mut self, boundary: u64, c: &Config, drops: u64) -> Result<Take, RecordError> {
        let end = boundary
            .min(self.plan.punch.map_or(u64::MAX, |p| p[1]))
            .max(self.origin);
        let frames = end - self.origin;
        if self.end < end {
            self.gap(self.end, end);
        }
        for w in &mut self.waves {
            w.finish(frames);
        }
        let tracks = self
            .waves
            .iter()
            .zip(&c.tracks)
            .map(|(w, t)| RecordedTrack {
                id: t.id.clone(),
                file: format!("{}.wav", t.id),
                channels: t.channels,
                latency_frames: t.latency_frames,
                frames: w.frames,
                peak: w.peak,
                error: w.error.clone(),
            })
            .collect::<Vec<_>>();
        let take = Take {
            input_discontinuities: 0,
            loop_boundaries: self
                .boundaries
                .into_iter()
                .filter(|b| b.sample >= self.origin && b.sample < end)
                .collect(),
            schema: 1,
            id: self.plan.id,
            label: self.plan.label,
            sample_rate: c.sample_rate,
            acquisition_start: self.boundary,
            file_start: self.origin,
            frames,
            pre_roll_frames: c.pre_roll_frames,
            punch: self.plan.punch,
            complete: tracks.iter().all(|t| t.error.is_none()),
            tracks,
            missing_frames: self
                .missing
                .iter()
                .map(|[a, b]| b.min(&end).saturating_sub((*a).max(self.origin)))
                .sum(),
            queue_drops: drops.saturating_sub(self.drops),
        };
        std::fs::write(
            self.directory.join("take.json"),
            serde_json::to_vec_pretty(&take)?,
        )?;
        Ok(take)
    }
}
fn consume(
    packet: Packet,
    history: &mut History,
    active: &mut Option<Active>,
    c: &Config,
    channels: usize,
) {
    if let Some(a) = active {
        a.consume(&packet, c, channels);
    }
    history.consume(&packet);
}
fn drain(
    boundary: u64,
    queue: &mut rtrb::Consumer<Packet>,
    pending: &mut Option<Packet>,
    history: &mut History,
    active: &mut Option<Active>,
    c: &Config,
    channels: usize,
) {
    loop {
        let packet = if let Some(p) = pending.take() {
            p
        } else {
            match queue.pop() {
                Ok(p) => p,
                Err(_) => break,
            }
        };
        if packet.sample >= boundary {
            *pending = Some(packet);
            break;
        }
        consume(packet, history, active, c, channels);
    }
}
fn worker(
    mut queue: rtrb::Consumer<Packet>,
    commands: mpsc::Receiver<Command>,
    shared: Arc<Shared>,
    c: Arc<Config>,
    channels: usize,
) {
    let mut history = History::new(
        (c.pre_roll_frames + c.max_block_frames * 2).max(1),
        channels,
    );
    let mut pending = None;
    let mut active: Option<Active> = None;
    loop {
        match commands.try_recv() {
            Ok(Command::Start(dir, plan, boundary, reply)) => {
                drain(
                    boundary,
                    &mut queue,
                    &mut pending,
                    &mut history,
                    &mut active,
                    &c,
                    channels,
                );
                let result = if active.is_some() {
                    Err("take already active".to_owned())
                } else {
                    Active::new(
                        dir,
                        plan,
                        boundary,
                        &c,
                        &history,
                        shared.drops.load(Ordering::Relaxed),
                    )
                    .map(|a| active = Some(a))
                    .map_err(|e| e.to_string())
                };
                let _ = reply.send(result);
            }
            Ok(Command::Stop(boundary, reply)) => {
                drain(
                    boundary,
                    &mut queue,
                    &mut pending,
                    &mut history,
                    &mut active,
                    &c,
                    channels,
                );
                let result = active
                    .take()
                    .ok_or_else(|| "no active take".to_owned())
                    .and_then(|a| {
                        a.finish(boundary, &c, shared.drops.load(Ordering::Relaxed))
                            .map_err(|e| e.to_string())
                    });
                if result.as_ref().is_ok_and(|t| !t.complete) || result.is_err() {
                    shared.io.fetch_add(1, Ordering::Relaxed);
                }
                let _ = reply.send(result);
            }
            Ok(Command::Quit) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if let Some(packet) = pending.take().or_else(|| queue.pop().ok()) {
            consume(packet, &mut history, &mut active, &c, channels);
        } else {
            thread::sleep(Duration::from_millis(1));
        }
    }
    shared.stop.store(true, Ordering::Release);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "multi-take-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    fn config() -> Config {
        Config {
            sample_rate: 48000,
            tracks: vec![
                TrackSpec {
                    id: "mono".into(),
                    channels: 1,
                    latency_frames: 0,
                },
                TrackSpec {
                    id: "stereo".into(),
                    channels: 2,
                    latency_frames: 3,
                },
            ],
            pre_roll_frames: 16,
            queue_packets: 16,
            max_block_frames: 64,
        }
    }
    #[test]
    fn synchronized_float_preroll_punch_arming_and_selection() {
        let root = root();
        let (mut r, mut tap) = pair(config()).unwrap();
        for i in 0..4 {
            tap.capture(i * 64, &[&[1.5; 64], &[-0.25; 128]]).unwrap();
        }
        r.start(
            root.join("one"),
            TakePlan {
                id: "one".into(),
                label: "First".into(),
                punch: None,
            },
        )
        .unwrap();
        let alloc = allocation_counter::measure(|| {
            tap.capture(256, &[&[1.5; 64], &[-0.25; 128]]).unwrap();
        });
        assert_eq!(alloc.count_total, 0);
        r.set_armed(1, false).unwrap();
        tap.capture(320, &[&[1.5; 64], &[-0.25; 128]]).unwrap();
        let take = r.stop().unwrap();
        assert_eq!(take.frames, 144);
        assert_eq!(take.file_start, 240);
        assert_eq!(take.missing_frames, 0);
        assert!(take.complete);
        let mono = hound::WavReader::open(root.join("one/mono.wav"))
            .unwrap()
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(mono, vec![1.5; 144]);
        let stereo = hound::WavReader::open(root.join("one/stereo.wav"))
            .unwrap()
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(stereo[..160].iter().all(|v| *v == -0.25));
        assert!(stereo[160..].iter().all(|v| *v == 0.));
        r.start(
            root.join("two"),
            TakePlan {
                id: "two".into(),
                label: "Punch".into(),
                punch: Some([400, 480]),
            },
        )
        .unwrap();
        for i in 6..9 {
            tap.capture(i * 64, &[&[0.75; 64], &[0.5; 128]]).unwrap();
        }
        let take = r.stop().unwrap();
        assert_eq!(take.file_start, 384);
        assert_eq!(take.frames, 96);
        let select = TakeSelection {
            schema: 1,
            directories: vec![root.join("one"), root.join("two")],
            selected: "two".into(),
        };
        let copy = TakeSelection::from_json(&select.to_json().unwrap()).unwrap();
        assert_eq!(copy.resolve().unwrap().1.id, "two");
        assert!(r
            .start(
                root.join("one"),
                TakePlan {
                    id: "x".into(),
                    label: "".into(),
                    punch: None
                }
            )
            .is_err());
        drop(r);
        drop(tap);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_buffers_do_not_publish_partial_tracks_and_gaps_are_aligned() {
        let root = root();
        let (mut r, mut tap) = pair(config()).unwrap();
        r.start(
            root.join("take"),
            TakePlan {
                id: "test".into(),
                label: "".into(),
                punch: None,
            },
        )
        .unwrap();
        assert!(tap.capture(0, &[&[1.; 64], &[0.; 127]]).is_err());
        assert_eq!(tap.stats().latest_sample, 0);
        tap.capture(0, &[&[1.; 64], &[0.; 128]]).unwrap();
        tap.capture(128, &[&[1.; 64], &[0.; 128]]).unwrap();
        assert!(tap.capture(1, &[&[1.; 64], &[0.; 128]]).is_err());
        let t = r.stop().unwrap();
        assert_eq!(t.frames, 192);
        assert_eq!(t.missing_frames, 64);
        let pcm = hound::WavReader::open(root.join("take/mono.wav"))
            .unwrap()
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&pcm[64..128], &[0.; 64]);
        drop(r);
        drop(tap);
        std::fs::remove_dir_all(root).unwrap();
    }
}
