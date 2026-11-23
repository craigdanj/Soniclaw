//! Prepared mono sample-rate signal patch with explicitly delayed feedback edges.
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::collections::VecDeque;

/// A small single-sample processor. Allocate in prepare and keep tick real-time safe.
pub trait SampleNode: Send {
    fn prepare(&mut self, _sample_rate: f64) -> Result<(), Error> {
        Ok(())
    }
    fn tick(&mut self, input: f32) -> f32;
    fn reset(&mut self) {}
}
pub struct Sum;
impl SampleNode for Sum {
    fn tick(&mut self, x: f32) -> f32 {
        x
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub delay_samples: usize,
    pub gain: f32,
}
struct Path {
    edge: Edge,
    memory: Vec<f32>,
    index: usize,
}

pub struct FeedbackPatch {
    nodes: Vec<Box<dyn SampleNode>>,
    paths: Vec<Path>,
    outgoing: Vec<Vec<usize>>,
    delayed: Vec<usize>,
    order: Vec<usize>,
    values: Vec<f32>,
    input: usize,
    output: usize,
    spec: Option<ProcessSpec>,
}

impl FeedbackPatch {
    pub fn new(
        nodes: Vec<Box<dyn SampleNode>>,
        edges: &[Edge],
        input: usize,
        output: usize,
    ) -> Result<Self, Error> {
        let count = nodes.len();
        if !(1..=128).contains(&count) || edges.len() > 512 || input >= count || output >= count {
            return Err(Error::InvalidSpec);
        }
        let mut total_delay = 0;
        let mut outgoing = vec![Vec::new(); count];
        let mut incoming = vec![0; count];
        let mut delayed = Vec::new();
        for (index, edge) in edges.iter().enumerate() {
            if edge.from >= count
                || edge.to >= count
                || !(-4. ..=4.).contains(&edge.gain)
                || edge.delay_samples > 196_608
            {
                return Err(Error::InvalidRouting);
            }
            total_delay += edge.delay_samples;
            if total_delay > 1_048_576 {
                return Err(Error::InvalidSpec);
            }
            if edge.delay_samples == 0 {
                outgoing[edge.from].push(index);
                incoming[edge.to] += 1;
            } else {
                delayed.push(index);
            }
        }
        let mut ready: VecDeque<_> = incoming
            .iter()
            .enumerate()
            .filter_map(|(id, &n)| (n == 0).then_some(id))
            .collect();
        let mut order = Vec::with_capacity(count);
        while let Some(node) = ready.pop_front() {
            order.push(node);
            for &index in &outgoing[node] {
                let destination = edges[index].to;
                incoming[destination] -= 1;
                if incoming[destination] == 0 {
                    ready.push_back(destination);
                }
            }
        }
        if order.len() != count {
            return Err(Error::InvalidRouting);
        }
        let paths = edges
            .iter()
            .map(|&edge| Path {
                edge,
                memory: vec![0.; edge.delay_samples],
                index: 0,
            })
            .collect();
        Ok(Self {
            nodes,
            paths,
            outgoing,
            delayed,
            order,
            values: vec![0.; count],
            input,
            output,
            spec: None,
        })
    }

    /// Change a routing gain by its index in the construction slice.
    pub fn set_edge_gain(&mut self, index: usize, gain: f32) -> Result<(), Error> {
        if !(-4. ..=4.).contains(&gain) {
            return Err(Error::InvalidParameter);
        }
        self.paths
            .get_mut(index)
            .ok_or(Error::InvalidRouting)?
            .edge
            .gain = gain;
        Ok(())
    }

    fn tick(&mut self, sample: f32) -> f32 {
        self.values.fill(0.);
        self.values[self.input] = bounded(sample);
        for &index in &self.delayed {
            let path = &self.paths[index];
            self.values[path.edge.to] += path.memory[path.index] * path.edge.gain;
        }
        for &id in &self.order {
            let result = bounded(self.nodes[id].tick(bounded(self.values[id])));
            self.values[id] = result;
            for &index in &self.outgoing[id] {
                let edge = self.paths[index].edge;
                self.values[edge.to] += result * edge.gain;
            }
        }
        for &index in &self.delayed {
            let path = &mut self.paths[index];
            path.memory[path.index] = self.values[path.edge.from];
            path.index = (path.index + 1) % path.memory.len();
        }
        self.values[self.output]
    }
}

fn bounded(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-32., 32.)
    } else {
        0.
    }
}

impl AudioProcessor for FeedbackPatch {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels != 1 {
            return Err(Error::InvalidSpec);
        }
        self.spec = None;
        for node in &mut self.nodes {
            node.prepare(spec.sample_rate)?;
        }
        self.reset();
        self.spec = Some(spec);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self
            .spec
            .is_none_or(|s| b.channels() != 1 || b.frames() > s.max_block_frames)
        {
            b.clear();
            return;
        }
        for sample in b.samples_mut() {
            *sample = self.tick(*sample);
        }
    }
    fn reset(&mut self) {
        for node in &mut self.nodes {
            node.reset();
        }
        for path in &mut self.paths {
            path.memory.fill(0.);
            path.index = 0;
        }
        self.values.fill(0.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn two() -> Vec<Box<dyn SampleNode>> {
        vec![Box::new(Sum), Box::new(Sum)]
    }
    #[test]
    fn delayed_cycle_has_exact_echo_and_zero_delay_fails() {
        let direct = Edge {
            from: 0,
            to: 1,
            delay_samples: 0,
            gain: 1.,
        };
        let return_path = Edge {
            from: 1,
            to: 0,
            delay_samples: 3,
            gain: 0.5,
        };
        assert!(matches!(
            FeedbackPatch::new(
                two(),
                &[
                    direct,
                    Edge {
                        delay_samples: 0,
                        ..return_path
                    }
                ],
                0,
                1
            ),
            Err(Error::InvalidRouting)
        ));
        let make = || FeedbackPatch::new(two(), &[direct, return_path], 0, 1).unwrap();
        let process = |size: usize| {
            let mut g = make();
            g.prepare(ProcessSpec {
                sample_rate: 48_000.,
                channels: 1,
                max_block_frames: 32,
            })
            .unwrap();
            let mut wave = [0.; 16];
            wave[0] = 1.;
            let memory = allocation_counter::measure(|| {
                for block in wave.chunks_mut(size) {
                    g.process(
                        &mut AudioBuffer::new(block, 1).unwrap(),
                        &crate::advanced::context(),
                    );
                }
            });
            assert_eq!(memory.count_total, 0);
            g.reset();
            let mut blank = [0.; 16];
            g.process(
                &mut AudioBuffer::new(&mut blank, 1).unwrap(),
                &crate::advanced::context(),
            );
            assert_eq!(blank, [0.; 16]);
            wave
        };
        assert_eq!(process(5), process(16));
        assert_eq!(
            process(5),
            [1., 0., 0., 0.5, 0., 0., 0.25, 0., 0., 0.125, 0., 0., 0.0625, 0., 0., 0.03125]
        );
    }
}
