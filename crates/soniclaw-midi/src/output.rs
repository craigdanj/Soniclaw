//! Bounded scheduled output worker. Hardware driver sends never run on the audio callback.
use super::wire::Packet;
use rtrb::{Producer, RingBuffer};
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
pub type OutputError = Box<dyn std::error::Error + Send + Sync>;
pub trait Sink: Send {
    fn send(&mut self, message: &Packet) -> Result<(), OutputError>;
}
#[derive(Clone, Copy)]
pub struct Clock {
    rate: f64,
    sample: u64,
    instant: Instant,
}
impl Clock {
    pub fn new(rate: f64, sample: u64, instant: Instant) -> Result<Self, OutputError> {
        if !rate.is_finite() || !(1.0..=768000.0).contains(&rate) {
            return Err("invalid MIDI output clock".into());
        }
        Ok(Self {
            rate,
            sample,
            instant,
        })
    }
    fn deadline(&self, sample: u64) -> Result<Instant, soniclaw_core::Error> {
        let delta = sample as i128 - self.sample as i128;
        let seconds = delta.abs() as f64 / self.rate;
        if seconds > 86400.0 {
            return Err(soniclaw_core::Error::InvalidEvents);
        }
        let duration = Duration::from_secs_f64(seconds);
        if delta >= 0 {
            self.instant.checked_add(duration)
        } else {
            self.instant.checked_sub(duration)
        }
        .ok_or(soniclaw_core::Error::InvalidEvents)
    }
}
#[derive(Default)]
struct Counters {
    sent: AtomicU64,
    late: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
}
#[derive(Debug, Serialize)]
pub struct Stats {
    pub sent: u64,
    pub late: u64,
    pub dropped: u64,
    pub errors: u64,
}
struct Shared {
    stop: AtomicBool,
    panic: AtomicBool,
    counters: Counters,
}
impl Shared {
    fn stats(&self) -> Stats {
        Stats {
            sent: self.counters.sent.load(Ordering::Relaxed),
            late: self.counters.late.load(Ordering::Relaxed),
            dropped: self.counters.dropped.load(Ordering::Relaxed),
            errors: self.counters.errors.load(Ordering::Relaxed),
        }
    }
}
#[derive(Clone, Copy)]
struct Timed {
    deadline: Instant,
    message: Packet,
}
pub struct Sender {
    queue: Producer<Timed>,
    shared: Arc<Shared>,
    clock: Clock,
    last: Option<u64>,
}
pub struct OutputWorker {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Sender {
    /// Nonblocking fixed-capacity send. Supply nondecreasing absolute audio sample positions.
    pub fn send(&mut self, sample: u64, message: Packet) -> Result<(), soniclaw_core::Error> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(soniclaw_core::Error::NotPrepared);
        }
        if self.last.is_some_and(|p| sample < p) {
            return Err(soniclaw_core::Error::InvalidEvents);
        }
        let deadline = self.clock.deadline(sample)?;
        if self.queue.push(Timed { deadline, message }).is_err() {
            self.shared.counters.dropped.fetch_add(1, Ordering::Relaxed);
            self.panic();
            return Err(soniclaw_core::Error::InvalidEvents);
        }
        self.last = Some(sample);
        Ok(())
    }
    pub fn panic(&mut self) {
        self.last = None;
        self.shared.panic.store(true, Ordering::Release);
    }
    pub fn stats(&self) -> Stats {
        self.shared.stats()
    }
}
impl Drop for Sender {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl OutputWorker {
    pub fn start<F>(
        capacity: usize,
        clock: Clock,
        factory: F,
    ) -> Result<(Self, Sender), OutputError>
    where
        F: FnOnce() -> Result<Box<dyn Sink>, OutputError> + Send + 'static,
    {
        if !(1..=4096).contains(&capacity) {
            return Err("MIDI output capacity must be 1..4096".into());
        }
        let (queue, receiver) = RingBuffer::<Timed>::new(capacity);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            panic: AtomicBool::new(false),
            counters: Default::default(),
        });
        let worker_shared = shared.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("soniclaw-midi-out".into())
            .spawn(move || {
                let mut sink = match factory() {
                    Ok(s) => {
                        let _ = tx.send(Ok(()));
                        s
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let mut receiver = receiver;
                let mut pending = None;
                let panic = |sink: &mut dyn Sink| {
                    for channel in 0..16 {
                        if sink
                            .send(&Packet::all_notes_off(channel).expect("channel"))
                            .is_err()
                        {
                            worker_shared
                                .counters
                                .errors
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                };
                while !worker_shared.stop.load(Ordering::Acquire) {
                    if worker_shared.panic.swap(false, Ordering::AcqRel) {
                        pending = None;
                        for _ in 0..capacity {
                            if receiver.pop().is_err() {
                                break;
                            }
                        }
                        panic(&mut *sink);
                    }
                    if pending.is_none() {
                        pending = receiver.pop().ok();
                    }
                    let Some(item) = pending else {
                        std::thread::park_timeout(Duration::from_millis(1));
                        continue;
                    };
                    let now = Instant::now();
                    if now < item.deadline {
                        std::thread::park_timeout(
                            (item.deadline - now).min(Duration::from_millis(1)),
                        );
                        continue;
                    }
                    if now.duration_since(item.deadline) > Duration::from_millis(1) {
                        worker_shared.counters.late.fetch_add(1, Ordering::Relaxed);
                    }
                    match sink.send(&item.message) {
                        Ok(()) => {
                            worker_shared.counters.sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            worker_shared
                                .counters
                                .errors
                                .fetch_add(1, Ordering::Relaxed);
                            worker_shared.panic.store(true, Ordering::Release);
                        }
                    }
                    pending = None;
                }
                panic(&mut *sink);
            })?;
        match rx.recv() {
            Ok(Ok(())) => Ok((
                Self {
                    shared: shared.clone(),
                    worker: Some(worker),
                },
                Sender {
                    queue,
                    shared,
                    clock,
                    last: None,
                },
            )),
            other => {
                let _ = worker.join();
                Err(format!("MIDI output initialization failed: {other:?}").into())
            }
        }
    }
    pub fn stats(&self) -> Stats {
        self.shared.stats()
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for OutputWorker {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(feature = "native")]
pub mod native {
    use super::*;
    #[derive(Clone, Debug)]
    pub enum Destination {
        Port(usize),
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        Virtual(String),
    }
    struct NativeSink(midir::MidiOutputConnection);
    impl Sink for NativeSink {
        fn send(&mut self, message: &Packet) -> Result<(), OutputError> {
            self.0
                .send(message.bytes())
                .map_err(|e| e.to_string().into())
        }
    }
    pub fn ports() -> Result<Vec<String>, OutputError> {
        let midi =
            midir::MidiOutput::new("Soniclaw output inventory").map_err(|e| e.to_string())?;
        midi.ports()
            .iter()
            .map(|p| midi.port_name(p).map_err(|e| e.to_string().into()))
            .collect()
    }
    pub fn connect(
        destination: Destination,
        capacity: usize,
        clock: Clock,
    ) -> Result<(OutputWorker, Sender), OutputError> {
        OutputWorker::start(capacity, clock, move || {
            let output = midir::MidiOutput::new("Soniclaw MIDI out").map_err(|e| e.to_string())?;
            let connection = match destination {
                Destination::Port(index) => {
                    let port = output
                        .ports()
                        .get(index)
                        .cloned()
                        .ok_or("MIDI output port index unavailable")?;
                    output
                        .connect(&port, "Soniclaw scheduled output")
                        .map_err(|e| e.to_string())?
                }
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                Destination::Virtual(name) => {
                    use midir::os::unix::VirtualOutput;
                    output.create_virtual(&name).map_err(|e| e.to_string())?
                }
            };
            Ok(Box::new(NativeSink(connection)))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Capture(Arc<AtomicU64>);
    impl Sink for Capture {
        fn send(&mut self, _: &Packet) -> Result<(), OutputError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    #[test]
    fn worker_sends_and_rejects_out_of_order_timestamps() {
        let count = Arc::new(AtomicU64::new(0));
        let c = count.clone();
        let clock = Clock::new(48000., 0, Instant::now()).unwrap();
        let (mut worker, mut sender) =
            OutputWorker::start(8, clock, move || Ok(Box::new(Capture(c)))).unwrap();
        sender.send(0, Packet::new(&[0xC0, 9]).unwrap()).unwrap();
        assert!(sender
            .send(u64::MAX, Packet::new(&[0xF8]).unwrap())
            .is_err());
        let start = Instant::now();
        while sender.stats().sent < 1 && start.elapsed() < Duration::from_secs(1) {
            std::thread::yield_now();
        }
        assert_eq!(sender.stats().sent, 1);
        worker.stop();
        assert!(count.load(Ordering::Relaxed) >= 17);
    }
}
