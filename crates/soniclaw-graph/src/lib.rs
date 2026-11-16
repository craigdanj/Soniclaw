//! Prepared acyclic routing with automatic alignment at fan-in and output buses.
use soniclaw_core::*;
pub type NodeId = usize;
const MAX_COMPENSATION: usize = 262144;
#[derive(Default)]
struct DelayLine {
    data: Vec<f32>,
    cursor: usize,
}
impl DelayLine {
    fn prepare(&mut self, frames: usize, channels: usize) -> Result<(), Error> {
        if frames > MAX_COMPENSATION {
            return Err(Error::InvalidRouting);
        }
        self.data = vec![0.0; frames * channels];
        self.cursor = 0;
        Ok(())
    }
    fn add(&mut self, source: &[f32], destination: &mut [f32], gain: f32) {
        if self.data.is_empty() {
            for (o, i) in destination.iter_mut().zip(source) {
                *o += *i * gain;
            }
            return;
        }
        for (o, i) in destination.iter_mut().zip(source) {
            let delayed = self.data[self.cursor];
            self.data[self.cursor] = *i;
            self.cursor = (self.cursor + 1) % self.data.len();
            *o += delayed * gain;
        }
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.cursor = 0;
    }
}
struct Edge {
    source: NodeId,
    gain: f32,
    delay: DelayLine,
}
struct Node {
    processor: Box<dyn AudioProcessor>,
    input_gain: f32,
    input_delay: DelayLine,
    incoming: Vec<Edge>,
}
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Node>,
    outputs: Vec<Edge>,
    order: Vec<NodeId>,
    buffers: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    spec: Option<ProcessSpec>,
    latency: usize,
}
impl Graph {
    pub fn add(&mut self, processor: impl AudioProcessor + 'static) -> NodeId {
        self.spec = None;
        let id = self.nodes.len();
        self.nodes.push(Node {
            processor: Box::new(processor),
            input_gain: 0.0,
            input_delay: DelayLine::default(),
            incoming: Vec::new(),
        });
        id
    }
    pub fn connect(&mut self, from: NodeId, to: NodeId, gain: f32) -> Result<(), Error> {
        if from >= self.nodes.len()
            || to >= self.nodes.len()
            || from == to
            || !gain.is_finite()
            || self.nodes[to].incoming.iter().any(|e| e.source == from)
        {
            return Err(Error::InvalidRouting);
        }
        self.nodes[to].incoming.push(Edge {
            source: from,
            gain,
            delay: DelayLine::default(),
        });
        self.spec = None;
        Ok(())
    }
    pub fn set_input(&mut self, node: NodeId, gain: f32) -> Result<(), Error> {
        if !gain.is_finite() {
            return Err(Error::InvalidRouting);
        }
        self.nodes
            .get_mut(node)
            .ok_or(Error::InvalidRouting)?
            .input_gain = gain;
        self.spec = None;
        Ok(())
    }
    pub fn set_output(&mut self, node: NodeId, gain: f32) -> Result<(), Error> {
        if node >= self.nodes.len() || !gain.is_finite() {
            return Err(Error::InvalidRouting);
        }
        self.outputs.retain(|e| e.source != node);
        self.outputs.push(Edge {
            source: node,
            gain,
            delay: DelayLine::default(),
        });
        self.spec = None;
        Ok(())
    }
}
impl AudioProcessor for Graph {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.spec = None;
        spec.validate()?;
        if self.nodes.is_empty() || self.outputs.is_empty() {
            return Err(Error::InvalidRouting);
        }
        self.order.clear();
        let mut pending: Vec<_> = self.nodes.iter().map(|n| n.incoming.len()).collect();
        self.order.extend(
            pending
                .iter()
                .enumerate()
                .filter_map(|(i, &n)| (n == 0).then_some(i)),
        );
        let mut cursor = 0;
        while cursor < self.order.len() {
            let id = self.order[cursor];
            cursor += 1;
            for (destination, node) in self.nodes.iter().enumerate() {
                if node.incoming.iter().any(|e| e.source == id) {
                    pending[destination] -= 1;
                    if pending[destination] == 0 {
                        self.order.push(destination);
                    }
                }
            }
        }
        if self.order.len() != self.nodes.len() {
            return Err(Error::InvalidRouting);
        }
        let samples = spec.channels * spec.max_block_frames;
        self.scratch = vec![0.0; samples];
        self.buffers = vec![vec![0.0; samples]; self.nodes.len()];
        let mut latency = vec![0usize; self.nodes.len()];
        for &id in &self.order {
            let node = &mut self.nodes[id];
            node.processor.prepare(spec)?;
            let upstream = node
                .incoming
                .iter()
                .map(|e| latency[e.source])
                .max()
                .unwrap_or(0);
            node.input_delay.prepare(
                if node.input_gain != 0.0 { upstream } else { 0 },
                spec.channels,
            )?;
            for edge in &mut node.incoming {
                edge.delay
                    .prepare(upstream - latency[edge.source], spec.channels)?;
            }
            latency[id] = upstream
                .checked_add(node.processor.latency_frames())
                .ok_or(Error::InvalidRouting)?;
        }
        self.latency = self
            .outputs
            .iter()
            .map(|e| latency[e.source])
            .max()
            .unwrap_or(0);
        for edge in &mut self.outputs {
            edge.delay
                .prepare(self.latency - latency[edge.source], spec.channels)?;
        }
        self.spec = Some(spec);
        Ok(())
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            audio.clear();
            return;
        };
        if audio.channels() != spec.channels || audio.frames() > spec.max_block_frames {
            audio.clear();
            return;
        }
        let length = audio.samples().len();
        for &id in &self.order {
            let scratch = &mut self.scratch[..length];
            scratch.fill(0.0);
            let node = &mut self.nodes[id];
            if node.input_gain != 0.0 {
                node.input_delay
                    .add(audio.samples(), scratch, node.input_gain);
            }
            for edge in &mut node.incoming {
                edge.delay
                    .add(&self.buffers[edge.source][..length], scratch, edge.gain);
            }
            if let Ok(mut block) = AudioBuffer::new(scratch, spec.channels) {
                node.processor.process(&mut block, ctx);
            }
            self.buffers[id][..length].copy_from_slice(scratch);
        }
        audio.clear();
        for edge in &mut self.outputs {
            edge.delay.add(
                &self.buffers[edge.source][..length],
                audio.samples_mut(),
                edge.gain,
            );
        }
    }
    fn reset(&mut self) {
        for node in &mut self.nodes {
            node.processor.reset();
            node.input_delay.reset();
            for e in &mut node.incoming {
                e.delay.reset();
            }
        }
        for e in &mut self.outputs {
            e.delay.reset();
        }
        for b in &mut self.buffers {
            b.fill(0.0);
        }
        self.scratch.fill(0.0);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Multiply(f32);
    impl AudioProcessor for Multiply {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for s in b.samples_mut() {
                *s *= self.0;
            }
        }
        fn reset(&mut self) {}
    }
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 64,
            channels: 1,
        }
    }
    #[test]
    fn parallel_branches_sum_and_handle_variable_lengths() {
        let mut g = Graph::default();
        let a = g.add(Multiply(2.0));
        let b = g.add(Multiply(3.0));
        let bus = g.add(Multiply(0.5));
        g.set_input(a, 1.0).unwrap();
        g.set_input(b, 1.0).unwrap();
        g.connect(a, bus, 1.0).unwrap();
        g.connect(b, bus, 1.0).unwrap();
        g.set_output(bus, 1.0).unwrap();
        g.prepare(spec()).unwrap();
        for size in [64, 7, 0] {
            let mut data = vec![1.0; size];
            g.process(
                &mut AudioBuffer::new(&mut data, 1).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
            assert!(data.iter().all(|s| *s == 2.5));
        }
    }
    #[test]
    fn cycles_and_duplicate_edges_are_rejected() {
        let mut g = Graph::default();
        let a = g.add(Multiply(1.0));
        let b = g.add(Multiply(1.0));
        g.connect(a, b, 1.0).unwrap();
        assert!(g.connect(a, b, 1.0).is_err());
        g.connect(b, a, 1.0).unwrap();
        g.set_output(b, 1.0).unwrap();
        assert_eq!(g.prepare(spec()), Err(Error::InvalidRouting));
    }
}
#[cfg(test)]
mod latency_tests {
    use super::*;
    struct Delayed {
        delay: usize,
        line: DelayLine,
    }
    impl AudioProcessor for Delayed {
        fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
            self.line.prepare(self.delay, s.channels)
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            if self.line.data.is_empty() {
                return;
            }
            for x in b.samples_mut() {
                std::mem::swap(x, &mut self.line.data[self.line.cursor]);
                self.line.cursor = (self.line.cursor + 1) % self.line.data.len();
            }
        }
        fn reset(&mut self) {
            self.line.reset();
        }
        fn latency_frames(&self) -> usize {
            self.delay
        }
    }
    #[test]
    fn fanin_and_output_buses_are_latency_aligned() {
        for bus in [false, true] {
            let mut g = Graph::default();
            let a = g.add(Delayed {
                delay: 0,
                line: DelayLine::default(),
            });
            let b = g.add(Delayed {
                delay: 5,
                line: DelayLine::default(),
            });
            g.set_input(a, 1.0).unwrap();
            g.set_input(b, 1.0).unwrap();
            if bus {
                let sum = g.add(Delayed {
                    delay: 0,
                    line: DelayLine::default(),
                });
                g.connect(a, sum, 1.0).unwrap();
                g.connect(b, sum, 1.0).unwrap();
                g.set_output(sum, 1.0).unwrap();
            } else {
                g.set_output(a, 1.0).unwrap();
                g.set_output(b, 1.0).unwrap();
            }
            g.prepare(ProcessSpec {
                sample_rate: 48000.0,
                max_block_frames: 8,
                channels: 1,
            })
            .unwrap();
            assert_eq!(g.latency_frames(), 5);
            let ctx = ProcessContext {
                events: &[],
                transport: Transport::default(),
            };
            let mut first = [1.0, 0.0, 0.0];
            g.process(&mut AudioBuffer::new(&mut first, 1).unwrap(), &ctx);
            assert_eq!(first, [0.0; 3]);
            let mut second = [0.0; 8];
            g.process(&mut AudioBuffer::new(&mut second, 1).unwrap(), &ctx);
            assert_eq!(second, [0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
            g.reset();
        }
    }
}
