//! Adapt an explicitly negotiated native bus processor to Studio stereo tracks and return banks.
use super::*;
use soniclaw_bus::{flex::Layout, multigraph::MultiProcessor};
use std::sync::Arc;
use studio::native_buses::{Frames, Request};
pub enum Driver {
    Live(Box<soniclaw_vst3::buses::BusProcessor>),
    Offline(Box<soniclaw_vst3::bus_offline::Processor>),
}
pub struct Adapter {
    driver: Driver,
    layout: Layout,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
    sidechains: BTreeMap<usize, Arc<Frames>>,
    banks: Vec<Arc<Frames>>,
    spec: ProcessSpec,
    latency: usize,
}
impl Adapter {
    pub fn new(
        driver: Driver,
        request: Request,
        registry: &studio::native_buses::Registry,
        id: &str,
        spec: ProcessSpec,
    ) -> Result<Self, HostError> {
        request.validate()?;
        let latency = match &driver {
            Driver::Live(p) => p.latency_frames(),
            Driver::Offline(p) => p.latency_frames(),
        };
        let banks = registry
            .outputs
            .get(id)
            .ok_or("native bus graph not prepared")?
            .clone();
        let mut sidechains = BTreeMap::new();
        for (i, source) in request.sidechains {
            let bank = registry
                .tracks
                .get(&source)
                .ok_or("missing native sidechain source")?
                .clone();
            if bank.latency.load(Ordering::Acquire) != 0 {
                return Err(
                    "native bus sidechain source currently requires zero track latency".into(),
                );
            }
            sidechains.insert(i, bank);
        }
        for b in &banks {
            b.latency.store(latency, Ordering::Release);
        }
        let layout = request.layout;
        Ok(Self {
            inputs: layout
                .inputs
                .iter()
                .map(|b| vec![0.; b.channels * spec.max_block_frames])
                .collect(),
            outputs: layout
                .outputs
                .iter()
                .map(|b| vec![0.; b.channels * spec.max_block_frames])
                .collect(),
            driver,
            layout,
            sidechains,
            banks,
            spec,
            latency,
        })
    }
    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[soniclaw_vst3::musical::ParameterPoint],
    ) -> Result<(), Error> {
        let frames = b.frames();
        if frames > self.spec.max_block_frames || b.channels() != 2 {
            return Err(Error::InvalidBuffer);
        }
        for (i, (buffer, shape)) in self.inputs.iter_mut().zip(&self.layout.inputs).enumerate() {
            for (f, frame) in buffer[..frames * shape.channels]
                .chunks_mut(shape.channels)
                .enumerate()
            {
                for (c, v) in frame.iter_mut().enumerate() {
                    *v = if i == 0 {
                        if shape.channels == 1 {
                            (b.samples()[f * 2] + b.samples()[f * 2 + 1]) * 0.5
                        } else {
                            b.samples()[f * 2 + c]
                        }
                    } else {
                        self.sidechains.get(&i).map_or(0., |s| {
                            if shape.channels == 1 {
                                (s.sample(ctx.transport.sample_position + f as u64, 0)
                                    + s.sample(ctx.transport.sample_position + f as u64, 1))
                                    * 0.5
                            } else {
                                s.sample(ctx.transport.sample_position + f as u64, c)
                            }
                        })
                    };
                }
            }
        }
        let mut inputs: [&[f32]; 8] = [&[]; 8];
        for ((out, data), shape) in inputs.iter_mut().zip(&self.inputs).zip(&self.layout.inputs) {
            *out = &data[..frames * shape.channels];
        }
        let mut outputs: [&mut [f32]; 8] = std::array::from_fn(|_| &mut [] as &mut [f32]);
        for ((out, data), shape) in outputs
            .iter_mut()
            .zip(&mut self.outputs)
            .zip(&self.layout.outputs)
        {
            *out = &mut data[..frames * shape.channels];
        }
        let result = match &mut self.driver {
            Driver::Live(p) => p.process_automated(
                &inputs[..self.inputs.len()],
                &mut outputs[..self.layout.outputs.len()],
                ctx,
                points,
            ),
            Driver::Offline(p) => p.process_automated(
                &inputs[..self.inputs.len()],
                &mut outputs[..self.layout.outputs.len()],
                ctx,
                points,
            ),
        };
        if result.is_err() {
            for output in &mut outputs[..self.layout.outputs.len()] {
                output.fill(0.);
            }
        }
        for ((bank, data), shape) in self
            .banks
            .iter()
            .zip(&self.outputs)
            .zip(&self.layout.outputs)
        {
            bank.write(
                ctx.transport.sample_position,
                &data[..frames * shape.channels],
            )?;
        }
        let main = &self.outputs[0];
        let count = self.layout.outputs[0].channels;
        for (f, frame) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            frame[0] = main[f * count];
            frame[1] = main[f * count + (count - 1).min(1)];
        }
        result
    }
}
impl AudioProcessor for Adapter {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2
            || s.sample_rate != self.spec.sample_rate
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        for b in &self.banks {
            b.invalidate();
        }
        match &mut self.driver {
            Driver::Live(p) => p.reset(),
            Driver::Offline(p) => p.reset(),
        }
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let _ = self.process_automated(b, ctx, &[]);
    }
}
