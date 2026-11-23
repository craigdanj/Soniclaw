//! The audio side only copies into a bounded queue. The worker owns disk I/O.
//! Overflow is counted and represented by silence, preserving recording time.
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
const FRAMES: usize = 256;
pub type RecordError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RecordSource {
    Dry = 1,
    Processed = 2,
}
#[derive(Default)]
pub struct RecordStats {
    pub captured_frames: AtomicU64,
    pub written_frames: AtomicU64,
    pub dropped_frames: AtomicU64,
    pub write_errors: AtomicU64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordSummary {
    pub captured_frames: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub write_errors: u64,
}
#[derive(Default)]
struct Shared {
    mode: AtomicU8,
    generation: AtomicU64,
    inflight: AtomicUsize,
    stats: RecordStats,
}
#[derive(Clone, Copy)]
struct Packet {
    generation: u64,
    start: u64,
    frames: usize,
    data: [f32; FRAMES * 2],
}
pub struct RecordTap {
    producer: rtrb::Producer<Packet>,
    shared: Arc<Shared>,
}
impl RecordTap {
    /// Both buffers must be interleaved stereo with identical lengths. Called by
    /// one audio thread. Invalid blocks are refused without writing partial data.
    pub fn capture_pair(&mut self, dry: &[f32], processed: &[f32]) {
        self.shared.inflight.fetch_add(1, Ordering::SeqCst);
        let mode = self.shared.mode.load(Ordering::SeqCst);
        if mode != 0 {
            let data = if mode == RecordSource::Dry as u8 {
                dry
            } else {
                processed
            };
            if dry.len() != processed.len() || !data.len().is_multiple_of(2) {
                self.shared
                    .stats
                    .write_errors
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                let generation = self.shared.generation.load(Ordering::SeqCst);
                for chunk in data.chunks(FRAMES * 2) {
                    let frames = chunk.len() / 2;
                    let start = self
                        .shared
                        .stats
                        .captured_frames
                        .fetch_add(frames as u64, Ordering::SeqCst);
                    let mut packet = Packet {
                        generation,
                        start,
                        frames,
                        data: [0.0; FRAMES * 2],
                    };
                    packet.data[..chunk.len()].copy_from_slice(chunk);
                    if self.producer.push(packet).is_err() {
                        self.shared
                            .stats
                            .dropped_frames
                            .fetch_add(frames as u64, Ordering::Relaxed);
                    }
                }
            }
        }
        self.shared.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}
enum Command {
    Start(PathBuf, u32, u64, mpsc::Sender<Result<(), String>>),
    Stop(u64, mpsc::Sender<Result<(), String>>),
    Quit,
}
pub struct Recorder {
    shared: Arc<Shared>,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl Recorder {
    pub fn new(queue_packets: usize) -> (Self, RecordTap) {
        let (producer, consumer) = rtrb::RingBuffer::new(queue_packets.clamp(1, 4096));
        let shared = Arc::new(Shared::default());
        let (commands, receiver) = mpsc::channel();
        let state = shared.clone();
        let worker = thread::spawn(move || worker(consumer, receiver, state));
        (
            Self {
                shared: shared.clone(),
                commands,
                worker: Some(worker),
            },
            RecordTap { producer, shared },
        )
    }
    pub fn stats(&self) -> &RecordStats {
        &self.shared.stats
    }
    pub fn summary(&self) -> RecordSummary {
        let s = self.stats();
        RecordSummary {
            captured_frames: s.captured_frames.load(Ordering::SeqCst),
            written_frames: s.written_frames.load(Ordering::Relaxed),
            dropped_frames: s.dropped_frames.load(Ordering::Relaxed),
            write_errors: s.write_errors.load(Ordering::Relaxed),
        }
    }
    pub fn is_recording(&self) -> bool {
        self.shared.mode.load(Ordering::SeqCst) != 0
    }
    /// Control-thread API. The worker opens the destination before recording arms.
    /// Uses create_new: never overwrites an existing recording.
    pub fn start(
        &mut self,
        path: impl AsRef<Path>,
        sample_rate: u32,
        source: RecordSource,
    ) -> Result<(), RecordError> {
        if self.is_recording() || !(1..=768000).contains(&sample_rate) {
            return Err("already recording or invalid sample rate".into());
        }
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = mpsc::channel();
        self.commands.send(Command::Start(
            path.as_ref().to_owned(),
            sample_rate,
            generation,
            tx,
        ))?;
        rx.recv()?.map_err(RecordError::from)?;
        self.shared.stats.captured_frames.store(0, Ordering::SeqCst);
        self.shared.stats.written_frames.store(0, Ordering::Relaxed);
        self.shared.stats.dropped_frames.store(0, Ordering::Relaxed);
        self.shared.stats.write_errors.store(0, Ordering::Relaxed);
        self.shared.mode.store(source as u8, Ordering::SeqCst);
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), RecordError> {
        self.shared.mode.store(0, Ordering::SeqCst);
        // Wait only on the control thread. New capture calls see mode=0.
        while self.shared.inflight.load(Ordering::SeqCst) != 0 {
            thread::yield_now();
        }
        let total = self.shared.stats.captured_frames.load(Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.commands.send(Command::Stop(total, tx))?;
        rx.recv()?.map_err(Into::into)
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = self.commands.send(Command::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Writer {
    wav: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    generation: u64,
    frames: u64,
}
fn silence(writer: &mut Writer, target: u64) -> Result<(), hound::Error> {
    while writer.frames < target {
        writer.wav.write_sample(0i16)?;
        writer.wav.write_sample(0i16)?;
        writer.frames += 1;
    }
    Ok(())
}
fn drain(
    queue: &mut rtrb::Consumer<Packet>,
    writer: &mut Option<Writer>,
    shared: &Shared,
) -> Result<(), hound::Error> {
    while let Ok(packet) = queue.pop() {
        if let Some(w) = writer
            .as_mut()
            .filter(|w| w.generation == packet.generation)
        {
            silence(w, packet.start)?;
            for &sample in &packet.data[..packet.frames * 2] {
                let value = if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                w.wav.write_sample((value * 32767.0).round() as i16)?;
            }
            w.frames += packet.frames as u64;
            shared
                .stats
                .written_frames
                .store(w.frames, Ordering::Relaxed);
        }
    }
    Ok(())
}
fn worker(
    mut queue: rtrb::Consumer<Packet>,
    commands: mpsc::Receiver<Command>,
    shared: Arc<Shared>,
) {
    let mut writer = None;
    let mut failure: Option<String> = None;
    loop {
        if let Err(error) = drain(&mut queue, &mut writer, &shared) {
            failure = Some(error.to_string());
            shared.stats.write_errors.fetch_add(1, Ordering::Relaxed);
            shared.mode.store(0, Ordering::SeqCst);
            writer = None;
        }
        match commands.recv_timeout(Duration::from_millis(2)) {
            Ok(Command::Start(path, rate, generation, reply)) => {
                failure = None;
                let result = (|| -> Result<(), RecordError> {
                    if writer.is_some() {
                        return Err("previous recording must be stopped".into());
                    }
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)?;
                    let spec = hound::WavSpec {
                        channels: 2,
                        sample_rate: rate,
                        bits_per_sample: 16,
                        sample_format: hound::SampleFormat::Int,
                    };
                    writer = Some(Writer {
                        wav: hound::WavWriter::new(std::io::BufWriter::new(file), spec)?,
                        generation,
                        frames: 0,
                    });
                    Ok(())
                })()
                .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            Ok(Command::Stop(total, reply)) => {
                let result = (|| -> Result<(), RecordError> {
                    drain(&mut queue, &mut writer, &shared)?;
                    if let Some(mut w) = writer.take() {
                        silence(&mut w, total)?;
                        shared
                            .stats
                            .written_frames
                            .store(w.frames, Ordering::Relaxed);
                        w.wav.finalize()?;
                    }
                    if let Some(error) = failure.take() {
                        return Err(error.into());
                    }
                    Ok(())
                })()
                .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    static ID: AtomicU64 = AtomicU64::new(0);
    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "soniclaw-record-{}-{}.wav",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn dry_and_wet_recordings_and_restarts() {
        let (mut recorder, mut tap) = Recorder::new(4);
        for (source, expected) in [(RecordSource::Dry, 0.25), (RecordSource::Processed, 0.5)] {
            let p = path();
            recorder.start(&p, 48000, source).unwrap();
            tap.capture_pair(&[0.25; 20], &[0.5; 20]);
            recorder.stop().unwrap();
            let mut r = hound::WavReader::open(&p).unwrap();
            assert_eq!(r.duration(), 10);
            assert!(
                (r.samples::<i16>().next().unwrap().unwrap() as f32 / 32767.0 - expected).abs()
                    < 0.0001
            );
            assert!(recorder.start(&p, 48000, source).is_err());
            std::fs::remove_file(p).unwrap();
        }
    }
    #[test]
    fn overflow_preserves_duration_and_reports_loss() {
        let (mut recorder, mut tap) = Recorder::new(1);
        let p = path();
        recorder.start(&p, 48000, RecordSource::Dry).unwrap();
        for _ in 0..10000 {
            tap.capture_pair(&[0.1; 512], &[0.0; 512]);
        }
        recorder.stop().unwrap();
        assert_eq!(hound::WavReader::open(&p).unwrap().duration(), 2560000);
        assert!(recorder.stats().dropped_frames.load(Ordering::Relaxed) > 0);
        std::fs::remove_file(p).unwrap();
    }
}
#[cfg(test)]
mod concurrent_tests {
    use super::*;
    #[test]
    fn stop_finalizes_all_inflight_packets_and_the_next_take_is_clean() {
        let (mut recorder, mut tap) = Recorder::new(16);
        let p = std::env::temp_dir().join(format!("soniclaw-inflight-{}.wav", std::process::id()));
        let _ = std::fs::remove_file(&p);
        recorder.start(&p, 48000, RecordSource::Dry).unwrap();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let running = finished.clone();
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            tap.capture_pair(&[0.25; 512], &[0.5; 512]);
            tx.send(()).unwrap();
            while !running.load(Ordering::Acquire) {
                tap.capture_pair(&[0.25; 512], &[0.5; 512]);
                thread::yield_now();
            }
            tap
        });
        rx.recv().unwrap();
        recorder.stop().unwrap();
        finished.store(true, Ordering::Release);
        let mut tap = thread.join().unwrap();
        let count = recorder.summary().captured_frames;
        assert_eq!(hound::WavReader::open(&p).unwrap().duration() as u64, count);
        std::fs::remove_file(&p).unwrap();
        recorder.start(&p, 44100, RecordSource::Processed).unwrap();
        tap.capture_pair(&[0.25; 20], &[0.5; 20]);
        recorder.stop().unwrap();
        let mut reader = hound::WavReader::open(&p).unwrap();
        assert_eq!(reader.duration(), 10);
        assert_eq!(reader.spec().sample_rate, 44100);
        assert!(
            (reader.samples::<i16>().next().unwrap().unwrap() as f32 / 32767.0 - 0.5).abs() < 1e-4
        );
        std::fs::remove_file(p).unwrap();
    }
}

pub mod comp;
pub mod multitrack;

pub mod salvage;
