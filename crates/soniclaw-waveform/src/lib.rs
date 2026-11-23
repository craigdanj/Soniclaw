//! Background/control-thread waveform analysis. Audio PCM is streamed; peak pyramids are bounded and immutable.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
};
const MAX_PAIRS: usize = 8 * 1024 * 1024;
const MAX_CACHE: usize = MAX_PAIRS * 8 + 8192;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}
impl Peak {
    fn empty() -> Self {
        Self {
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
        }
    }
    fn include(&mut self, p: Self) {
        self.min = self.min.min(p.min);
        self.max = self.max.max(p.max);
    }
    fn sample(&mut self, v: f32) {
        self.include(Self { min: v, max: v });
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub enum Backend {
    #[default]
    Wav,
    #[cfg(feature = "formats")]
    Native,
}
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub base_bucket: u32,
    pub backend: Backend,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            base_bucket: 256,
            backend: Backend::Wav,
        }
    }
}
impl Options {
    fn validate(self) -> Result<Self> {
        if !(64..=8192).contains(&self.base_bucket) || !self.base_bucket.is_power_of_two() {
            return Err(Error::Invalid("base bucket must be a power of two64..8192"));
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
pub enum Stage {
    Hashing,
    Decoding,
    Verifying,
    Complete,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Progress {
    pub stage: Stage,
    pub done: u64,
    pub total: u64,
}
#[derive(Debug)]
pub enum Error {
    Cancelled,
    Invalid(&'static str),
    Decode(String),
    Io(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("waveform request cancelled"),
            Self::Invalid(s) => f.write_str(s),
            Self::Decode(s) => f.write_str(s),
            Self::Io(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
struct Level {
    span: u64,
    peaks: Vec<Peak>,
}
pub struct Thumbnail {
    rate: u32,
    channels: usize,
    frames: u64,
    source_sha: [u8; 32],
    levels: Vec<Level>,
}
impl Thumbnail {
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
    pub fn channels(&self) -> usize {
        self.channels
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
    pub fn base_bucket(&self) -> u64 {
        self.levels[0].span
    }
    pub fn source_digest(&self) -> [u8; 32] {
        self.source_sha
    }
    pub fn source_sha256(&self) -> String {
        hex(&self.source_sha)
    }
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
    pub fn memory_bytes(&self) -> usize {
        self.levels.iter().map(|l| l.peaks.len() * 8).sum()
    }
    /// Conservative peak envelopes: includes complete cache buckets overlapping each output pixel.
    pub fn query(&self, channel: usize, start: u64, end: u64, output: &mut [Peak]) -> Result<()> {
        output.fill(Peak::default());
        if channel >= self.channels || start > end || end > self.frames || output.len() > 8192 {
            return Err(Error::Invalid("invalid thumbnail channel/range/width"));
        }
        if start == end || output.is_empty() {
            return Ok(());
        }
        let per_pixel = (end - start) / output.len() as u64;
        let level = self
            .levels
            .iter()
            .rev()
            .find(|l| l.span <= per_pixel)
            .unwrap_or(&self.levels[0]);
        let count = level.peaks.len() / self.channels;
        let width = output.len() as u64;
        for (i, p) in output.iter_mut().enumerate() {
            let a = start + (end - start) * i as u64 / width;
            let b = start + ((end - start) * (i as u64 + 1) / width).max(1);
            let from = (a / level.span) as usize;
            let to = b.div_ceil(level.span).min(count as u64) as usize;
            let mut peak = Peak::empty();
            for bucket in from..to {
                peak.include(level.peaks[bucket * self.channels + channel]);
            }
            if peak.min.is_finite() {
                *p = peak;
            }
        }
        Ok(())
    }
}
pub struct Loaded {
    pub thumbnail: Arc<Thumbnail>,
    pub cache_hit: bool,
    pub rebuilt_corrupt_cache: bool,
}
trait Source {
    fn info(&self) -> (u32, usize, u64);
    fn read(&mut self, out: &mut [f32]) -> Result<usize>;
    fn seek(&mut self, frame: u64) -> Result<()>;
}
struct Wav {
    reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    rate: u32,
    channels: usize,
    frames: u64,
    float: bool,
    scale: f32,
}
impl Source for Wav {
    fn info(&self) -> (u32, usize, u64) {
        (self.rate, self.channels, self.frames)
    }
    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        out.fill(0.);
        let mut count = 0;
        if self.float {
            for sample in self.reader.samples::<f32>().take(out.len()) {
                let value = sample.map_err(|e| Error::Decode(e.to_string()))?;
                if !value.is_finite() {
                    return Err(Error::Invalid("nonfinite source audio"));
                }
                out[count] = value;
                count += 1;
            }
        } else {
            for sample in self.reader.samples::<i32>().take(out.len()) {
                out[count] = sample.map_err(|e| Error::Decode(e.to_string()))? as f32 * self.scale;
                count += 1;
            }
        }
        if count % self.channels != 0 {
            return Err(Error::Invalid("partial source frame"));
        }
        Ok(count / self.channels)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        let frame =
            u32::try_from(frame).map_err(|_| Error::Invalid("WAV seek exceeds RIFF range"))?;
        self.reader
            .seek(frame)
            .map_err(|e| Error::Decode(e.to_string()))
    }
}
#[cfg(feature = "formats")]
impl Source for soniclaw_formats::Reader {
    fn info(&self) -> (u32, usize, u64) {
        let i = self.info();
        (i.sample_rate, i.channels as usize, i.frames)
    }
    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        self.read_frames(out)
            .map_err(|e| Error::Decode(e.to_string()))
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.seek(frame)
            .map(|_| ())
            .map_err(|e| Error::Decode(e.to_string()))
    }
}
fn source(path: &Path, backend: Backend) -> Result<Box<dyn Source>> {
    match backend {
        Backend::Wav => {
            let reader = hound::WavReader::open(path).map_err(|e| Error::Decode(e.to_string()))?;
            let spec = reader.spec();
            if spec.channels == 0
                || spec.channels > 64
                || spec.sample_rate == 0
                || !matches!(
                    (spec.sample_format, spec.bits_per_sample),
                    (hound::SampleFormat::Float, 32) | (hound::SampleFormat::Int, 8 | 16 | 24 | 32)
                )
            {
                return Err(Error::Invalid("unsupported WAV layout/encoding"));
            }
            let frames = reader.duration() as u64;
            Ok(Box::new(Wav {
                reader,
                rate: spec.sample_rate,
                channels: spec.channels as usize,
                frames,
                float: spec.sample_format == hound::SampleFormat::Float,
                scale: 1. / (1u64 << (spec.bits_per_sample - 1)) as f32,
            }))
        }
        #[cfg(feature = "formats")]
        Backend::Native => {
            let manager = soniclaw_formats::FormatManager::native_default()
                .map_err(|e| Error::Decode(e.to_string()))?;
            Ok(Box::new(
                manager
                    .open(path)
                    .map_err(|e| Error::Decode(e.to_string()))?,
            ))
        }
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hash(
    path: &Path,
    cancel: &impl Fn() -> bool,
    progress: &impl Fn(Progress),
    stage: Stage,
) -> Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let total = f.metadata()?.len();
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    let mut done = 0;
    loop {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        done += n as u64;
        progress(Progress { stage, done, total });
    }
    Ok(hash.finalize().into())
}
fn estimate(frames: u64, channels: usize, bucket: u64) -> Result<usize> {
    if channels == 0 || channels > 64 {
        return Err(Error::Invalid("invalid channel count"));
    }
    let mut count = frames.div_ceil(bucket);
    let mut total = 0u64;
    for _ in 0..32 {
        total = total
            .checked_add(
                count
                    .checked_mul(channels as u64)
                    .ok_or(Error::Invalid("peak count overflow"))?,
            )
            .ok_or(Error::Invalid("peak count overflow"))?;
        if total > MAX_PAIRS as u64 {
            return Err(Error::Invalid(
                "thumbnail exceeds64MiB; increase base bucket",
            ));
        }
        if count <= 1 {
            return Ok(total as usize);
        }
        count = count.div_ceil(2);
    }
    Err(Error::Invalid("too many pyramid levels"))
}
fn decode(
    path: &Path,
    options: Options,
    sha: [u8; 32],
    cancel: &impl Fn() -> bool,
    progress: &impl Fn(Progress),
) -> Result<Thumbnail> {
    let mut reader = source(path, options.backend)?;
    let (rate, channels, frames) = reader.info();
    if rate == 0 {
        return Err(Error::Invalid("invalid sample rate"));
    }
    estimate(frames, channels, options.base_bucket as u64)?;
    let capacity = usize::try_from(frames.div_ceil(options.base_bucket as u64))
        .map_err(|_| Error::Invalid("frame count overflow"))?
        * channels;
    let mut peaks = Vec::with_capacity(capacity);
    let mut current = vec![Peak::empty(); channels];
    let mut bucket_frames = 0;
    let mut buffer = vec![0.; 8192 * channels];
    let mut done = 0;
    loop {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        for frame in buffer[..n * channels].chunks_exact(channels) {
            for (c, &v) in frame.iter().enumerate() {
                current[c].sample(v);
            }
            bucket_frames += 1;
            if bucket_frames == options.base_bucket {
                peaks.extend_from_slice(&current);
                current.fill(Peak::empty());
                bucket_frames = 0;
            }
        }
        done += n as u64;
        if done > frames {
            return Err(Error::Invalid("source length changed during decode"));
        }
        progress(Progress {
            stage: Stage::Decoding,
            done,
            total: frames,
        });
    }
    if done != frames {
        return Err(Error::Invalid("source truncated during decode"));
    }
    if bucket_frames > 0 {
        peaks.extend_from_slice(&current);
    }
    let mut levels = vec![Level {
        span: options.base_bucket as u64,
        peaks,
    }];
    while levels.last().unwrap().peaks.len() > channels {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let previous = levels.last().unwrap();
        let buckets = previous.peaks.len() / channels;
        let mut next = Vec::with_capacity(buckets.div_ceil(2) * channels);
        for bucket in (0..buckets).step_by(2) {
            for c in 0..channels {
                let mut p = previous.peaks[bucket * channels + c];
                if bucket + 1 < buckets {
                    p.include(previous.peaks[(bucket + 1) * channels + c]);
                }
                next.push(p);
            }
        }
        levels.push(Level {
            span: previous.span * 2,
            peaks: next,
        });
    }
    Ok(Thumbnail {
        rate,
        channels,
        frames,
        source_sha: sha,
        levels,
    })
}
fn encode(t: &Thumbnail) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(t.memory_bytes() + 512);
    bytes.extend_from_slice(b"SWAVE001");
    bytes.extend_from_slice(&t.rate.to_le_bytes());
    bytes.extend_from_slice(&(t.channels as u32).to_le_bytes());
    bytes.extend_from_slice(&t.frames.to_le_bytes());
    bytes.extend_from_slice(&t.source_sha);
    bytes.extend_from_slice(&(t.levels.len() as u32).to_le_bytes());
    for l in &t.levels {
        bytes.extend_from_slice(&l.span.to_le_bytes());
        bytes.extend_from_slice(&(l.peaks.len() as u64).to_le_bytes());
        for p in &l.peaks {
            bytes.extend_from_slice(&p.min.to_le_bytes());
            bytes.extend_from_slice(&p.max.to_le_bytes());
        }
    }
    let sha = Sha256::digest(&bytes);
    bytes.extend_from_slice(&sha);
    bytes
}
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(Error::Invalid("cache cursor overflow"))?;
        let data = self
            .bytes
            .get(self.position..end)
            .ok_or(Error::Invalid("truncated thumbnail cache"))?;
        self.position = end;
        Ok(data.try_into().unwrap())
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
}
fn read_cache(
    path: &Path,
    sha: [u8; 32],
    bucket: u32,
    cancel: &impl Fn() -> bool,
) -> Result<Thumbnail> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_CACHE + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CACHE || bytes.len() < 96 {
        return Err(Error::Invalid("invalid thumbnail cache length"));
    }
    let payload = bytes.len() - 32;
    if Sha256::digest(&bytes[..payload]).as_slice() != &bytes[payload..] {
        return Err(Error::Invalid("thumbnail cache digest mismatch"));
    }
    let mut cursor = Cursor {
        bytes: &bytes[..payload],
        position: 0,
    };
    if cursor.take::<8>()? != *b"SWAVE001" {
        return Err(Error::Invalid("invalid thumbnail cache schema"));
    }
    let rate = cursor.u32()?;
    let channels = cursor.u32()? as usize;
    let frames = cursor.u64()?;
    let source_sha = cursor.take()?;
    let levels_count = cursor.u32()? as usize;
    let expected = estimate(frames, channels, bucket as u64)?;
    if rate == 0 || source_sha != sha || levels_count == 0 || levels_count > 32 {
        return Err(Error::Invalid("invalid thumbnail cache metadata"));
    }
    let mut required_levels = 1;
    let mut required_buckets = frames.div_ceil(bucket as u64);
    while required_buckets > 1 {
        required_levels += 1;
        required_buckets = required_buckets.div_ceil(2);
    }
    if levels_count != required_levels {
        return Err(Error::Invalid("unexpected pyramid level count"));
    }
    let mut levels = Vec::new();
    let mut buckets = frames.div_ceil(bucket as u64);
    let mut span = bucket as u64;
    let mut pairs = 0;
    for _ in 0..levels_count {
        if cancel() {
            return Err(Error::Cancelled);
        }
        let stored_span = cursor.u64()?;
        let count = cursor.u64()?;
        if stored_span != span
            || count != buckets * channels as u64
            || count > MAX_PAIRS as u64
            || count * 8 > (payload - cursor.position) as u64
        {
            return Err(Error::Invalid("invalid thumbnail level bounds"));
        }
        let mut peaks = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let min = f32::from_le_bytes(cursor.take()?);
            let max = f32::from_le_bytes(cursor.take()?);
            if !min.is_finite() || !max.is_finite() || min > max {
                return Err(Error::Invalid("invalid thumbnail peak"));
            }
            peaks.push(Peak { min, max });
        }
        pairs += peaks.len();
        levels.push(Level { span, peaks });
        buckets = buckets.div_ceil(2);
        span *= 2;
    }
    if cursor.position != payload
        || pairs != expected
        || levels.last().unwrap().peaks.len() > channels
    {
        return Err(Error::Invalid("incomplete thumbnail pyramid"));
    }
    Ok(Thumbnail {
        rate,
        channels,
        frames,
        source_sha,
        levels,
    })
}
fn write_cache(path: &Path, thumbnail: &Thumbnail, cancel: &impl Fn() -> bool) -> Result<()> {
    if cancel() {
        return Err(Error::Cancelled);
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid("clock before epoch"))?
        .as_nanos();
    let temp = path.with_extension(format!("{}.{}.tmp", std::process::id(), nonce));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&encode(thumbnail))?;
        file.sync_all()?;
        drop(file);
        if cancel() {
            return Err(Error::Cancelled);
        }
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
/// Full-byte fingerprints invalidate modified content even if size and mtime are unchanged.
pub fn load_or_build(
    path: &Path,
    cache: &Path,
    options: Options,
    cancel: impl Fn() -> bool,
    progress: impl Fn(Progress),
) -> Result<Loaded> {
    let options = options.validate()?;
    let sha = hash(path, &cancel, &progress, Stage::Hashing)?;
    let backend = match options.backend {
        Backend::Wav => b"wav" as &[u8],
        #[cfg(feature = "formats")]
        Backend::Native => b"native",
    };
    let mut key = Sha256::new();
    key.update(b"soniclaw-peaks-v1");
    key.update(options.base_bucket.to_le_bytes());
    key.update(backend);
    key.update(sha);
    let key = hex(&key.finalize());
    let target = cache.join(key + ".swave");
    let existed = target.exists();
    if existed {
        match read_cache(&target, sha, options.base_bucket, &cancel) {
            Ok(t) => {
                progress(Progress {
                    stage: Stage::Complete,
                    done: t.frames,
                    total: t.frames,
                });
                return Ok(Loaded {
                    thumbnail: Arc::new(t),
                    cache_hit: true,
                    rebuilt_corrupt_cache: false,
                });
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {}
        }
    }
    let thumbnail = decode(path, options, sha, &cancel, &progress)?;
    if hash(path, &cancel, &progress, Stage::Verifying)? != sha {
        return Err(Error::Invalid("source changed during thumbnail build"));
    }
    if cancel() {
        return Err(Error::Cancelled);
    }
    std::fs::create_dir_all(cache)?;
    write_cache(&target, &thumbnail, &cancel)?;
    progress(Progress {
        stage: Stage::Complete,
        done: thumbnail.frames,
        total: thumbnail.frames,
    });
    Ok(Loaded {
        thumbnail: Arc::new(thumbnail),
        cache_hit: false,
        rebuilt_corrupt_cache: existed,
    })
}
/// Bounded PCM detail read for zooms below the base peak-bucket resolution. Control/background thread only.
pub fn read_detail(path: &Path, start: u64, frames: usize, backend: Backend) -> Result<Vec<f32>> {
    let mut reader = source(path, backend)?;
    let (_, channels, total) = reader.info();
    if frames > 65536 || channels * frames > 1024 * 1024 || start > total {
        return Err(Error::Invalid("invalid waveform detail range"));
    }
    let frames = (frames as u64).min(total - start) as usize;
    reader.seek(start)?;
    let mut pcm = vec![0.; frames * channels];
    if reader.read(&mut pcm)? != frames {
        return Err(Error::Invalid("short waveform detail read"));
    }
    Ok(pcm)
}
struct Request {
    ticket: u64,
    path: PathBuf,
    options: Options,
}
pub struct Completion {
    pub ticket: u64,
    pub result: Result<Loaded>,
}
struct Work {
    pending: Option<Request>,
    result: Option<Completion>,
    progress: Option<Progress>,
}
struct Shared {
    work: Mutex<Work>,
    wake: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
}
pub struct Loader {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Loader {
    pub fn new(cache: PathBuf) -> std::io::Result<Self> {
        let cache = std::path::absolute(cache)?;
        let shared = Arc::new(Shared {
            work: Mutex::new(Work {
                pending: None,
                result: None,
                progress: None,
            }),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("soniclaw-waveform".into())
            .spawn(move || loop {
                let request = {
                    let mut work = worker_shared.work.lock().unwrap_or_else(|e| e.into_inner());
                    while work.pending.is_none() && !worker_shared.stop.load(Ordering::Acquire) {
                        work = worker_shared
                            .wake
                            .wait(work)
                            .unwrap_or_else(|e| e.into_inner());
                    }
                    if worker_shared.stop.load(Ordering::Acquire) {
                        return;
                    }
                    work.pending.take().unwrap()
                };
                let ticket = request.ticket;
                let cancelled = || {
                    worker_shared.stop.load(Ordering::Acquire)
                        || worker_shared.generation.load(Ordering::Acquire) != ticket
                };
                let result =
                    load_or_build(&request.path, &cache, request.options, cancelled, |p| {
                        let mut work = worker_shared.work.lock().unwrap_or_else(|e| e.into_inner());
                        if worker_shared.generation.load(Ordering::Acquire) == ticket {
                            work.progress = Some(p);
                        }
                    });
                let mut work = worker_shared.work.lock().unwrap_or_else(|e| e.into_inner());
                if !cancelled() {
                    work.result = Some(Completion { ticket, result });
                }
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    /// Replace a queued job and cancel any older active job; at most one pending request and one result are retained.
    pub fn request(&mut self, path: PathBuf, options: Options) -> Result<u64> {
        let path = std::path::absolute(path)?;
        options.validate()?;
        let mut work = self.shared.work.lock().unwrap_or_else(|e| e.into_inner());
        let ticket = self
            .shared
            .generation
            .load(Ordering::Acquire)
            .checked_add(1)
            .filter(|ticket| *ticket < u64::MAX)
            .ok_or(Error::Invalid("waveform ticket exhausted"))?;
        self.shared.generation.store(ticket, Ordering::Release);
        work.pending = Some(Request {
            ticket,
            path,
            options,
        });
        work.result = None;
        work.progress = None;
        self.shared.wake.notify_one();
        Ok(ticket)
    }
    pub fn cancel(&mut self) {
        let mut work = self.shared.work.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .shared
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_add(1))
            });
        work.pending = None;
        work.result = None;
        work.progress = None;
    }
    pub fn poll(&mut self) -> Option<Completion> {
        self.shared
            .work
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .take()
    }
    pub fn progress(&self) -> Option<Progress> {
        self.shared
            .work
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .progress
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        {
            let _work = self.shared.work.lock().unwrap_or_else(|e| e.into_inner());
            self.shared.stop.store(true, Ordering::Release);
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Pruned {
    pub files_removed: usize,
    pub bytes_removed: u64,
    pub bytes_retained: u64,
}
/// Evict only canonical cache filenames from an application-owned cache directory. Never call on audio.
pub fn prune(cache: &Path, max_bytes: u64) -> Result<Pruned> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in std::fs::read_dir(cache)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(key) = name.strip_suffix(".swave") else {
            continue;
        };
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let metadata = entry.metadata()?;
        if entry.file_type()?.is_symlink() || !metadata.is_file() {
            continue;
        }
        total = total
            .checked_add(metadata.len())
            .ok_or(Error::Invalid("cache size overflow"))?;
        files.push((metadata.modified()?, entry.path(), metadata.len()));
    }
    files.sort_by_key(|f| f.0);
    let mut report = Pruned {
        files_removed: 0,
        bytes_removed: 0,
        bytes_retained: total,
    };
    for (_, path, size) in files {
        if report.bytes_retained <= max_bytes {
            break;
        }
        std::fs::remove_file(path)?;
        report.files_removed += 1;
        report.bytes_removed += size;
        report.bytes_retained -= size;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "soniclaw-waveform-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn wav(path: &Path, channels: u16, samples: &[f32]) {
        let mut w = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }
    #[test]
    fn pyramid_preserves_channels_partial_buckets_and_cache_integrity() {
        let temp = Temp::new();
        let path = temp.0.join("test.wav");
        let cache = temp.0.join("cache");
        let mut audio = vec![0.2; 513 * 2];
        audio[0] = 0.9;
        audio[1024] = -0.7;
        audio[1025] = -0.5;
        wav(&path, 2, &audio);
        let loaded = load_or_build(&path, &cache, Default::default(), || false, |_| {}).unwrap();
        assert!(!loaded.cache_hit);
        assert_eq!(loaded.thumbnail.frames(), 513);
        assert_eq!(loaded.thumbnail.level_count(), 3);
        let mut peak = [Peak::default()];
        loaded.thumbnail.query(0, 0, 513, &mut peak).unwrap();
        assert_eq!(
            peak[0],
            Peak {
                min: -0.7,
                max: 0.9
            }
        );
        loaded.thumbnail.query(1, 0, 513, &mut peak).unwrap();
        assert_eq!(
            peak[0],
            Peak {
                min: -0.5,
                max: 0.2
            }
        );
        assert_eq!(
            read_detail(&path, 512, 1, Backend::Wav).unwrap(),
            vec![-0.7, -0.5]
        );
        assert!(
            load_or_build(&path, &cache, Default::default(), || false, |_| {})
                .unwrap()
                .cache_hit
        );
        let file = std::fs::read_dir(&cache)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] = 0;
        std::fs::write(file, bytes).unwrap();
        let repaired = load_or_build(&path, &cache, Default::default(), || false, |_| {}).unwrap();
        assert!(repaired.rebuilt_corrupt_cache);
    }
    #[test]
    fn full_fingerprint_detects_content_with_same_size_and_timestamp() {
        let temp = Temp::new();
        let path = temp.0.join("test.wav");
        let cache = temp.0.join("cache");
        wav(&path, 1, &[0.1; 256]);
        let time = std::fs::metadata(&path).unwrap().modified().unwrap();
        let old = load_or_build(&path, &cache, Default::default(), || false, |_| {}).unwrap();
        wav(&path, 1, &[0.8; 256]);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(time))
            .unwrap();
        let new = load_or_build(&path, &cache, Default::default(), || false, |_| {}).unwrap();
        assert!(!new.cache_hit);
        assert_ne!(old.thumbnail.source_sha256(), new.thumbnail.source_sha256());
    }
    #[test]
    fn cancellation_latest_request_and_shutdown_are_bounded() {
        let temp = Temp::new();
        let a = temp.0.join("a.wav");
        let b = temp.0.join("b.wav");
        wav(&a, 1, &[0.1; 1000]);
        wav(&b, 1, &[0.7; 512]);
        assert!(matches!(
            load_or_build(
                &a,
                &temp.0.join("cancel"),
                Default::default(),
                || true,
                |_| {}
            ),
            Err(Error::Cancelled)
        ));
        assert!(!temp.0.join("cancel").exists());
        let mut loader = Loader::new(temp.0.join("cache")).unwrap();
        let first = loader.request(a, Default::default()).unwrap();
        let latest = loader.request(b, Default::default()).unwrap();
        assert!(latest > first);
        let start = std::time::Instant::now();
        loop {
            if let Some(done) = loader.poll() {
                assert_eq!(done.ticket, latest);
                assert_eq!(done.result.unwrap().thumbnail.frames(), 512);
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        drop(loader);
        for _ in 0..16 {
            drop(Loader::new(temp.0.join("idle")).unwrap());
        }
    }
    #[test]
    fn hostile_cache_counts_and_empty_audio_do_not_allocate_unboundedly() {
        assert!(estimate(u64::MAX, 64, 64).is_err());
        let temp = Temp::new();
        let path = temp.0.join("empty.wav");
        wav(&path, 1, &[]);
        let loaded = load_or_build(
            &path,
            &temp.0.join("cache"),
            Default::default(),
            || false,
            |_| {},
        )
        .unwrap();
        assert_eq!(loaded.thumbnail.frames(), 0);
        let mut p = [Peak::empty(); 3];
        loaded.thumbnail.query(0, 0, 0, &mut p).unwrap();
        assert_eq!(p, [Peak::default(); 3]);
    }
    #[test]
    fn prune_only_removes_owned_cache_names() {
        let temp = Temp::new();
        let path = temp.0.join("test.wav");
        let cache = temp.0.join("cache");
        wav(&path, 1, &[0.5; 256]);
        load_or_build(&path, &cache, Default::default(), || false, |_| {}).unwrap();
        std::fs::write(cache.join("notes.txt"), "preserve").unwrap();
        let report = prune(&cache, 0).unwrap();
        assert_eq!(report.files_removed, 1);
        assert_eq!(report.bytes_retained, 0);
        assert!(cache.join("notes.txt").exists());
    }
    #[cfg(feature = "formats")]
    #[test]
    #[ignore = "requires installed dynamic libsndfile; run explicitly on a codec-enabled machine"]
    fn native_formats_backend_matches_pcm_wave() {
        let temp = Temp::new();
        let path = temp.0.join("test.wav");
        wav(&path, 2, &[0.25, -0.75, 0.5, -0.5]);
        let loaded = load_or_build(
            &path,
            &temp.0.join("cache"),
            Options {
                backend: Backend::Native,
                ..Default::default()
            },
            || false,
            |_| {},
        )
        .unwrap();
        let mut p = [Peak::default()];
        loaded.thumbnail.query(1, 0, 2, &mut p).unwrap();
        assert_eq!(
            p[0],
            Peak {
                min: -0.75,
                max: -0.5
            }
        );
    }
}
