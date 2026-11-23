//! Prepared named-bus DAG with explicit channel routes and feed-forward latency compensation.
use crate::flex::Layout;
use serde::{Deserialize, Serialize};
use soniclaw_core::{Error, ProcessContext, ProcessSpec};
#[derive(Clone, Debug)]
pub struct MultiSpec {
    pub sample_rate: f64,
    pub max_block_frames: usize,
    pub layout: Layout,
}
impl MultiSpec {
    pub fn validate(&self) -> Result<(), Error> {
        self.layout.validate()?;
        ProcessSpec {
            sample_rate: self.sample_rate,
            max_block_frames: self.max_block_frames,
            channels: 1,
        }
        .validate()?;
        if self.max_block_frames > 8192 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
}
pub trait MultiProcessor: Send {
    fn prepare(&mut self, spec: MultiSpec) -> Result<(), Error>;
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        context: &ProcessContext<'_>,
    ) -> Result<(), Error>;
    fn reset(&mut self);
    fn latency_frames(&self) -> usize {
        0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub id: String,
    pub layout: Layout,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub node: Option<String>,
    pub bus: usize,
    pub channel: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub from: Endpoint,
    pub to: Endpoint,
    pub gain: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    pub schema: u32,
    pub layout: Layout,
    pub nodes: Vec<NodeDef>,
    pub connections: Vec<Connection>,
}
impl Blueprint {
    fn topology(&self) -> Result<Vec<usize>, Error> {
        self.layout.validate()?;
        if self.schema != 1 || self.nodes.len() > 32 || self.connections.len() > 512 {
            return Err(Error::InvalidRouting);
        }
        for (i, n) in self.nodes.iter().enumerate() {
            n.layout.validate()?;
            if n.id.is_empty()
                || n.id.len() > 64
                || !n
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                || self.nodes[..i].iter().any(|p| p.id == n.id)
            {
                return Err(Error::InvalidRouting);
            }
        }
        let mut degree = vec![0; self.nodes.len()];
        for (i, e) in self.connections.iter().enumerate() {
            if !e.gain.is_finite()
                || e.gain.abs() > 16.
                || self.connections[..i]
                    .iter()
                    .any(|p| p.from == e.from && p.to == e.to)
            {
                return Err(Error::InvalidRouting);
            }
            let a = self.endpoint(&e.from, true)?;
            let b = self.endpoint(&e.to, false)?;
            if a.is_some() && a == b {
                return Err(Error::InvalidRouting);
            }
            if a.is_some() {
                if let Some(b) = b {
                    degree[b] += 1;
                }
            }
        }
        let mut order: Vec<usize> = degree
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut cursor = 0;
        while cursor < order.len() {
            let id = &self.nodes[order[cursor]].id;
            cursor += 1;
            for e in &self.connections {
                if e.from.node.as_ref() == Some(id) {
                    if let Some(to) = self.endpoint(&e.to, false)? {
                        degree[to] -= 1;
                        if degree[to] == 0 {
                            order.push(to);
                        }
                    }
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(Error::InvalidRouting);
        }
        Ok(order)
    }
    fn endpoint(&self, e: &Endpoint, source: bool) -> Result<Option<usize>, Error> {
        let index = e
            .node
            .as_ref()
            .map(|id| {
                self.nodes
                    .iter()
                    .position(|n| &n.id == id)
                    .ok_or(Error::InvalidRouting)
            })
            .transpose()?;
        let buses = if let Some(i) = index {
            if source {
                &self.nodes[i].layout.outputs
            } else {
                &self.nodes[i].layout.inputs
            }
        } else if source {
            &self.layout.inputs
        } else {
            &self.layout.outputs
        };
        if buses.get(e.bus).is_none_or(|b| e.channel >= b.channels) {
            return Err(Error::InvalidRouting);
        }
        Ok(index)
    }
    pub fn validate(&self) -> Result<(), Error> {
        self.topology().map(|_| ())
    }
    pub fn build(
        &self,
        rate: f64,
        max_frames: usize,
        mut factory: impl FnMut(&NodeDef) -> Result<Box<dyn MultiProcessor>, Error>,
    ) -> Result<MultiGraph, Error> {
        let order = self.topology()?;
        let spec = MultiSpec {
            sample_rate: rate,
            max_block_frames: max_frames,
            layout: self.layout.clone(),
        };
        spec.validate()?;
        let buffer_samples = self
            .nodes
            .iter()
            .map(|n| {
                n.layout
                    .inputs
                    .iter()
                    .chain(&n.layout.outputs)
                    .map(|b| b.channels * max_frames)
                    .sum::<usize>()
            })
            .sum::<usize>();
        if buffer_samples > 64 * 1024 * 1024 {
            return Err(Error::InvalidSpec);
        }
        let mut nodes = Vec::new();
        for n in &self.nodes {
            let mut processor = factory(n)?;
            processor.prepare(MultiSpec {
                layout: n.layout.clone(),
                ..spec.clone()
            })?;
            if processor.latency_frames() > 262144 {
                return Err(Error::InvalidSpec);
            }
            nodes.push(Node {
                processor,
                inputs: n
                    .layout
                    .inputs
                    .iter()
                    .map(|b| vec![0.; max_frames * b.channels])
                    .collect(),
                outputs: n
                    .layout
                    .outputs
                    .iter()
                    .map(|b| vec![0.; max_frames * b.channels])
                    .collect(),
                layout: n.layout.clone(),
            });
        }
        let mut arrival = vec![0usize; nodes.len()];
        let mut finish = vec![0usize; nodes.len()];
        for &i in &order {
            arrival[i] = self
                .connections
                .iter()
                .filter(|e| e.to.node.as_ref() == Some(&self.nodes[i].id))
                .map(|e| {
                    self.endpoint(&e.from, true)
                        .unwrap()
                        .map_or(0, |j| finish[j])
                })
                .max()
                .unwrap_or(0);
            finish[i] = arrival[i]
                .checked_add(nodes[i].processor.latency_frames())
                .ok_or(Error::InvalidSpec)?;
            if finish[i] > 262144 {
                return Err(Error::InvalidSpec);
            }
        }
        let latency = self
            .connections
            .iter()
            .filter(|e| e.to.node.is_none())
            .map(|e| {
                self.endpoint(&e.from, true)
                    .unwrap()
                    .map_or(0, |j| finish[j])
            })
            .max()
            .unwrap_or(0);
        let mut routes = Vec::new();
        let mut delay_samples = 0;
        for e in &self.connections {
            let from = self.endpoint(&e.from, true)?;
            let to = self.endpoint(&e.to, false)?;
            let upstream = from.map_or(0, |i| finish[i]);
            let target = to.map_or(latency, |i| arrival[i]);
            let delay = target.checked_sub(upstream).ok_or(Error::InvalidRouting)?;
            delay_samples += delay;
            if delay_samples > 32 * 1024 * 1024 {
                return Err(Error::InvalidSpec);
            }
            let in_channels = from.map_or(&self.layout.inputs, |i| &self.nodes[i].layout.outputs)
                [e.from.bus]
                .channels;
            let out_channels = to.map_or(&self.layout.outputs, |i| &self.nodes[i].layout.inputs)
                [e.to.bus]
                .channels;
            routes.push(Route {
                from,
                to,
                in_bus: e.from.bus,
                out_bus: e.to.bus,
                in_channel: e.from.channel,
                out_channel: e.to.channel,
                in_channels,
                out_channels,
                gain: e.gain,
                delay: vec![0.; delay],
                cursor: 0,
            });
        }
        Ok(MultiGraph {
            spec,
            nodes,
            routes,
            order,
            latency,
        })
    }
}
struct Node {
    processor: Box<dyn MultiProcessor>,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
    layout: Layout,
}
struct Route {
    from: Option<usize>,
    to: Option<usize>,
    in_bus: usize,
    out_bus: usize,
    in_channel: usize,
    out_channel: usize,
    in_channels: usize,
    out_channels: usize,
    gain: f32,
    delay: Vec<f32>,
    cursor: usize,
}
impl Route {
    fn mix(&mut self, src: &[f32], dst: &mut [f32], frames: usize) {
        for i in 0..frames {
            let sample = src[i * self.in_channels + self.in_channel] * self.gain;
            let value = if self.delay.is_empty() {
                sample
            } else {
                let value = self.delay[self.cursor];
                self.delay[self.cursor] = sample;
                self.cursor = (self.cursor + 1) % self.delay.len();
                value
            };
            dst[i * self.out_channels + self.out_channel] += value;
        }
    }
}
pub struct MultiGraph {
    spec: MultiSpec,
    nodes: Vec<Node>,
    routes: Vec<Route>,
    order: Vec<usize>,
    latency: usize,
}
impl MultiGraph {
    pub fn layout(&self) -> &Layout {
        &self.spec.layout
    }
    pub fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        ctx: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        for out in outputs.iter_mut() {
            out.fill(0.);
        }
        let frames =
            self.spec
                .layout
                .validate_buffers(inputs, outputs, self.spec.max_block_frames)?;
        if ctx.events.len() > 1024 {
            return Err(Error::InvalidEvents);
        }
        soniclaw_core::validate_events(ctx.events, frames)?;
        for &i in &self.order {
            for b in &mut self.nodes[i].inputs {
                b.fill(0.);
            }
            for e in self.routes.iter_mut().filter(|e| e.to == Some(i)) {
                if let Some(j) = e.from {
                    let (src, dst) = if j < i {
                        let (a, b) = self.nodes.split_at_mut(i);
                        (&a[j], &mut b[0])
                    } else {
                        let (a, b) = self.nodes.split_at_mut(j);
                        (&b[0], &mut a[i])
                    };
                    e.mix(&src.outputs[e.in_bus], &mut dst.inputs[e.out_bus], frames);
                } else {
                    e.mix(
                        inputs[e.in_bus],
                        &mut self.nodes[i].inputs[e.out_bus],
                        frames,
                    );
                }
            }
            let node = &mut self.nodes[i];
            let mut ins: [&[f32]; 8] = [&[]; 8];
            let mut outs: [&mut [f32]; 8] = std::array::from_fn(|_| &mut [][..]);
            for (j, b) in node.inputs.iter().enumerate() {
                ins[j] = &b[..frames * node.layout.inputs[j].channels];
            }
            for (j, b) in node.outputs.iter_mut().enumerate() {
                outs[j] = &mut b[..frames * node.layout.outputs[j].channels];
                outs[j].fill(0.);
            }
            if let Err(error) = node.processor.process(
                &ins[..node.inputs.len()],
                &mut outs[..node.layout.outputs.len()],
                ctx,
            ) {
                for output in outputs.iter_mut() {
                    output.fill(0.);
                }
                return Err(error);
            }
        }
        for e in self.routes.iter_mut().filter(|e| e.to.is_none()) {
            let source = match e.from {
                Some(i) => &self.nodes[i].outputs[e.in_bus],
                None => inputs[e.in_bus],
            };
            e.mix(source, outputs[e.out_bus], frames);
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        for e in &mut self.routes {
            e.delay.fill(0.);
            e.cursor = 0;
        }
        for n in &mut self.nodes {
            n.processor.reset();
            for b in n.inputs.iter_mut().chain(&mut n.outputs) {
                b.fill(0.);
            }
        }
    }
    pub fn latency_frames(&self) -> usize {
        self.latency
    }
}
impl MultiProcessor for MultiGraph {
    fn prepare(&mut self, s: MultiSpec) -> Result<(), Error> {
        s.validate()?;
        if s.layout != self.spec.layout
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        context: &ProcessContext<'_>,
    ) -> Result<(), Error> {
        MultiGraph::process(self, inputs, outputs, context)
    }
    fn reset(&mut self) {
        MultiGraph::reset(self);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::flex::{Bus, Role};
    fn layout() -> Layout {
        Layout {
            inputs: vec![Bus {
                id: "in".into(),
                name: "Input".into(),
                channels: 1,
                role: Role::Main,
            }],
            outputs: vec![Bus {
                id: "out".into(),
                name: "Output".into(),
                channels: 1,
                role: Role::Main,
            }],
        }
    }
    fn endpoint(id: Option<&str>) -> Endpoint {
        Endpoint {
            node: id.map(str::to_owned),
            bus: 0,
            channel: 0,
        }
    }
    struct Delay {
        line: [f32; 3],
        cursor: usize,
    }
    impl MultiProcessor for Delay {
        fn prepare(&mut self, _: MultiSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(
            &mut self,
            i: &[&[f32]],
            o: &mut [&mut [f32]],
            _: &ProcessContext<'_>,
        ) -> Result<(), Error> {
            for (s, d) in i[0].iter().zip(o[0].iter_mut()) {
                *d = self.line[self.cursor];
                self.line[self.cursor] = *s;
                self.cursor = (self.cursor + 1) % 3;
            }
            Ok(())
        }
        fn reset(&mut self) {
            self.line.fill(0.);
            self.cursor = 0;
        }
        fn latency_frames(&self) -> usize {
            3
        }
    }
    #[test]
    fn compensation_aligns_impulses_across_partitions_without_allocating() {
        let b = Blueprint {
            schema: 1,
            layout: layout(),
            nodes: vec![NodeDef {
                id: "delay".into(),
                layout: layout(),
            }],
            connections: vec![
                Connection {
                    from: endpoint(None),
                    to: endpoint(Some("delay")),
                    gain: 1.,
                },
                Connection {
                    from: endpoint(Some("delay")),
                    to: endpoint(None),
                    gain: 1.,
                },
                Connection {
                    from: endpoint(None),
                    to: endpoint(None),
                    gain: 1.,
                },
            ],
        };
        let mut g = b
            .build(48000., 8, |_| {
                Ok(Box::new(Delay {
                    line: [0.; 3],
                    cursor: 0,
                }))
            })
            .unwrap();
        assert_eq!(g.latency_frames(), 3);
        let mut source = [0.; 12];
        source[0] = 1.;
        let mut out = [0.; 12];
        let ctx = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        let m = allocation_counter::measure(|| {
            for (a, z) in [(0, 1), (1, 3), (3, 7), (7, 12)] {
                g.process(&[&source[a..z]], &mut [&mut out[a..z]], &ctx)
                    .unwrap();
            }
        });
        assert_eq!(m.count_total, 0);
        assert_eq!(out[3], 2.);
        assert_eq!(out.iter().sum::<f32>(), 2.);
        g.reset();
    }
    #[test]
    fn cycles_and_unknown_channels_are_rejected_before_factory() {
        let mut b = Blueprint {
            schema: 1,
            layout: layout(),
            nodes: vec![
                NodeDef {
                    id: "a".into(),
                    layout: layout(),
                },
                NodeDef {
                    id: "b".into(),
                    layout: layout(),
                },
            ],
            connections: vec![
                Connection {
                    from: endpoint(Some("a")),
                    to: endpoint(Some("b")),
                    gain: 1.,
                },
                Connection {
                    from: endpoint(Some("b")),
                    to: endpoint(Some("a")),
                    gain: 1.,
                },
            ],
        };
        assert_eq!(b.validate(), Err(Error::InvalidRouting));
        b.connections.pop();
        b.connections[0].from.channel = 1;
        assert_eq!(b.validate(), Err(Error::InvalidRouting));
    }
    #[test]
    fn independent_auxiliary_and_sidechain_channels_do_not_leak() {
        let bus = |id: &str, channels, role| Bus {
            id: id.into(),
            name: id.into(),
            channels,
            role,
        };
        let layout = Layout {
            inputs: vec![bus("main", 2, Role::Main), bus("key", 1, Role::Sidechain)],
            outputs: vec![bus("main", 2, Role::Main), bus("aux", 1, Role::Auxiliary)],
        };
        let e = |bus, channel| Endpoint {
            node: None,
            bus,
            channel,
        };
        let b = Blueprint {
            schema: 1,
            layout,
            nodes: vec![],
            connections: vec![
                Connection {
                    from: e(0, 1),
                    to: e(0, 0),
                    gain: 1.,
                },
                Connection {
                    from: e(0, 0),
                    to: e(0, 1),
                    gain: 0.5,
                },
                Connection {
                    from: e(1, 0),
                    to: e(1, 0),
                    gain: 1.,
                },
            ],
        };
        let mut g = b.build(48000., 8, |_| panic!("no nodes")).unwrap();
        let mut main = [0.; 4];
        let mut aux = [0.; 2];
        g.process(
            &[&[1., 2., 3., 4.], &[9., 8.]],
            &mut [&mut main, &mut aux],
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        )
        .unwrap();
        assert_eq!(main, [2., 0.5, 4., 1.5]);
        assert_eq!(aux, [9., 8.]);
        let error = g.process(
            &[&[1.], &[2.]],
            &mut [&mut main, &mut aux],
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
        assert_eq!(error, Err(Error::InvalidBuffer));
        assert_eq!(main, [0.; 4]);
        assert_eq!(aux, [0.; 2]);
    }
}
