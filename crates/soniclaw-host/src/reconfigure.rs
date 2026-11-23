//! Stopped-time topology plans and explicit prepared channel routing. No native ownership is implicit.
use super::*;
use soniclaw_bus::flex::Layout;
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topology {
    pub layout: Layout,
    pub latency: usize,
    pub sample_rate: u32,
    pub max_frames: usize,
}
impl Topology {
    pub fn validate(&self) -> Result<(), Error> {
        self.layout.validate()?;
        if !(8000..=192000).contains(&self.sample_rate)
            || !(1..=1024).contains(&self.max_frames)
            || self.latency > 262144
            || self.width() > 8
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    pub fn width(&self) -> usize {
        self.layout
            .inputs
            .iter()
            .map(|b| b.channels)
            .sum::<usize>()
            .max(self.layout.outputs.iter().map(|b| b.channels).sum())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub bus: String,
    pub channel: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub port: Channel,
    pub external: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    pub channels: usize,
    pub inputs: Vec<Connection>,
    pub silent_inputs: Vec<Channel>,
    pub outputs: Vec<Connection>,
    pub discarded_outputs: Vec<Channel>,
}
fn slots(buses: &[soniclaw_bus::flex::Bus]) -> BTreeSet<Channel> {
    buses
        .iter()
        .flat_map(|b| {
            (0..b.channels).map(|channel| Channel {
                bus: b.id.clone(),
                channel,
            })
        })
        .collect()
}
impl Routing {
    pub fn validate(&self, layout: &Layout) -> Result<(), Error> {
        layout.validate()?;
        if !(1..=8).contains(&self.channels)
            || self.inputs.len() + self.silent_inputs.len() > 64
            || self.outputs.len() + self.discarded_outputs.len() > 64
        {
            return Err(Error::InvalidRouting);
        }
        for (connections, unused, buses) in [
            (&self.inputs, &self.silent_inputs, &layout.inputs),
            (&self.outputs, &self.discarded_outputs, &layout.outputs),
        ] {
            let expected = slots(buses);
            let mut seen = BTreeSet::new();
            for c in connections {
                if c.external >= self.channels
                    || !expected.contains(&c.port)
                    || !seen.insert(c.port.clone())
                {
                    return Err(Error::InvalidRouting);
                }
            }
            for c in unused {
                if !expected.contains(c) || !seen.insert(c.clone()) {
                    return Err(Error::InvalidRouting);
                }
            }
            if seen != expected {
                return Err(Error::InvalidRouting);
            }
        }
        // No implicit output summing. Unmapped external outputs are explicit silence in this adapter.
        let mut seen = BTreeSet::new();
        if self.outputs.iter().any(|c| !seen.insert(c.external)) {
            return Err(Error::InvalidRouting);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: u32,
    pub previous: Topology,
    pub next: Topology,
    pub routing: Routing,
    pub pipeline: usize,
    pub latency_budget: usize,
}
impl Plan {
    pub fn validate(&self) -> Result<(), Error> {
        self.previous.validate()?;
        self.next.validate()?;
        self.routing.validate(&self.next.layout)?;
        if self.schema != 1
            || self.pipeline < self.next.max_frames * 2
            || self.pipeline > 262144
            || self.latency_budget > 262144
            || self
                .pipeline
                .checked_add(self.next.latency)
                .is_none_or(|v| v > self.latency_budget)
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|_| Error::InvalidParameter)
    }
    pub fn from_json(s: &str) -> Result<Self, Error> {
        if s.len() > 64 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        let p: Self = serde_json::from_str(s).map_err(|_| Error::InvalidParameter)?;
        p.validate()?;
        Ok(p)
    }
}
fn index(buses: &[soniclaw_bus::flex::Bus], port: &Channel) -> usize {
    let mut offset = 0;
    for b in buses {
        if b.id == port.bus {
            return offset + port.channel;
        }
        offset += b.channels;
    }
    unreachable!("validated channel")
}
/// Preallocated routing around a newly prepared isolated processor. Use in a rebuilt graph to recalculate PDC.
pub struct RoutedProcessor {
    inner: isolation::Processor,
    flat: Vec<f32>,
    inputs: Vec<(usize, usize)>,
    outputs: Vec<(usize, usize)>,
    width: usize,
    spec: ProcessSpec,
}
impl RoutedProcessor {
    pub(crate) fn new(inner: isolation::Processor, plan: &Plan) -> Result<Self, Error> {
        plan.validate()?;
        let width = plan.next.width();
        Ok(Self {
            inner,
            flat: vec![0.; plan.next.max_frames * width],
            inputs: plan
                .routing
                .inputs
                .iter()
                .map(|c| (c.external, index(&plan.next.layout.inputs, &c.port)))
                .collect(),
            outputs: plan
                .routing
                .outputs
                .iter()
                .map(|c| (index(&plan.next.layout.outputs, &c.port), c.external))
                .collect(),
            width,
            spec: ProcessSpec {
                sample_rate: plan.next.sample_rate as f64,
                max_block_frames: plan.next.max_frames,
                channels: plan.routing.channels,
            },
        })
    }
    pub fn spec(&self) -> ProcessSpec {
        self.spec
    }
    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        parameters: &[ParameterEvent],
    ) -> Result<(), Error> {
        if b.channels() != self.spec.channels || b.frames() > self.spec.max_block_frames {
            b.clear();
            return Err(Error::InvalidBuffer);
        }
        let frames = b.frames();
        let flat = &mut self.flat[..frames * self.width];
        flat.fill(0.);
        for f in 0..frames {
            for &(external, native) in &self.inputs {
                flat[f * self.width + native] = b.samples()[f * self.spec.channels + external];
            }
        }
        let result =
            self.inner
                .process_automated(&mut AudioBuffer::new(flat, self.width)?, ctx, parameters);
        b.clear();
        result?;
        for f in 0..frames {
            for &(native, external) in &self.outputs {
                b.samples_mut()[f * self.spec.channels + external] = flat[f * self.width + native];
            }
        }
        Ok(())
    }
}
impl AudioProcessor for RoutedProcessor {
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
        let _ = self.process_automated(b, ctx, &[]);
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_bus::flex::{Bus, Role};
    #[test]
    fn topology_routes_budget_and_serialized_plans_reject_implicit_drops() {
        let bus = |id: &str, channels| Bus {
            id: id.into(),
            name: id.into(),
            channels,
            role: Role::Main,
        };
        let previous = Topology {
            layout: Layout {
                inputs: vec![bus("main", 2)],
                outputs: vec![bus("main", 2)],
            },
            latency: 0,
            sample_rate: 48000,
            max_frames: 64,
        };
        let mut next = previous.clone();
        next.layout.inputs.push(bus("key", 1));
        next.latency = 16;
        let inputs = vec![
            Connection {
                port: Channel {
                    bus: "main".into(),
                    channel: 0,
                },
                external: 0,
            },
            Connection {
                port: Channel {
                    bus: "main".into(),
                    channel: 1,
                },
                external: 1,
            },
        ];
        let routing = Routing {
            channels: 2,
            inputs: inputs.clone(),
            silent_inputs: vec![Channel {
                bus: "key".into(),
                channel: 0,
            }],
            outputs: inputs,
            discarded_outputs: vec![],
        };
        let p = Plan {
            schema: 1,
            previous,
            next,
            routing,
            pipeline: 256,
            latency_budget: 272,
        };
        assert!(p.validate().is_ok());
        assert!(Plan::from_json(&p.to_json().unwrap()).is_ok());
        let mut bad = p.clone();
        bad.routing.silent_inputs.clear();
        assert!(bad.validate().is_err());
        bad = p.clone();
        bad.latency_budget = 271;
        assert!(bad.validate().is_err());
        bad = p.clone();
        bad.routing.outputs[1].external = 0;
        assert!(bad.validate().is_err());
        bad = p;
        bad.next.max_frames = 1024;
        assert!(bad.validate().is_err());
    }
}
