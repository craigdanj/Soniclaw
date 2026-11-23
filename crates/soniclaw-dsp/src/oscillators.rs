//! High-rate oscillator rendering with source-specific latency alignment.
use crate::{
    advanced::wavetable::WavetableBank,
    oversampling::{self, Oversampler},
};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Sine,
    Saw,
    Pulse,
    Wavetable,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub frequency: f64,
    pub shape: Shape,
    pub pulse_width: f64,
    pub fm_hz: f64,
    pub fm_depth_hz: f64,
    pub pm_hz: f64,
    pub pm_depth_cycles: f64,
    pub sync_ratio: f64,
    pub warp: f64,
    pub morph: f32,
    pub gain: f32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            frequency: 220.,
            shape: Shape::Saw,
            pulse_width: 0.5,
            fm_hz: 0.,
            fm_depth_hz: 0.,
            pm_hz: 0.,
            pm_depth_cycles: 0.,
            sync_ratio: 1.,
            warp: 0.,
            morph: 0.,
            gain: 0.2,
        }
    }
}
impl Controls {
    pub fn validate(self) -> Result<(), Error> {
        let values = [
            (self.frequency, 1., 20000.),
            (self.pulse_width, 0.02, 0.98),
            (self.fm_hz, 0., 20000.),
            (self.fm_depth_hz, 0., 20000.),
            (self.pm_hz, 0., 20000.),
            (self.pm_depth_cycles, 0., 4.),
            (self.sync_ratio, 1., 8.),
            (self.warp, -0.95, 0.95),
            (self.morph as f64, 0., 1.),
            (self.gain as f64, 0., 1.),
        ];
        if values
            .iter()
            .any(|(v, a, b)| !v.is_finite() || v < a || v > b)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub controls: Controls,
    pub oversampling: Option<oversampling::Config>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            controls: Default::default(),
            oversampling: Some(oversampling::Config {
                schema: 1,
                factor: oversampling::Factor::Four,
                quality: oversampling::Quality::High,
            }),
        }
    }
}
#[derive(Default)]
struct Phase {
    master: f64,
    slave: f64,
    fm: f64,
    pm: f64,
}
impl Phase {
    fn tick(&mut self, c: Controls, rate: f64, bank: Option<&WavetableBank>) -> f32 {
        let tau = std::f64::consts::TAU;
        let frequency = (c.frequency * c.sync_ratio + c.fm_depth_hz * (tau * self.fm).sin())
            .clamp(-rate * 0.45, rate * 0.45);
        let phase = (self.slave + c.pm_depth_cycles * (tau * self.pm).sin()).rem_euclid(1.);
        let phase = (phase + c.warp * (tau * phase).sin() / tau).rem_euclid(1.);
        let x = match c.shape {
            Shape::Sine => (tau * phase).sin() as f32,
            Shape::Saw => (2. * phase - 1.) as f32,
            Shape::Pulse => {
                if phase < c.pulse_width {
                    1.
                } else {
                    -1.
                }
            }
            Shape::Wavetable => bank.map_or(0., |b| {
                b.read(
                    phase,
                    frequency.abs() + c.fm_depth_hz + c.pm_depth_cycles * c.pm_hz * tau,
                    rate,
                    c.morph,
                )
            }),
        };
        let step = c.frequency / rate;
        self.master += step;
        self.slave = (self.slave + frequency / rate).rem_euclid(1.);
        if self.master >= 1. {
            let remainder = (self.master - 1.) / step;
            self.master -= 1.;
            if c.sync_ratio > 1.000001 {
                self.slave = (frequency / rate * remainder).rem_euclid(1.);
            }
        }
        self.fm = (self.fm + c.fm_hz / rate).rem_euclid(1.);
        self.pm = (self.pm + c.pm_hz / rate).rem_euclid(1.);
        x * c.gain
    }
}
pub struct Oscillator {
    config: Config,
    bank: Option<Arc<WavetableBank>>,
    phase: Phase,
    converter: Option<Oversampler>,
    spec: Option<ProcessSpec>,
    padding: Vec<f32>,
    cursor: usize,
}
impl Oscillator {
    pub fn new(config: Config, bank: Option<Arc<WavetableBank>>) -> Result<Self, Error> {
        config.controls.validate()?;
        if config.schema != 1 || matches!(config.controls.shape, Shape::Wavetable) && bank.is_none()
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            config,
            bank,
            phase: Default::default(),
            converter: None,
            spec: None,
            padding: vec![],
            cursor: 0,
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn set_controls(&mut self, c: Controls) -> Result<(), Error> {
        c.validate()?;
        if matches!(c.shape, Shape::Wavetable) && self.bank.is_none()
            || self
                .spec
                .is_some_and(|s| c.frequency > s.sample_rate * 0.45)
        {
            return Err(Error::InvalidParameter);
        }
        self.config.controls = c;
        Ok(())
    }
}
impl AudioProcessor for Oscillator {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if !(8000.0..=192000.).contains(&s.sample_rate)
            || s.channels > 2
            || self.config.controls.frequency > s.sample_rate * 0.45
        {
            return Err(Error::InvalidSpec);
        }
        let converter = self
            .config
            .oversampling
            .map(|c| Oversampler::new(c, s))
            .transpose()?;
        let pad = converter.as_ref().map_or(0, |c| c.latency_frames() / 2);
        self.padding = vec![0.; pad * s.channels];
        self.converter = converter;
        self.spec = Some(s);
        self.reset();
        Ok(())
    }
    fn reset(&mut self) {
        self.phase = Phase::default();
        if let Some(c) = &mut self.converter {
            c.reset();
        }
        self.padding.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        self.converter.as_ref().map_or(0, |c| c.latency_frames())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.clear();
            return;
        }
        b.clear();
        let phase = &mut self.phase;
        let c = self.config.controls;
        let bank = self.bank.as_deref();
        if let Some(engine) = &mut self.converter {
            if engine
                .process_with(b, |b, hs| {
                    for frame in b.samples_mut().chunks_exact_mut(hs.channels) {
                        let x = phase.tick(c, hs.sample_rate, bank);
                        frame.fill(x);
                    }
                })
                .is_err()
            {
                b.clear();
            }
        } else {
            for frame in b.samples_mut().chunks_exact_mut(s.channels) {
                frame.fill(phase.tick(c, s.sample_rate, bank));
            }
        }
        if !self.padding.is_empty() {
            for x in b.samples_mut() {
                std::mem::swap(x, &mut self.padding[self.cursor]);
                self.cursor = (self.cursor + 1) % self.padding.len();
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn render(block: usize) -> Vec<f32> {
        let mut p = Oscillator::new(
            Config {
                controls: Controls {
                    fm_hz: 80.,
                    fm_depth_hz: 40.,
                    pm_hz: 10.,
                    pm_depth_cycles: 0.2,
                    sync_ratio: 2.3,
                    warp: 0.5,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
        )
        .unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 127,
            channels: 2,
        })
        .unwrap();
        let mut data = vec![0.; 4096];
        for b in data.chunks_mut(block * 2) {
            assert_eq!(
                allocation_counter::measure(|| p.process(
                    &mut AudioBuffer::new(b, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default()
                    }
                ))
                .count_total,
                0
            );
        }
        assert!(data.iter().all(|x| x.is_finite()));
        for f in data.as_chunks::<2>().0 {
            assert_eq!(f[0], f[1]);
        }
        data
    }
    #[test]
    fn modulation_partition_and_allocation() {
        assert_eq!(render(127), render(1));
    }
    #[test]
    fn generator_padding_and_reset() {
        let mut p = Oscillator::new(Default::default(), None).unwrap();
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        })
        .unwrap();
        assert_eq!(p.latency_frames(), 96);
        let mut data = [0.; 128];
        p.process(
            &mut AudioBuffer::new(&mut data, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert!(data[..48].iter().all(|x| *x == 0.));
        p.reset();
        let mut reset = [0.; 128];
        p.process(
            &mut AudioBuffer::new(&mut reset, 1).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert_eq!(data, reset);
        assert!(p
            .set_controls(Controls {
                pulse_width: f64::NAN,
                ..Default::default()
            })
            .is_err());
    }
}
