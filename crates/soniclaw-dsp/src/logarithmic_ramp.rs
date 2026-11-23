//! Endpoint-exact parameter ramps with independent rising and falling curves.
use soniclaw_core::{AudioBuffer, Error};

#[derive(Debug, Clone, Copy)]
pub enum Curve {
    Linear,
    /// Fast initial motion, then a gradual approach.
    Logarithmic {
        strength: f64,
    },
    /// Gradual initial motion that accelerates toward the target.
    Exponential {
        strength: f64,
    },
}

#[derive(Clone, Copy)]
enum PreparedCurve {
    Linear,
    Logarithmic { strength: f64, span: f64 },
    Exponential { strength: f64, span: f64 },
}

impl PreparedCurve {
    fn new(curve: Curve) -> Result<Self, Error> {
        let strength = match curve {
            Curve::Linear => return Ok(Self::Linear),
            Curve::Logarithmic { strength } | Curve::Exponential { strength } => strength,
        };
        if !(0. ..=16.).contains(&strength) {
            return Err(Error::InvalidParameter);
        }
        if strength < 1e-8 {
            return Ok(Self::Linear);
        }
        let span = strength.exp_m1();
        Ok(match curve {
            Curve::Logarithmic { .. } => Self::Logarithmic { strength, span },
            Curve::Exponential { .. } => Self::Exponential { strength, span },
            Curve::Linear => Self::Linear,
        })
    }

    fn evaluate(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::Logarithmic { strength, span } => (t * span).ln_1p() / strength,
            Self::Exponential { strength, span } => (t * strength).exp_m1() / span,
        }
    }
}

/// Scalar ramp for any finite parameter range, including zero and negative values.
pub struct LogarithmicRamp {
    rise: PreparedCurve,
    fall: PreparedCurve,
    active: PreparedCurve,
    start: f32,
    current: f32,
    target: f32,
    duration: u32,
    remaining: u32,
}

impl LogarithmicRamp {
    pub fn new(initial: f32, rise: Curve, fall: Curve) -> Result<Self, Error> {
        if !initial.is_finite() {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            rise: PreparedCurve::new(rise)?,
            fall: PreparedCurve::new(fall)?,
            active: PreparedCurve::Linear,
            start: initial,
            current: initial,
            target: initial,
            duration: 0,
            remaining: 0,
        })
    }

    pub fn value(&self) -> f32 {
        self.current
    }
    pub fn target(&self) -> f32 {
        self.target
    }
    pub fn remaining_frames(&self) -> u32 {
        self.remaining
    }

    /// Restart from the current value. A zero duration sets the value immediately.
    pub fn set_target(&mut self, target: f32, frames: u32) -> Result<(), Error> {
        if !target.is_finite() {
            return Err(Error::InvalidParameter);
        }
        self.start = self.current;
        self.target = target;
        if frames == 0 || target == self.current {
            self.current = target;
            self.remaining = 0;
            self.duration = 0;
        } else {
            self.active = if target > self.current {
                self.rise
            } else {
                self.fall
            };
            self.remaining = frames;
            self.duration = frames;
        }
        Ok(())
    }

    /// Advance by a frame and return the value for that frame.
    pub fn next_value(&mut self) -> f32 {
        self.advance(1)
    }

    /// Advance multiple frames in constant time. Returns the resulting value.
    pub fn advance(&mut self, frames: u32) -> f32 {
        if self.remaining == 0 || frames == 0 {
            return self.current;
        }
        self.remaining = self.remaining.saturating_sub(frames);
        if self.remaining == 0 {
            self.current = self.target;
        } else {
            let t = f64::from(self.duration - self.remaining) / f64::from(self.duration);
            let shape = self.active.evaluate(t).clamp(0., 1.);
            let a = f64::from(self.start);
            let b = f64::from(self.target);
            self.current = (a * (1. - shape) + b * shape) as f32;
        }
        self.current
    }

    /// Applies one shared ramp value to every channel of each successive frame.
    pub fn apply_gain(&mut self, audio: &mut AudioBuffer<'_>) {
        let channels = audio.channels();
        for frame in audio.samples_mut().chunks_exact_mut(channels) {
            let gain = self.next_value();
            for sample in frame {
                *sample *= gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asymmetric_shapes_and_continuous_retarget() {
        let mut r = LogarithmicRamp::new(
            -1.,
            Curve::Logarithmic { strength: 5. },
            Curve::Exponential { strength: 4. },
        )
        .unwrap();
        r.set_target(1., 100).unwrap();
        assert!(r.advance(50) > 0.5);
        let current = r.value();
        r.set_target(-0.9, 240).unwrap();
        assert_eq!(r.value(), current);
        assert!(r.advance(120) > 0.4);
        assert_eq!(r.advance(120), -0.9);
        assert_eq!(r.remaining_frames(), 0);
        assert!(r.set_target(f32::NAN, 1).is_err());
        assert_eq!(r.set_target(0., 0), Ok(()));
        assert_eq!(r.next_value(), 0.);
    }

    #[test]
    fn partitioned_shared_frame_gain_matches_advance() {
        let make = || {
            LogarithmicRamp::new(0., Curve::Logarithmic { strength: 2.25 }, Curve::Linear).unwrap()
        };
        let mut stepped = make();
        stepped.set_target(1., 100).unwrap();
        let mut data = [1.; 200];
        for part in data.chunks_mut(26) {
            stepped.apply_gain(&mut AudioBuffer::new(part, 2).unwrap());
        }
        assert!(data.as_chunks::<2>().0.iter().all(|s| s[0] == s[1]));
        let mut jumped = make();
        jumped.set_target(1., 100).unwrap();
        assert_eq!(jumped.advance(37), data[72]);
        assert_eq!(jumped.advance(u32::MAX), 1.);
        assert_eq!(data[199], 1.);
    }
}
