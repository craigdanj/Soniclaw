//! Independent per-channel peak or root-mean-square envelope ballistics.
use soniclaw_core::{AudioBuffer, Error, ProcessSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeMode {
    Peak,
    Rms,
}

/// Time constants in milliseconds. At one time constant, a step has traversed about 63%
/// of the internal amplitude (Peak) or power (Rms) difference.
pub struct EnvelopeFollower {
    spec: ProcessSpec,
    mode: EnvelopeMode,
    states: Vec<f64>,
    attack: f64,
    release: f64,
}

impl EnvelopeFollower {
    pub fn new(
        spec: ProcessSpec,
        mode: EnvelopeMode,
        attack_ms: f64,
        release_ms: f64,
    ) -> Result<Self, Error> {
        let spec = spec.validate()?;
        let mut follower = Self {
            spec,
            mode,
            states: vec![0.; spec.channels],
            attack: 0.,
            release: 0.,
        };
        follower.set_times(attack_ms, release_ms)?;
        Ok(follower)
    }

    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64) -> Result<(), Error> {
        if !(0. ..=60_000.).contains(&attack_ms) || !(0. ..=60_000.).contains(&release_ms) {
            return Err(Error::InvalidParameter);
        }
        self.attack = coefficient(self.spec.sample_rate, attack_ms);
        self.release = coefficient(self.spec.sample_rate, release_ms);
        Ok(())
    }

    pub fn process_sample(&mut self, channel: usize, sample: f32) -> Result<f32, Error> {
        let state = self.states.get_mut(channel).ok_or(Error::InvalidBuffer)?;
        Ok(tick(self.mode, self.attack, self.release, state, sample))
    }

    /// Replace samples with the corresponding continuous envelope on each channel.
    pub fn process(&mut self, buffer: &mut AudioBuffer<'_>) -> Result<(), Error> {
        if buffer.channels() != self.spec.channels || buffer.frames() > self.spec.max_block_frames {
            return Err(Error::InvalidBuffer);
        }
        for frame in buffer.samples_mut().chunks_exact_mut(self.spec.channels) {
            for (sample, state) in frame.iter_mut().zip(&mut self.states) {
                *sample = tick(self.mode, self.attack, self.release, state, *sample);
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.states.fill(0.);
    }
}

fn coefficient(rate: f64, ms: f64) -> f64 {
    if ms == 0. {
        1.
    } else {
        -(-1. / (rate * ms * 0.001)).exp_m1()
    }
}

fn tick(mode: EnvelopeMode, attack: f64, release: f64, state: &mut f64, sample: f32) -> f32 {
    let amplitude = if sample.is_finite() {
        f64::from(sample).abs()
    } else {
        0.
    };
    let target = match mode {
        EnvelopeMode::Peak => amplitude,
        EnvelopeMode::Rms => amplitude * amplitude,
    };
    let rate = if target > *state { attack } else { release };
    *state += rate * (target - *state);
    let level = match mode {
        EnvelopeMode::Peak => *state,
        EnvelopeMode::Rms => state.max(0.).sqrt(),
    };
    level as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_time_and_channel_isolation() {
        let spec = ProcessSpec {
            sample_rate: 1000.,
            max_block_frames: 32,
            channels: 2,
        };
        for mode in [EnvelopeMode::Peak, EnvelopeMode::Rms] {
            let mut env = EnvelopeFollower::new(spec, mode, 16., 32.).unwrap();
            let mut signal = [0.; 32];
            for [left, _] in signal.as_chunks_mut::<2>().0 {
                *left = -1.;
            }
            let count = allocation_counter::measure(|| {
                env.process(&mut AudioBuffer::new(&mut signal, 2).unwrap())
                    .unwrap()
            })
            .count_total;
            assert_eq!(count, 0);
            let power = if mode == EnvelopeMode::Peak { 1 } else { 2 };
            assert!((f64::from(signal[30]).powi(power) - (1. - (-1f64).exp())).abs() < 1e-7);
            assert!(signal.iter().skip(1).step_by(2).all(|v| *v == 0.));
            let mut tail = 0.;
            for _ in 0..32 {
                tail = env.process_sample(0, 0.).unwrap();
            }
            assert!(
                (f64::from(tail).powi(power) - (1. - (-1f64).exp()) * (-1f64).exp()).abs() < 1e-7
            );
            env.reset();
            assert_eq!(env.process_sample(0, f32::NAN).unwrap(), 0.);
        }
    }
}
