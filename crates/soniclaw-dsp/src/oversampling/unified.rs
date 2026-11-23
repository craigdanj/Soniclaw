//! A common prepared entry point for FIR and allpass IIR oversampling.
use super::{Config, Factor, Oversampler, Quality};
use crate::iir_oversampling::{AllpassDelay, IirOversampler};
use soniclaw_core::{AudioBuffer, Error, ProcessSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ratio {
    One,
    #[default]
    Two,
    Four,
    Eight,
    Sixteen,
}
impl Ratio {
    pub const fn multiplier(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    Fir,
    Iir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OversamplingConfig {
    pub ratio: Ratio,
    pub mode: FilterMode,
    pub quality: Quality,
    /// Add fractional allpass delay to align the low-frequency IIR reference to a whole frame.
    pub integer_latency: bool,
}
impl Default for OversamplingConfig {
    fn default() -> Self {
        Self {
            ratio: Ratio::Two,
            mode: FilterMode::Fir,
            quality: Quality::Balanced,
            integer_latency: false,
        }
    }
}

/// Latency of resampling alone, in base-rate frames.
#[derive(Debug, Clone, Copy)]
pub struct OversamplingLatency {
    /// Exact constant group delay for FIR, low-frequency reference for IIR.
    pub dc_frames: f64,
    pub integer_reference_frames: Option<usize>,
    pub linear_phase: bool,
}

pub struct UnifiedOversampler {
    spec: ProcessSpec,
    high: ProcessSpec,
    engine: Engine,
    latency: OversamplingLatency,
    compensation: Vec<AllpassDelay>,
    added_delay: f64,
}
enum Engine {
    Bypass,
    Fir(Oversampler),
    Iir(IirOversampler),
}

impl UnifiedOversampler {
    pub fn new(config: OversamplingConfig, spec: ProcessSpec) -> Result<Self, Error> {
        let spec = spec.validate()?;
        let multiplier = config.ratio.multiplier();
        let high = ProcessSpec {
            sample_rate: spec.sample_rate * multiplier as f64,
            max_block_frames: spec
                .max_block_frames
                .checked_mul(multiplier)
                .ok_or(Error::InvalidSpec)?,
            channels: spec.channels,
        }
        .validate()?;
        if high.max_block_frames * high.channels > 4_194_304 {
            return Err(Error::InvalidSpec);
        }
        let count = multiplier.trailing_zeros() as usize;
        let engine = match (count, config.mode) {
            (0, _) => Engine::Bypass,
            (_, FilterMode::Fir) => Engine::Fir(Oversampler::with_stages(
                Config {
                    schema: 1,
                    factor: Factor::Two,
                    quality: config.quality,
                },
                spec,
                count,
            )?),
            (_, FilterMode::Iir) => Engine::Iir(IirOversampler::with_stages(
                spec,
                count,
                config.quality,
                4_194_304,
            )?),
        };
        let (raw, linear_phase) = match &engine {
            Engine::Bypass => (0., true),
            Engine::Fir(engine) => (engine.latency_frames() as f64, true),
            Engine::Iir(engine) => (engine.dc_delay_samples(), false),
        };
        let added_delay = if linear_phase || !config.integer_latency {
            0.
        } else {
            raw.ceil() - raw
        };
        let latency = OversamplingLatency {
            dc_frames: raw + added_delay,
            integer_reference_frames: (linear_phase || config.integer_latency)
                .then(|| raw.ceil() as usize),
            linear_phase,
        };
        Ok(Self {
            spec,
            high,
            engine,
            latency,
            added_delay,
            compensation: vec![
                AllpassDelay::default();
                if added_delay == 0. { 0 } else { spec.channels }
            ],
        })
    }

    pub fn high_rate_spec(&self) -> ProcessSpec {
        self.high
    }
    pub fn latency(&self) -> OversamplingLatency {
        self.latency
    }
    pub fn reset(&mut self) {
        match &mut self.engine {
            Engine::Bypass => {}
            Engine::Fir(engine) => engine.reset(),
            Engine::Iir(engine) => engine.reset(),
        }
        self.compensation.fill(AllpassDelay::default());
    }

    /// The closure receives one interleaved block, at the configured high rate.
    /// Include any latency introduced by the closure separately in a host's report.
    pub fn process_with(
        &mut self,
        block: &mut AudioBuffer<'_>,
        render: impl FnOnce(&mut AudioBuffer<'_>, ProcessSpec),
    ) -> Result<(), Error> {
        if block.channels() != self.spec.channels
            || block.frames() > self.spec.max_block_frames
            || block.samples().iter().any(|x| !x.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        if block.frames() == 0 {
            return Ok(());
        }
        let processed = match &mut self.engine {
            Engine::Bypass => {
                render(block, self.high);
                Ok(())
            }
            Engine::Fir(engine) => engine.process_with(block, render),
            Engine::Iir(engine) => engine.process_with(block, render),
        };
        if processed.is_err() || block.samples().iter().any(|x| !x.is_finite()) {
            block.clear();
            self.reset();
            return Err(Error::InvalidBuffer);
        }
        if !self.compensation.is_empty() {
            for frame in block.samples_mut().chunks_exact_mut(self.spec.channels) {
                for (sample, delay) in frame.iter_mut().zip(&mut self.compensation) {
                    *sample = delay.tick(f64::from(*sample), self.added_delay) as f32;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 48_000.,
            max_block_frames: 97,
            channels: 2,
        }
    }

    fn impulse(conf: OversamplingConfig, block: usize) -> Vec<f32> {
        let mut engine = UnifiedOversampler::new(conf, spec()).unwrap();
        let mut data = vec![0.; 8192];
        data[0] = 1.;
        let allocated = allocation_counter::measure(|| {
            for part in data.chunks_mut(block * 2) {
                engine
                    .process_with(&mut AudioBuffer::new(part, 2).unwrap(), |buffer, spec| {
                        assert_eq!(buffer.channels(), 2);
                        assert_eq!(spec.sample_rate, 48_000. * conf.ratio.multiplier() as f64);
                    })
                    .unwrap();
            }
        });
        assert_eq!(allocated.count_total, 0);
        data
    }

    #[test]
    fn delay_moment_impulse_dc_and_partition_in_all_modes() {
        for ratio in [
            Ratio::One,
            Ratio::Two,
            Ratio::Four,
            Ratio::Eight,
            Ratio::Sixteen,
        ] {
            for mode in [FilterMode::Fir, FilterMode::Iir] {
                for quality in [Quality::Balanced, Quality::High] {
                    for integer_latency in [false, true] {
                        let config = OversamplingConfig {
                            ratio,
                            mode,
                            quality,
                            integer_latency,
                        };
                        let a = impulse(config, 97);
                        assert_eq!(a, impulse(config, 23));
                        assert!(a.iter().skip(1).step_by(2).all(|x| *x == 0.));
                        let engine = UnifiedOversampler::new(config, spec()).unwrap();
                        let dc: f64 = a.iter().step_by(2).map(|&x| f64::from(x)).sum();
                        let moment: f64 = a
                            .iter()
                            .step_by(2)
                            .enumerate()
                            .map(|(i, &x)| i as f64 * f64::from(x))
                            .sum();
                        assert!((dc - 1.).abs() < 2e-6, "{config:?} {dc}");
                        assert!(
                            (moment / dc - engine.latency().dc_frames).abs() < 6e-4,
                            "{config:?} {moment}"
                        );
                        if engine.latency().linear_phase {
                            let peak = a
                                .iter()
                                .step_by(2)
                                .enumerate()
                                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                                .unwrap()
                                .0;
                            assert_eq!(Some(peak), engine.latency().integer_reference_frames);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn stage_rejects_high_rate_content_above_base_nyquist() {
        for mode in [FilterMode::Fir, FilterMode::Iir] {
            let config = OversamplingConfig {
                mode,
                quality: Quality::High,
                ..Default::default()
            };
            let mut engine = UnifiedOversampler::new(config, spec()).unwrap();
            let mut phase = 0;
            let mut data = [0.; 194];
            for _ in 0..22 {
                engine.process_with(&mut AudioBuffer::new(&mut data, 2).unwrap(), |b, _| {
                    for frame in b.samples_mut().as_chunks_mut::<2>().0 {
                        frame.fill((f64::from(phase) * std::f64::consts::TAU * 0.4).sin() as f32);
                        phase += 1;
                    }
                }).unwrap();
            }
            let rms = (data.iter().map(|x| x * x).sum::<f32>() / data.len() as f32).sqrt();
            assert!(rms < 0.0001, "{mode:?} {rms}");
        }
    }
}
