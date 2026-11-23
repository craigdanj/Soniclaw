//! Pressure-driven single-reed model and a closed-open cylindrical bore.
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::f64::consts::TAU;

const LOWEST_NOTE: f32 = 65.;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub frequency_hz: f32,
    pub breath: f32,
    pub reed_slope: f32,
    pub bore_retention: f32,
    pub damping_hz: f32,
    pub gain: f32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            frequency_hz: 220.,
            breath: 0.,
            reed_slope: 0.30,
            bore_retention: 0.985,
            damping_hz: 6500.,
            gain: 0.38,
        }
    }
}
impl Controls {
    fn validated(self) -> Result<Self, Error> {
        if !(LOWEST_NOTE..=1400.).contains(&self.frequency_hz)
            || !(0. ..=1.).contains(&self.breath)
            || !(0.15..=0.6).contains(&self.reed_slope)
            || !(0.85..=0.995).contains(&self.bore_retention)
            || !(300. ..=18_000.).contains(&self.damping_hz)
            || !(0. ..=1.).contains(&self.gain)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }
}

struct Bore {
    ring: Vec<f32>,
    head: usize,
    delay: f64,
    target_delay: f64,
    breath: f32,
    breath_alpha: f32,
    damping_alpha: f32,
    dc_alpha: f32,
    low: f32,
    dc: f32,
}
impl Bore {
    fn configure(&mut self, rate: f64, controls: Controls) {
        let damping = f64::from(controls.damping_hz).min(rate * 0.42);
        let alpha = 1. - (-TAU * damping / rate).exp();
        self.damping_alpha = alpha as f32;
        // The bore inverts once on reflection, for a quarter-wave fundamental.
        self.target_delay =
            (rate / (2. * f64::from(controls.frequency_hz)) - (1. - alpha) / alpha).max(2.);
    }

    fn tick(&mut self, controls: Controls) -> f32 {
        self.breath += self.breath_alpha * (controls.breath - self.breath);
        self.delay += (self.target_delay - self.delay).clamp(-0.15, 0.15);
        let n = self.ring.len();
        let d = self.delay.floor() as usize;
        let fraction = self.delay.fract() as f32;
        let first = self.ring[(self.head + n - d) % n];
        let second = self.ring[(self.head + n - d - 1) % n];
        let wave = first + (second - first) * fraction;
        self.low += self.damping_alpha * (wave - self.low);
        let reflected = -controls.bore_retention * self.low;
        let pressure_difference = reflected - self.breath;
        // At large positive mouth pressure the reed closes and reflects the wave.
        let reflection = (0.68 - controls.reed_slope * pressure_difference).clamp(0., 1.);
        let outgoing = self.breath + pressure_difference * reflection;
        self.ring[self.head] = outgoing;
        self.head = (self.head + 1) % n;
        self.dc += self.dc_alpha * (wave - self.dc);
        (wave - self.dc) * controls.gain
    }
}

pub struct ReedVoice {
    controls: Controls,
    prepared: Option<(ProcessSpec, Bore)>,
}
impl ReedVoice {
    pub fn new(controls: Controls) -> Result<Self, Error> {
        Ok(Self {
            controls: controls.validated()?,
            prepared: None,
        })
    }
    /// Update breath or tuning without reallocating or resetting the bore.
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        let controls = controls.validated()?;
        if let Some((spec, bore)) = &mut self.prepared {
            bore.configure(spec.sample_rate, controls);
        }
        self.controls = controls;
        Ok(())
    }
    pub fn next_sample(&mut self) -> f32 {
        self.prepared
            .as_mut()
            .map_or(0., |(_, bore)| bore.tick(self.controls))
    }
}
impl AudioProcessor for ReedVoice {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        if !(8_000. ..=192_000.).contains(&spec.sample_rate) || spec.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        let max_delay = (spec.sample_rate / (2. * f64::from(LOWEST_NOTE))).ceil() as usize + 4;
        let mut bore = Bore {
            ring: vec![0.; max_delay],
            head: 0,
            delay: 0.,
            target_delay: 0.,
            breath: 0.,
            breath_alpha: (1. - (-1. / (0.006 * spec.sample_rate)).exp()) as f32,
            damping_alpha: 0.,
            dc_alpha: (1. - (-TAU * 15. / spec.sample_rate).exp()) as f32,
            low: 0.,
            dc: 0.,
        };
        bore.configure(spec.sample_rate, self.controls);
        bore.delay = bore.target_delay;
        self.prepared = Some((spec, bore));
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some((spec, bore)) = &mut self.prepared else {
            buffer.clear();
            return;
        };
        if buffer.channels() != spec.channels || buffer.frames() > spec.max_block_frames {
            buffer.clear();
            return;
        }
        for frame in buffer.samples_mut().chunks_exact_mut(spec.channels) {
            frame.fill(bore.tick(self.controls));
        }
    }

    fn reset(&mut self) {
        if let Some((_, bore)) = &mut self.prepared {
            bore.ring.fill(0.);
            bore.head = 0;
            bore.delay = bore.target_delay;
            bore.low = 0.;
            bore.dc = 0.;
            bore.breath = 0.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advanced::analysis::PitchDetector;

    #[test]
    fn sustained_pitch_tracks_bore_length_and_expires_without_breath() {
        for frequency_hz in [220., 330.] {
            let controls = Controls {
                frequency_hz,
                breath: 0.8,
                ..Default::default()
            };
            let mut voice = ReedVoice::new(controls).unwrap();
            voice
                .prepare(ProcessSpec {
                    sample_rate: 48_000.,
                    max_block_frames: 128,
                    channels: 1,
                })
                .unwrap();
            for _ in 0..24_000 {
                voice.next_sample();
            }
            let signal: Vec<_> = (0..4096).map(|_| voice.next_sample()).collect();
            let mut detector = PitchDetector::new(48_000., 4096, 80., 650.).unwrap();
            let found = detector.estimate(&signal).unwrap();
            assert!(
                (found.hz / f64::from(frequency_hz) - 1.).abs() < 0.025,
                "{found:?}"
            );
            voice
                .set_controls(Controls {
                    breath: 0.,
                    ..controls
                })
                .unwrap();
            for _ in 0..24_000 {
                voice.next_sample();
            }
            assert!(voice.next_sample().abs() < 0.0001);
        }
    }
}
