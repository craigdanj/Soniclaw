//! Dependency-level worker execution behind a bounded, timestamped audio bridge.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc,
};
const SAMPLES: usize = 8192;
const EVENTS: usize = 1024;
#[derive(Clone)]
struct Packet {
    clock: u64,
    generation: u64,
    frames: usize,
    transport: Transport,
    count: usize,
    events: [Event; EVENTS],
    pcm: [f32; SAMPLES],
}
impl Packet {
    fn empty() -> Self {
        Self {
            clock: 0,
            generation: 0,
            frames: 0,
            transport: Transport::default(),
            count: 0,
            events: [Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            }; EVENTS],
            pcm: [0.; SAMPLES],
        }
    }
}
#[derive(Default)]
pub struct Telemetry {
    pub dropped_inputs: AtomicU64,
    pub missing_frames: AtomicU64,
    pub late_packets: AtomicU64,
    pub completed_blocks: AtomicU64,
    pub failures: AtomicU64,
}
pub struct Owner {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
    pub telemetry: Arc<Telemetry>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
pub struct Bridge {
    input: rtrb::Producer<Packet>,
    output: rtrb::Consumer<Packet>,
    current: Option<Packet>,
    spec: ProcessSpec,
    clock: u64,
    generation: u64,
    pipeline: usize,
    latency: usize,
    stop: Arc<AtomicBool>,
    telemetry: Arc<Telemetry>,
}
struct Task {
    id: usize,
    pcm: Vec<f32>,
    frames: usize,
    events: Vec<Event>,
    transport: Transport,
    reset: bool,
}
struct Routing {
    input_gain: f32,
    input_delay: DelayLine,
    incoming: Vec<Edge>,
}
impl Graph {
    /// Prepare once and transfer DSP ownership to fixed workers. Drop `Owner` off callback.
    /// This mode requires processors safe to prepare on control and process/reset on their worker.
    pub fn parallel(
        mut self,
        spec: ProcessSpec,
        workers: usize,
        pipeline_frames: usize,
    ) -> Result<(Owner, Bridge), Error> {
        if workers == 0
            || workers > 16
            || self.nodes.len() > 64
            || spec.channels > 8
            || spec.max_block_frames > 1024
            || pipeline_frames < spec.max_block_frames * 2
            || pipeline_frames > 262144
        {
            return Err(Error::InvalidSpec);
        }
        self.prepare(spec)?;
        let latency = self
            .latency
            .checked_add(pipeline_frames)
            .ok_or(Error::InvalidSpec)?;
        let mut levels = vec![0; self.nodes.len()];
        for &id in &self.order {
            levels[id] = self.nodes[id]
                .incoming
                .iter()
                .map(|e| levels[e.source] + 1)
                .max()
                .unwrap_or(0);
        }
        let maximum = *levels.iter().max().ok_or(Error::InvalidRouting)?;
        let waves: Vec<Vec<usize>> = (0..=maximum)
            .map(|l| {
                self.order
                    .iter()
                    .copied()
                    .filter(|id| levels[*id] == l)
                    .collect()
            })
            .collect();
        let mut groups: Vec<Vec<(usize, Box<dyn AudioProcessor>)>> =
            (0..workers).map(|_| Vec::new()).collect();
        let routing: Vec<_> = self
            .nodes
            .into_iter()
            .enumerate()
            .map(|(id, n)| {
                groups[id % workers].push((id, n.processor));
                Routing {
                    input_gain: n.input_gain,
                    input_delay: n.input_delay,
                    incoming: n.incoming,
                }
            })
            .collect();
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Telemetry::default());
        let (tx, rx) = rtrb::RingBuffer::new(32);
        let (out, receive) = rtrb::RingBuffer::new(32);
        let flag = stop.clone();
        let telemetry = stats.clone();
        let outputs = self.outputs;
        let buffers = self.buffers;
        let join = std::thread::Builder::new()
            .name("soniclaw-graph-dispatch".into())
            .spawn(move || {
                let (results, done) = mpsc::channel();
                let mut senders = Vec::new();
                let mut joins = Vec::new();
                for group in groups {
                    let (sender, receiver) = mpsc::channel::<Task>();
                    senders.push(sender);
                    let result = results.clone();
                    joins.push(std::thread::spawn(move || {
                        let mut processors = group;
                        while let Ok(mut t) = receiver.recv() {
                            let p = &mut processors
                                .iter_mut()
                                .find(|(id, _)| *id == t.id)
                                .expect("assigned worker")
                                .1;
                            if t.reset {
                                p.reset();
                            }
                            p.process(
                                &mut AudioBuffer::new(
                                    &mut t.pcm[..t.frames * spec.channels],
                                    spec.channels,
                                )
                                .expect("prepared"),
                                &ProcessContext {
                                    events: &t.events,
                                    transport: t.transport,
                                },
                            );
                            if result.send(t).is_err() {
                                break;
                            }
                        }
                    }));
                }
                drop(results);
                dispatch(
                    rx,
                    out,
                    spec,
                    routing,
                    outputs,
                    buffers,
                    waves,
                    senders,
                    &done,
                    pipeline_frames,
                    &flag,
                    &telemetry,
                );
                for j in joins {
                    let _ = j.join();
                }
            })
            .map_err(|_| Error::InvalidSpec)?;
        Ok((
            Owner {
                stop: stop.clone(),
                join: Some(join),
                telemetry: stats.clone(),
            },
            Bridge {
                input: tx,
                output: receive,
                current: None,
                spec,
                clock: 0,
                generation: 0,
                pipeline: pipeline_frames,
                latency,
                stop,
                telemetry: stats,
            },
        ))
    }
}
#[allow(clippy::too_many_arguments)]
fn dispatch(
    mut rx: rtrb::Consumer<Packet>,
    mut output: rtrb::Producer<Packet>,
    spec: ProcessSpec,
    mut routing: Vec<Routing>,
    mut outputs: Vec<Edge>,
    mut buffers: Vec<Vec<f32>>,
    waves: Vec<Vec<usize>>,
    senders: Vec<mpsc::Sender<Task>>,
    done: &mpsc::Receiver<Task>,
    pipeline: usize,
    stop: &AtomicBool,
    telemetry: &Telemetry,
) {
    let mut expected = None;
    let mut generation = 0;
    let mut tasks: Vec<_> = (0..routing.len())
        .map(|id| {
            Some(Task {
                id,
                pcm: vec![0.; spec.max_block_frames * spec.channels],
                frames: 0,
                events: Vec::with_capacity(EVENTS),
                transport: Transport::default(),
                reset: false,
            })
        })
        .collect();
    while !stop.load(Ordering::Acquire) {
        let Ok(mut packet) = rx.pop() else {
            std::thread::sleep(std::time::Duration::from_micros(50));
            continue;
        };
        let reset = expected.is_some()
            && (expected != Some(packet.clock) || generation != packet.generation);
        expected = Some(packet.clock + packet.frames as u64);
        generation = packet.generation;
        let len = packet.frames * spec.channels;
        if reset {
            for r in &mut routing {
                r.input_delay.reset();
                for e in &mut r.incoming {
                    e.delay.reset();
                }
            }
            for e in &mut outputs {
                e.delay.reset();
            }
        }
        for wave in &waves {
            for &id in wave {
                let mut t = tasks[id].take().expect("returned task");
                t.frames = packet.frames;
                t.transport = packet.transport;
                t.events.clear();
                t.events.extend_from_slice(&packet.events[..packet.count]);
                t.reset = reset;
                t.pcm[..len].fill(0.);
                let r = &mut routing[id];
                r.input_delay
                    .add(&packet.pcm[..len], &mut t.pcm[..len], r.input_gain);
                for edge in &mut r.incoming {
                    edge.delay
                        .add(&buffers[edge.source][..len], &mut t.pcm[..len], edge.gain);
                }
                if senders[id % senders.len()].send(t).is_err() {
                    telemetry.failures.fetch_add(1, Ordering::Relaxed);
                    return;
                }
            }
            for _ in wave {
                let Ok(t) = done.recv() else {
                    telemetry.failures.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                let id = t.id;
                buffers[id][..len].copy_from_slice(&t.pcm[..len]);
                tasks[id] = Some(t);
            }
        }
        packet.pcm[..len].fill(0.);
        for edge in &mut outputs {
            edge.delay.add(
                &buffers[edge.source][..len],
                &mut packet.pcm[..len],
                edge.gain,
            );
        }
        packet.clock += pipeline as u64;
        if output.push(packet).is_err() {
            telemetry.late_packets.fetch_add(1, Ordering::Relaxed);
        }
        telemetry.completed_blocks.fetch_add(1, Ordering::Release);
    }
    // Sender destruction stops workers before the outer dispatcher joins them.
}
impl AudioProcessor for Bridge {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.sample_rate != self.spec.sample_rate
            || s.channels != self.spec.channels
            || s.max_block_frames != self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let frames = b.frames();
        if self.stop.load(Ordering::Acquire)
            || b.channels() != self.spec.channels
            || frames > self.spec.max_block_frames
            || ctx.events.len() > EVENTS
            || validate_events(ctx.events, frames).is_err()
        {
            b.clear();
            return;
        }
        if frames == 0 {
            return;
        }
        let mut p = Packet::empty();
        p.clock = self.clock;
        p.generation = self.generation;
        p.frames = frames;
        p.transport = ctx.transport;
        p.count = ctx.events.len();
        p.events[..p.count].copy_from_slice(ctx.events);
        p.pcm[..b.samples().len()].copy_from_slice(b.samples());
        if self.input.push(p).is_err() {
            self.telemetry
                .dropped_inputs
                .fetch_add(1, Ordering::Relaxed);
        }
        b.clear();
        for frame in 0..frames {
            let clock = self.clock + frame as u64;
            loop {
                if self.current.as_ref().is_some_and(|p| {
                    p.generation != self.generation || p.clock + p.frames as u64 <= clock
                }) {
                    self.current = None;
                    self.telemetry.late_packets.fetch_add(1, Ordering::Relaxed);
                }
                if self.current.is_none() {
                    self.current = self.output.pop().ok();
                    if self.current.is_some() {
                        continue;
                    }
                }
                break;
            }
            if let Some(p) = &self.current {
                if p.clock <= clock {
                    let offset = (clock - p.clock) as usize * self.spec.channels;
                    let dest = frame * self.spec.channels;
                    b.samples_mut()[dest..dest + self.spec.channels]
                        .copy_from_slice(&p.pcm[offset..offset + self.spec.channels]);
                    if clock + 1 == p.clock + p.frames as u64 {
                        self.current = None;
                    }
                    continue;
                }
            }
            if clock >= self.pipeline as u64 {
                self.telemetry
                    .missing_frames
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        self.clock += frames as u64;
    }
    fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.current = None;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Multiply(f32, Option<Arc<std::sync::Barrier>>);
    impl AudioProcessor for Multiply {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            if let Some(barrier) = &self.1 {
                barrier.wait();
            }
            for s in b.samples_mut() {
                *s *= self.0;
            }
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn concurrent_branches_dependency_sum_and_bounded_callback() {
        let spec = ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 64,
        };
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut graph = Graph::default();
        let a = graph.add(Multiply(2., Some(barrier.clone())));
        let b = graph.add(Multiply(3., Some(barrier)));
        let c = graph.add(Multiply(0.5, None));
        graph.set_input(a, 1.).unwrap();
        graph.set_input(b, 1.).unwrap();
        graph.connect(a, c, 1.).unwrap();
        graph.connect(b, c, 1.).unwrap();
        graph.set_output(c, 1.).unwrap();
        let (owner, mut audio) = graph.parallel(spec, 2, 128).unwrap();
        let ctx = ProcessContext {
            events: &[],
            transport: Transport::default(),
        };
        for i in 0..5 {
            let mut pcm = [1.; 64];
            let measured = allocation_counter::measure(|| {
                audio.process(&mut AudioBuffer::new(&mut pcm, 1).unwrap(), &ctx)
            });
            assert_eq!(measured.count_total, 0);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while owner.telemetry.completed_blocks.load(Ordering::Acquire) < i + 1 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            if i >= 2 {
                assert!(pcm.iter().all(|s| *s == 2.5));
            }
        }
        assert_eq!(owner.telemetry.missing_frames.load(Ordering::Relaxed), 0);
        assert_eq!(audio.latency_frames(), 128);
        assert_eq!(owner.telemetry.late_packets.load(Ordering::Relaxed), 0);
        drop(owner);
    }
    #[test]
    fn missed_deadline_silences_and_reset_rejects_old_packets() {
        let s = ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 64,
        };
        let mut g = Graph::default();
        let n = g.add(Multiply(2., None));
        g.set_input(n, 1.).unwrap();
        g.set_output(n, 1.).unwrap();
        let (owner, mut audio) = g.parallel(s, 1, 128).unwrap();
        owner.stop.store(true, Ordering::Release);
        let mut p = [1.; 64];
        audio.process(
            &mut AudioBuffer::new(&mut p, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert_eq!(p, [0.; 64]);
        audio.reset();
        assert_eq!(audio.generation, 1);
    }
}
