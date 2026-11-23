//! Prepared, fixed-latency graph handover; control destroys retired resources.
use soniclaw_core::*;
struct Prepared {
    processor: Box<dyn AudioProcessor>,
    delay: Vec<f32>,
    cursor: usize,
    seed: Vec<Event>,
    scratch: Vec<Event>,
    seeded: bool,
}
impl Prepared {
    fn new(mut p: Box<dyn AudioProcessor>, s: ProcessSpec, budget: usize) -> Result<Self, Error> {
        p.prepare(s)?;
        let n = p.latency_frames();
        if n > budget {
            return Err(Error::InvalidRouting);
        }
        Ok(Self {
            processor: p,
            delay: vec![0.0; (budget - n) * s.channels],
            cursor: 0,
            seed: vec![],
            scratch: Vec::with_capacity(1024),
            seeded: false,
        })
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if !self.seeded && !self.seed.is_empty() {
            self.seeded = true;
            self.scratch.clear();
            if self.seed.len() + c.events.len() > 1024 {
                self.scratch.push(Event {
                    frame: 0,
                    message: MidiMessage::AllNotesOff,
                });
            } else {
                self.scratch.extend_from_slice(&self.seed);
                self.scratch.extend_from_slice(c.events);
            }
            self.processor.process(
                b,
                &ProcessContext {
                    events: &self.scratch,
                    transport: c.transport,
                },
            );
        } else {
            self.processor.process(b, c);
        }
        if !self.delay.is_empty() {
            for x in b.samples_mut() {
                std::mem::swap(x, &mut self.delay[self.cursor]);
                self.cursor = (self.cursor + 1) % self.delay.len();
            }
        }
    }
    fn reset(&mut self) {
        self.processor.reset();
        self.delay.fill(0.0);
        self.cursor = 0;
        self.seeded = true;
    }
}
pub struct GraphController {
    requests: rtrb::Producer<Box<Prepared>>,
    retired: rtrb::Consumer<Box<Prepared>>,
    spec: ProcessSpec,
    budget: usize,
}
impl GraphController {
    pub fn replace(&mut self, p: Box<dyn AudioProcessor>) -> Result<(), Error> {
        self.replace_with_notes(p, &[])
    }
    pub fn replace_with_notes(
        &mut self,
        p: Box<dyn AudioProcessor>,
        notes: &[MidiMessage],
    ) -> Result<(), Error> {
        if notes.len() > 128 {
            return Err(Error::InvalidEvents);
        }
        let events: Vec<_> = notes
            .iter()
            .map(|m| Event {
                frame: 0,
                message: *m,
            })
            .collect();
        validate_events(&events, 1)?;
        self.collect_retired();
        if self.requests.slots() == 0 {
            return Err(Error::InvalidRouting);
        }
        let mut p = Box::new(Prepared::new(p, self.spec, self.budget)?);
        p.seed = events;
        self.requests.push(p).map_err(|_| Error::InvalidRouting)
    }
    pub fn collect_retired(&mut self) -> usize {
        let mut n = 0;
        while let Ok(p) = self.retired.pop() {
            drop(p);
            n += 1;
        }
        n
    }
}
pub struct LiveGraph {
    current: Box<Prepared>,
    previous: Option<Box<Prepared>>,
    requests: rtrb::Consumer<Box<Prepared>>,
    retired: rtrb::Producer<Box<Prepared>>,
    old: Vec<f32>,
    spec: ProcessSpec,
    budget: usize,
    fade: usize,
    position: usize,
    warmup: usize,
}
pub fn graph_pair(
    p: Box<dyn AudioProcessor>,
    s: ProcessSpec,
    budget: usize,
    fade: usize,
) -> Result<(GraphController, LiveGraph), Error> {
    s.validate()?;
    if budget > 262144 || !(1..=65536).contains(&fade) {
        return Err(Error::InvalidSpec);
    }
    let current = Box::new(Prepared::new(p, s, budget)?);
    let (tx, requests) = rtrb::RingBuffer::new(2);
    let (retired, rx) = rtrb::RingBuffer::new(2);
    Ok((
        GraphController {
            requests: tx,
            retired: rx,
            spec: s,
            budget,
        },
        LiveGraph {
            current,
            previous: None,
            requests,
            retired,
            old: vec![0.0; s.channels * s.max_block_frames],
            spec: s,
            budget,
            fade,
            position: fade,
            warmup: 0,
        },
    ))
}
impl AudioProcessor for LiveGraph {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.sample_rate != self.spec.sample_rate
            || s.channels != self.spec.channels
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if b.channels() != self.spec.channels || b.frames() > self.spec.max_block_frames {
            b.clear();
            return;
        }
        if self.position >= self.fade && self.previous.is_some() && self.retired.slots() > 0 {
            let p = self.previous.take().expect("previous");
            if let Err(rtrb::PushError::Full(p)) = self.retired.push(p) {
                self.previous = Some(p);
            }
        }
        if self.previous.is_none() {
            if let Ok(p) = self.requests.pop() {
                self.previous = Some(std::mem::replace(&mut self.current, p));
                self.position = 0;
                self.warmup = self.budget;
            }
        }
        let n = b.samples().len();
        self.old[..n].copy_from_slice(b.samples());
        self.current.process(b, c);
        if self.position < self.fade {
            if let Some(p) = &mut self.previous {
                p.process(
                    &mut AudioBuffer::new(&mut self.old[..n], self.spec.channels).expect("frames"),
                    c,
                );
                for (f, old) in b
                    .samples_mut()
                    .chunks_exact_mut(self.spec.channels)
                    .zip(self.old[..n].chunks_exact(self.spec.channels))
                {
                    let t = if self.warmup > 0 {
                        self.warmup -= 1;
                        0.0
                    } else {
                        self.position = self.position.saturating_add(1);
                        (self.position as f32 / self.fade as f32).min(1.0)
                    };
                    for (x, y) in f.iter_mut().zip(old) {
                        *x = *y * (1.0 - t) + *x * t;
                    }
                }
            }
        }
    }
    fn reset(&mut self) {
        self.current.reset();
        if let Some(p) = &mut self.previous {
            p.reset();
        }
        self.position = self.fade;
        self.warmup = 0;
        self.old.fill(0.0);
    }
    fn latency_frames(&self) -> usize {
        self.budget
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Gain(f32);
    impl AudioProcessor for Gain {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for x in b.samples_mut() {
                *x *= self.0;
            }
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn exact_fade_and_control_retirement() {
        let s = ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 16,
            channels: 1,
        };
        let (mut ctl, mut p) = graph_pair(Box::new(Gain(1.0)), s, 0, 8).unwrap();
        ctl.replace(Box::new(Gain(0.5))).unwrap();
        let mut d = [1.0; 16];
        let c = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        p.process(&mut AudioBuffer::new(&mut d, 1).unwrap(), &c);
        assert_eq!(d[0], 0.9375);
        assert_eq!(d[7], 0.5);
        assert_eq!(ctl.collect_retired(), 0);
        p.process(&mut AudioBuffer::new(&mut d, 1).unwrap(), &c);
        assert_eq!(ctl.collect_retired(), 1);
    }
}
