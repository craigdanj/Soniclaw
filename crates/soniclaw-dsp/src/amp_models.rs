//! Bounded implicit diode/RC model, tone filtering, supply sag and optional mono cabinet IR.
use crate::{
    oversampling::{self, Oversampled},
    Convolution,
};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub drive_db: f32,
    pub bias: f32,
    pub asymmetry: f32,
    pub tone: f32,
    pub sag: f32,
    pub output_db: f32,
    pub mix: f32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            drive_db: 18.,
            bias: 0.,
            asymmetry: 1.,
            tone: 0.5,
            sag: 0.3,
            output_db: -6.,
            mix: 1.,
        }
    }
}
impl Controls {
    fn validate(self) -> Result<(), Error> {
        if [
            (self.drive_db, 0., 36.),
            (self.bias, -0.2, 0.2),
            (self.asymmetry, 0.5, 2.),
            (self.tone, 0.05, 0.95),
            (self.sag, 0., 1.),
            (self.output_db, -36., 6.),
            (self.mix, 0., 1.),
        ]
        .iter()
        .any(|(x, a, b)| !x.is_finite() || x < a || x > b)
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
    pub oversampling: oversampling::Config,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 1,
            controls: Default::default(),
            oversampling: oversampling::Config {
                factor: oversampling::Factor::Four,
                schema: 1,
                quality: oversampling::Quality::High,
            },
        }
    }
}
#[derive(Clone, Copy, Default)]
struct State {
    capacitor: f64,
    dc: f64,
    low: f64,
    high: f64,
    envelope: f64,
}
struct Circuit {
    controls: Controls,
    states: Vec<State>,
    spec: Option<ProcessSpec>,
}
fn diode(y: f64, asymmetry: f64) -> (f64, f64) {
    let a = (y / 0.12).clamp(-20., 20.).exp();
    let b = (-y / (0.12 * asymmetry)).clamp(-20., 20.).exp();
    (0.002 * (a - b), 0.002 * (a / 0.12 + b / (0.12 * asymmetry)))
}
/// Safeguarded Newton iterations for y-state+g*(y-input+diode(y))=0.
fn solve(state: f64, input: f64, g: f64, asymmetry: f64) -> f64 {
    let (mut lo, mut hi) = (-20., 20.);
    let mut y = state.clamp(lo, hi);
    for _ in 0..16 {
        let (d, derivative) = diode(y, asymmetry);
        let residual = y - state + g * (y - input + d);
        if residual.abs() < 1e-12 {
            return y;
        }
        if residual > 0. {
            hi = y;
        } else {
            lo = y;
        }
        let next = y - (residual / (1. + g * (1. + derivative))).clamp(-0.25, 0.25);
        y = if next > lo && next < hi {
            next
        } else {
            (lo + hi) * 0.5
        };
    }
    y
}
impl AudioProcessor for Circuit {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        self.states = vec![State::default(); s.channels];
        self.spec = Some(s);
        Ok(())
    }
    fn reset(&mut self) {
        self.states.fill(State::default());
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.samples_mut().fill(0.);
            return;
        };
        if b.channels() != s.channels || b.frames() > s.max_block_frames {
            b.samples_mut().fill(0.);
            return;
        }
        let c = self.controls;
        let sr = s.sample_rate;
        let g = std::f64::consts::TAU * 6000. / sr;
        let alpha = |hz: f64| 1. - (-std::f64::consts::TAU * hz / sr).exp();
        let dc = alpha(20.);
        let bass = alpha(250.);
        let tone = alpha(500. + 10000. * c.tone as f64);
        let sag = 1. - (-1. / (sr * 0.05)).exp();
        let drive = 10f64.powf(c.drive_db as f64 / 20.);
        let output = 10f64.powf(c.output_db as f64 / 20.);
        for frame in b.samples_mut().chunks_exact_mut(s.channels) {
            for (x, state) in frame.iter_mut().zip(&mut self.states) {
                let dry = if x.is_finite() {
                    (*x as f64).clamp(-4., 4.)
                } else {
                    0.
                };
                state.envelope += sag * (dry.abs() - state.envelope);
                let input = (dry * drive / (1. + 3. * c.sag as f64 * state.envelope)
                    + c.bias as f64)
                    .clamp(-20., 20.);
                state.capacitor = solve(state.capacitor, input, g, c.asymmetry as f64);
                state.dc += dc * (state.capacitor - state.dc);
                let ac = state.capacitor - state.dc;
                state.low += bass * (ac - state.low);
                state.high += tone * (ac - state.high);
                let wet = (state.low * (1.4 - 0.8 * c.tone as f64)
                    + (state.high - state.low) * (0.4 + 0.9 * c.tone as f64))
                    * output;
                *x = (dry + (wet - dry) * c.mix as f64) as f32;
            }
        }
    }
}
pub struct Amp {
    config: Config,
    inner: Oversampled<Circuit>,
    cabinet: Option<Convolution>,
    spec: Option<ProcessSpec>,
}
impl Amp {
    pub fn new(config: Config) -> Result<Self, Error> {
        if config.schema != 1 {
            return Err(Error::InvalidParameter);
        }
        config.controls.validate()?;
        Ok(Self {
            config,
            inner: Oversampled::new(
                Circuit {
                    controls: config.controls,
                    states: vec![],
                    spec: None,
                },
                config.oversampling,
            ),
            cabinet: None,
            spec: None,
        })
    }
    /// Cabinet rate must match the eventual base rate; IR is applied after the dry/wet circuit blend.
    pub fn with_cabinet(
        config: Config,
        impulse: Vec<f32>,
        rate: f64,
        partition: usize,
    ) -> Result<Self, Error> {
        let mut p = Self::new(config)?;
        p.cabinet = Some(Convolution::new(impulse, rate, partition)?);
        Ok(p)
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        controls.validate()?;
        self.inner.inner_mut().controls = controls;
        self.config.controls = controls;
        Ok(())
    }
}
impl AudioProcessor for Amp {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if !(8000. ..=192000.).contains(&s.sample_rate) || s.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        self.spec = None;
        self.inner.prepare(s)?;
        if let Some(c) = &mut self.cabinet {
            c.prepare(s)?;
        }
        self.spec = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self
            .spec
            .is_none_or(|s| b.channels() != s.channels || b.frames() > s.max_block_frames)
        {
            b.samples_mut().fill(0.);
            return;
        }
        self.inner.process(b, ctx);
        if let Some(c) = &mut self.cabinet {
            c.process(b, ctx);
        }
    }
    fn reset(&mut self) {
        self.inner.reset();
        if let Some(c) = &mut self.cabinet {
            c.reset();
        }
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames() + self.cabinet.as_ref().map_or(0, |c| c.latency_frames())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn implicit_solution_and_asymmetry() {
        for input in [-20., -1., 0., 0.1, 1., 20.] {
            let y = solve(0., input, 0.2, 1.);
            let residual = y + 0.2 * (y - input + diode(y, 1.).0);
            assert!(residual.abs() < 0.00001, "{input} {residual}");
            assert!((solve(0., -input, 0.2, 1.) + y).abs() < 1e-8);
        }
        assert!((solve(0., -10., 0.2, 1.7) + solve(0., 10., 0.2, 1.7)).abs() > 0.01);
    }
    #[test]
    fn prepared_chain_partition_and_reset() {
        let render = |block: usize| {
            let mut p = Amp::with_cabinet(Default::default(), vec![1.], 48000., 64).unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 1,
            })
            .unwrap();
            assert_eq!(p.latency_frames(), 160);
            let mut data: Vec<_> = (0..4096).map(|i| 0.2 * (i as f32 * 0.07).sin()).collect();
            for b in data.chunks_mut(block) {
                assert_eq!(
                    allocation_counter::measure(|| p.process(
                        &mut AudioBuffer::new(b, 1).unwrap(),
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
            p.reset();
            data
        };
        assert_eq!(render(128), render(1));
    }
}
