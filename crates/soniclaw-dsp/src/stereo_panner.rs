//! Stereo gain panning with selectable center attenuation.
use crate::logarithmic_ramp::{Curve, LogarithmicRamp};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::f32::consts::FRAC_PI_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanLaw {
    Linear,
    Balanced,
    Sine3Db,
    Sine4_5Db,
    Sine6Db,
    SquareRoot3Db,
    SquareRoot4_5Db,
}

impl PanLaw {
    /// Gains at a position between −1 (left) and +1 (right).
    pub fn gains(self, pan: f32) -> Result<[f32; 2], Error> {
        validate(pan)?;
        Ok(self.gains_unchecked(pan))
    }

    fn gains_unchecked(self, pan: f32) -> [f32; 2] {
        if pan == -1. {
            return [1., 0.];
        }
        if pan == 1. {
            return [0., 1.];
        }
        let right = 0.5 * (pan + 1.);
        let left = 1. - right;
        match self {
            Self::Balanced => [1. - pan.max(0.), 1. + pan.min(0.)],
            Self::Linear => [left, right],
            Self::SquareRoot3Db => [left.sqrt(), right.sqrt()],
            Self::SquareRoot4_5Db => [left, right].map(|x| (x * x.sqrt()).sqrt()),
            Self::Sine3Db => [left, right].map(|x| (x * FRAC_PI_2).sin()),
            Self::Sine4_5Db => [left, right].map(|x| {
                let s = (x * FRAC_PI_2).sin();
                s * s.sqrt()
            }),
            Self::Sine6Db => [left, right].map(|x| (x * FRAC_PI_2).sin().powi(2)),
        }
    }
}

fn validate(pan: f32) -> Result<(), Error> {
    if !(-1. ..=1.).contains(&pan) {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

/// Smooths the pan position over a user-selected number of frames.
pub struct StereoPanner {
    law: PanLaw,
    position: LogarithmicRamp,
    smoothing_frames: u32,
}

impl StereoPanner {
    pub fn new(law: PanLaw, initial_pan: f32, smoothing_frames: u32) -> Result<Self, Error> {
        validate(initial_pan)?;
        Ok(Self {
            law,
            position: LogarithmicRamp::new(initial_pan, Curve::Linear, Curve::Linear)?,
            smoothing_frames,
        })
    }

    /// Selects a rule immediately. Call at an appropriate transition point.
    pub fn set_law(&mut self, law: PanLaw) {
        self.law = law;
    }

    pub fn set_pan(&mut self, target: f32) -> Result<(), Error> {
        validate(target)?;
        if target != self.position.target() {
            self.position.set_target(target, self.smoothing_frames)?;
        }
        Ok(())
    }

    pub fn pan(&self) -> f32 {
        self.position.value()
    }

    /// Route a mono source to a separate stereo interleaved output.
    pub fn process_mono(
        &mut self,
        input: &[f32],
        output: &mut AudioBuffer<'_>,
    ) -> Result<(), Error> {
        if output.channels() != 2 || output.frames() != input.len() {
            return Err(Error::InvalidBuffer);
        }
        for (&sample, frame) in input
            .iter()
            .zip(output.samples_mut().as_chunks_mut::<2>().0)
        {
            let [left, right] = self.law.gains_unchecked(self.position.next_value());
            *frame = [sample * left, sample * right];
        }
        Ok(())
    }

    /// Attenuate each existing stereo channel independently, with no crossfeed.
    pub fn process_stereo(&mut self, buffer: &mut AudioBuffer<'_>) -> Result<(), Error> {
        if buffer.channels() != 2 {
            return Err(Error::InvalidBuffer);
        }
        for [left, right] in buffer.samples_mut().as_chunks_mut::<2>().0 {
            let [gain_left, gain_right] = self.law.gains_unchecked(self.position.next_value());
            *left *= gain_left;
            *right *= gain_right;
        }
        Ok(())
    }
}

impl AudioProcessor for StereoPanner {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        if spec.validate()?.channels != 2 {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.process_stereo(buffer).is_err() {
            buffer.clear();
        }
    }
    fn reset(&mut self) {
        let _ = self.position.set_target(self.position.target(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laws_have_expected_center_and_endpoints() {
        for (law, expected_db) in [
            (PanLaw::Balanced, 0.),
            (PanLaw::Linear, -6.0206),
            (PanLaw::Sine3Db, -3.0103),
            (PanLaw::Sine4_5Db, -4.51545),
            (PanLaw::Sine6Db, -6.0206),
            (PanLaw::SquareRoot3Db, -3.0103),
            (PanLaw::SquareRoot4_5Db, -4.51545),
        ] {
            assert_eq!(law.gains(-1.).unwrap(), [1., 0.]);
            assert_eq!(law.gains(1.).unwrap(), [0., 1.]);
            let center = law.gains(0.).unwrap();
            assert_eq!(center[0], center[1]);
            assert!((20. * center[0].log10() - expected_db).abs() < 1e-5);
            assert!(law.gains(f32::NAN).is_err());
        }
        for law in [PanLaw::Sine3Db, PanLaw::SquareRoot3Db] {
            for i in -100..=100 {
                let [a, b] = law.gains(i as f32 / 100.).unwrap();
                assert!((a * a + b * b - 1.).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn mono_and_stereo_are_shaped_and_ramp_once_per_frame() {
        let make = || {
            let mut p = StereoPanner::new(PanLaw::Sine3Db, -1., 32).unwrap();
            p.set_pan(1.).unwrap();
            p
        };
        let mut p = make();
        let mut mono = [0.; 64];
        let mut invalid = [0.; 4];
        assert!(p
            .process_mono(&[1.], &mut AudioBuffer::new(&mut invalid, 2).unwrap())
            .is_err());
        assert_eq!(p.pan(), -1.);
        for chunk in mono.chunks_mut(10) {
            let n = chunk.len() / 2;
            p.process_mono(&[1.; 5][..n], &mut AudioBuffer::new(chunk, 2).unwrap())
                .unwrap();
            p.set_pan(1.).unwrap();
        }
        assert_eq!(p.pan(), 1.);
        let mut stereo = [1.; 64];
        make()
            .process_stereo(&mut AudioBuffer::new(&mut stereo, 2).unwrap())
            .unwrap();
        assert_eq!(mono, stereo);
        assert_eq!(mono[62..], [0., 1.]);
        let mut independent = [0.5, -0.25];
        StereoPanner::new(PanLaw::Balanced, -1., 0)
            .unwrap()
            .process_stereo(&mut AudioBuffer::new(&mut independent, 2).unwrap())
            .unwrap();
        assert_eq!(independent, [0.5, 0.]);
    }
}
