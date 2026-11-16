//! Explicit interleaved main-input, main-output and sidechain buses. Layouts are
//! negotiated while stopped. Sidechains are never silently mixed into main audio.
use soniclaw_core::*;
use soniclaw_engine::Engine;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusLayout {
    pub input_channels: usize,
    pub output_channels: usize,
    pub sidechain_channels: usize,
}
impl BusLayout {
    pub fn validate(self) -> Result<Self, Error> {
        if self.input_channels > 64
            || !(1..=64).contains(&self.output_channels)
            || self.sidechain_channels > 64
        {
            return Err(Error::InvalidSpec);
        }
        Ok(self)
    }
}
#[derive(Clone, Copy)]
pub struct BusSpec {
    pub sample_rate: f64,
    pub max_block_frames: usize,
    pub layout: BusLayout,
}
impl BusSpec {
    pub fn validate(self) -> Result<Self, Error> {
        self.layout.validate()?;
        ProcessSpec {
            sample_rate: self.sample_rate,
            max_block_frames: self.max_block_frames,
            channels: self.layout.output_channels,
        }
        .validate()?;
        Ok(self)
    }
}
pub struct BusBuffers<'a> {
    pub input: &'a [f32],
    pub output: &'a mut [f32],
    pub sidechain: &'a [f32],
    pub frames: usize,
}
pub trait BusProcessor: Send {
    fn prepare(&mut self, spec: BusSpec) -> Result<(), Error>;
    fn process(&mut self, buses: &mut BusBuffers<'_>, context: &ProcessContext<'_>);
    fn reset(&mut self);
    fn latency_frames(&self) -> usize {
        0
    }
}
pub struct BusEngine {
    processor: Box<dyn BusProcessor>,
    spec: Option<BusSpec>,
    transport: Transport,
    suspended: bool,
}
impl BusEngine {
    pub fn new(processor: impl BusProcessor + 'static) -> Self {
        Self {
            processor: Box::new(processor),
            spec: None,
            transport: Transport::default(),
            suspended: false,
        }
    }
    pub fn prepare(&mut self, spec: BusSpec) -> Result<(), Error> {
        self.spec = None;
        spec.validate()?;
        self.processor.prepare(spec)?;
        self.spec = Some(spec);
        self.transport.sample_position = 0;
        Ok(())
    }
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.processor.reset();
    }
    pub fn resume(&mut self) {
        self.suspended = false;
    }
    pub fn latency_frames(&self) -> usize {
        self.processor.latency_frames()
    }
    pub fn reset(&mut self) {
        self.processor.reset();
        self.transport.sample_position = 0;
    }
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        sidechain: &[f32],
        events: &[Event],
    ) -> Result<(), Error> {
        let check = (|| {
            let spec = self.spec.ok_or(Error::NotPrepared)?;
            if !output.len().is_multiple_of(spec.layout.output_channels) {
                return Err(Error::InvalidBuffer);
            }
            let frames = output.len() / spec.layout.output_channels;
            if frames > spec.max_block_frames {
                return Err(Error::BlockTooLarge);
            }
            if input.len() != frames * spec.layout.input_channels
                || sidechain.len() != frames * spec.layout.sidechain_channels
            {
                return Err(Error::InvalidBuffer);
            }
            validate_events(events, frames)?;
            Ok((spec, frames))
        })();
        let (_, frames) = match check {
            Ok(v) => v,
            Err(error) => {
                output.fill(0.0);
                return Err(error);
            }
        };
        if self.suspended {
            output.fill(0.0);
            return Ok(());
        }
        self.processor.process(
            &mut BusBuffers {
                input,
                output,
                sidechain,
                frames,
            },
            &ProcessContext {
                events,
                transport: self.transport,
            },
        );
        self.transport.sample_position =
            self.transport.sample_position.saturating_add(frames as u64);
        Ok(())
    }
}
/// Adapts an in-place engine to independent input/output buses, including mono
/// conversion. A sidechain requires a BusProcessor that actually consumes it.
pub struct InPlaceBus {
    pub engine: Engine,
    layout: BusLayout,
}
impl InPlaceBus {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            layout: BusLayout {
                input_channels: 0,
                output_channels: 2,
                sidechain_channels: 0,
            },
        }
    }
}
impl BusProcessor for InPlaceBus {
    fn prepare(&mut self, spec: BusSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.layout.sidechain_channels != 0 {
            return Err(Error::InvalidRouting);
        }
        self.layout = spec.layout;
        self.engine.prepare(ProcessSpec {
            sample_rate: spec.sample_rate,
            max_block_frames: spec.max_block_frames,
            channels: spec.layout.output_channels,
        })
    }
    fn process(&mut self, b: &mut BusBuffers<'_>, ctx: &ProcessContext<'_>) {
        b.output.fill(0.0);
        if self.layout.input_channels > 0 {
            for (input, output) in b
                .input
                .chunks_exact(self.layout.input_channels)
                .zip(b.output.chunks_exact_mut(self.layout.output_channels))
            {
                if output.len() == 1 {
                    output[0] = input.iter().sum::<f32>() / input.len() as f32;
                } else if input.len() == 1 {
                    output.fill(input[0]);
                } else {
                    for (o, i) in output.iter_mut().zip(input) {
                        *o = *i;
                    }
                }
            }
        }
        let _ = self.engine.set_transport(ctx.transport);
        let _ = self.engine.process(b.output, ctx.events);
    }
    fn reset(&mut self) {
        self.engine.reset();
    }
    fn latency_frames(&self) -> usize {
        self.engine.latency_frames()
    }
}
/// A reusable sidechain compressor, linked across output channels.
pub struct SidechainDucker {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    layout: BusLayout,
    envelope: f32,
    attack: f32,
    release: f32,
}
impl Default for SidechainDucker {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 6.0,
            attack_ms: 5.0,
            release_ms: 100.0,
            layout: BusLayout {
                input_channels: 2,
                output_channels: 2,
                sidechain_channels: 1,
            },
            envelope: 0.0,
            attack: 0.0,
            release: 0.0,
        }
    }
}
impl BusProcessor for SidechainDucker {
    fn prepare(&mut self, spec: BusSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.layout.input_channels != spec.layout.output_channels
            || spec.layout.sidechain_channels == 0
            || !self.threshold_db.is_finite()
            || !self.ratio.is_finite()
            || self.ratio < 1.0
            || !self.attack_ms.is_finite()
            || self.attack_ms <= 0.0
            || !self.release_ms.is_finite()
            || self.release_ms <= 0.0
        {
            return Err(Error::InvalidSpec);
        }
        self.layout = spec.layout;
        self.attack = (-1.0 / (self.attack_ms * 0.001 * spec.sample_rate as f32)).exp();
        self.release = (-1.0 / (self.release_ms * 0.001 * spec.sample_rate as f32)).exp();
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut BusBuffers<'_>, _: &ProcessContext<'_>) {
        for ((input, output), side) in b
            .input
            .chunks_exact(self.layout.input_channels)
            .zip(b.output.chunks_exact_mut(self.layout.output_channels))
            .zip(b.sidechain.chunks_exact(self.layout.sidechain_channels))
        {
            let peak = side.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
            let coefficient = if peak > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = coefficient * self.envelope + (1.0 - coefficient) * peak;
            let db = 20.0 * self.envelope.max(1e-12).log10();
            let above = (db - self.threshold_db).max(0.0);
            let gain = 10.0_f32.powf(-above * (1.0 - 1.0 / self.ratio) / 20.0);
            for (o, i) in output.iter_mut().zip(input) {
                *o = *i * gain;
            }
        }
    }
    fn reset(&mut self) {
        self.envelope = 0.0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mono_to_stereo_and_suspend() {
        let mut e = BusEngine::new(InPlaceBus::new(Engine::default()));
        e.prepare(BusSpec {
            sample_rate: 48000.0,
            max_block_frames: 4,
            layout: BusLayout {
                input_channels: 1,
                output_channels: 2,
                sidechain_channels: 0,
            },
        })
        .unwrap();
        let mut output = [0.0; 8];
        e.process(&[0.1, 0.2, 0.3, 0.4], &mut output, &[], &[])
            .unwrap();
        assert_eq!(output, [0.1, 0.1, 0.2, 0.2, 0.3, 0.3, 0.4, 0.4]);
        e.suspend();
        e.process(&[1.0; 4], &mut output, &[], &[]).unwrap();
        assert_eq!(output, [0.0; 8]);
        e.resume();
    }
    #[test]
    fn sidechain_reduces_main_without_leaking_into_output() {
        let mut e = BusEngine::new(SidechainDucker::default());
        e.prepare(BusSpec {
            sample_rate: 48000.0,
            max_block_frames: 1024,
            layout: BusLayout {
                input_channels: 2,
                output_channels: 2,
                sidechain_channels: 1,
            },
        })
        .unwrap();
        let mut output = [0.0; 2048];
        e.process(&[1.0; 2048], &mut output, &[0.0; 1024], &[])
            .unwrap();
        assert_eq!(output, [1.0; 2048]);
        e.process(&[1.0; 2048], &mut output, &[1.0; 1024], &[])
            .unwrap();
        assert!(output[2047] < 0.3);
        e.process(&[0.0; 2048], &mut output, &[1.0; 1024], &[])
            .unwrap();
        assert_eq!(output, [0.0; 2048]);
    }
}
