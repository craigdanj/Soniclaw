//! Forward-only WAV transport. A worker reads PCM/float data into a bounded
//! ring; the audio callback emits silence on misses and preserves elapsed time.
//! This legacy player is same-rate and forward-only. See the separately exported
//! paged DiskSampler and SeekablePlayer APIs for new transport/instrument support.
mod paged;
pub use paged::*;
use soniclaw_core::*;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
struct Frame {
    position: u64,
    samples: [f32; 2],
}
#[derive(Default)]
pub struct StreamStats {
    pub underrun_frames: AtomicU64,
    pub io_errors: AtomicU64,
    pub frames_read: AtomicU64,
    queued: AtomicUsize,
    stop: AtomicBool,
    finished: AtomicBool,
}
pub struct StreamHandle {
    worker: Option<JoinHandle<()>>,
    pub stats: Arc<StreamStats>,
    pub sample_rate: u32,
    pub frames: u64,
}
impl StreamHandle {
    /// Control thread only. Prebuffer before starting a native device.
    pub fn prebuffer(&self, min_frames: usize, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if self.stats.queued.load(Ordering::Acquire) >= min_frames
                || self.stats.finished.load(Ordering::Acquire)
            {
                return true;
            }
            if start.elapsed() >= timeout {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
    pub fn stop(&mut self) {
        self.stats.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
pub struct StreamingPlayer {
    consumer: rtrb::Consumer<Frame>,
    pending: Option<Frame>,
    pub stats: Arc<StreamStats>,
    position: u64,
    total: u64,
    rate: u32,
}
pub fn open_wav(
    path: impl AsRef<Path>,
    capacity_frames: usize,
) -> Result<(StreamHandle, StreamingPlayer), Box<dyn std::error::Error>> {
    if !(64..=1_048_576).contains(&capacity_frames) {
        return Err("stream capacity must be 64..1048576 frames".into());
    }
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if !(1..=2).contains(&spec.channels)
        || spec.sample_rate == 0
        || spec.bits_per_sample > 32
        || spec.bits_per_sample == 0
    {
        return Err("unsupported WAV stream format".into());
    }
    let frames = reader.duration() as u64;
    let stats = Arc::new(StreamStats::default());
    let worker_stats = stats.clone();
    let (mut producer, consumer) = rtrb::RingBuffer::new(capacity_frames);
    let worker = thread::Builder::new()
        .name("soniclaw-wav-reader".into())
        .spawn(move || {
            let mut reader = reader;
            let mut decode: Box<dyn Iterator<Item = Result<f32, hound::Error>>> =
                if spec.sample_format == hound::SampleFormat::Float {
                    Box::new(reader.samples::<f32>())
                } else {
                    let scale = 2.0_f32.powi(spec.bits_per_sample as i32 - 1);
                    Box::new(
                        reader
                            .samples::<i32>()
                            .map(move |v| v.map(|x| x as f32 / scale)),
                    )
                };
            for position in 0..frames {
                if worker_stats.stop.load(Ordering::Acquire) {
                    break;
                }
                let mut samples = [0.0; 2];
                let mut failed = false;
                for sample in samples.iter_mut().take(spec.channels as usize) {
                    match decode.next() {
                        Some(Ok(x)) if x.is_finite() => *sample = x,
                        _ => {
                            worker_stats.io_errors.fetch_add(1, Ordering::Relaxed);
                            failed = true;
                            break;
                        }
                    }
                }
                if failed {
                    break;
                }
                if spec.channels == 1 {
                    samples[1] = samples[0];
                }
                let mut frame = Frame { position, samples };
                loop {
                    if worker_stats.stop.load(Ordering::Acquire) {
                        break;
                    }
                    // Publish queued count before making the element visible; decrementing on
                    // the consumer can never wrap. Count includes this one in-flight frame.
                    worker_stats.queued.fetch_add(1, Ordering::Release);
                    match producer.push(frame) {
                        Ok(()) => break,
                        Err(rtrb::PushError::Full(f)) => {
                            worker_stats.queued.fetch_sub(1, Ordering::Release);
                            frame = f;
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
                worker_stats
                    .frames_read
                    .store(position + 1, Ordering::Relaxed);
            }
            worker_stats.finished.store(true, Ordering::Release);
        })?;
    Ok((
        StreamHandle {
            worker: Some(worker),
            stats: stats.clone(),
            sample_rate: spec.sample_rate,
            frames,
        },
        StreamingPlayer {
            consumer,
            pending: None,
            stats,
            position: 0,
            total: frames,
            rate: spec.sample_rate,
        },
    ))
}
impl StreamingPlayer {
    pub fn position(&self) -> u64 {
        self.position
    }
}
impl AudioProcessor for StreamingPlayer {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if (s.sample_rate - self.rate as f64).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for out in b.samples_mut().chunks_exact_mut(ch) {
            let mut sample = None;
            // Bounded catch-up work even after a lengthy worker stall.
            for _ in 0..64 {
                if self.pending.is_none() {
                    match self.consumer.pop() {
                        Ok(f) => {
                            self.stats.queued.fetch_sub(1, Ordering::Release);
                            self.pending = Some(f);
                        }
                        Err(_) => break,
                    }
                }
                let f = self.pending.as_ref().expect("pending frame");
                if f.position < self.position {
                    self.pending = None;
                    continue;
                }
                if f.position == self.position {
                    sample = Some(f.samples);
                    self.pending = None;
                }
                break;
            }
            if let Some(s) = sample {
                out.fill((s[0] + s[1]) * 0.5);
                if ch >= 2 {
                    out[0] = s[0];
                    out[1] = s[1];
                }
            } else {
                out.fill(0.0);
                if self.position < self.total {
                    self.stats.underrun_frames.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.position = self.position.saturating_add(1);
        }
    }
    /// Reset clears DSP state only; use a newly opened stream to rewind.
    fn reset(&mut self) {}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_stream_matches_pcm_and_has_silent_eof() {
        let path = std::env::temp_dir().join(format!("soniclaw-stream-{}.wav", std::process::id()));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..100 {
            w.write_sample(i as i16 * 100).unwrap();
        }
        w.finalize().unwrap();
        let (mut handle, mut player) = open_wav(&path, 256).unwrap();
        assert!(handle.prebuffer(100, Duration::from_secs(2)));
        player
            .prepare(ProcessSpec {
                sample_rate: 48000.0,
                max_block_frames: 128,
                channels: 2,
            })
            .unwrap();
        let mut d = [0.0; 256];
        player.process(
            &mut AudioBuffer::new(&mut d, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        for i in 0..100 {
            assert_eq!(d[i * 2], i as f32 * 100.0 / 32768.0);
            assert_eq!(d[i * 2], d[i * 2 + 1]);
        }
        assert!(d[200..].iter().all(|x| *x == 0.0));
        assert_eq!(handle.stats.underrun_frames.load(Ordering::Relaxed), 0);
        handle.stop();
        std::fs::remove_file(path).unwrap();
    }
}

pub mod articulations;
pub mod clips;

pub use articulations::{DiskInstrument, ZoneRule};
