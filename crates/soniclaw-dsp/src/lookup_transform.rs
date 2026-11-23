//! Immutable linear-interpolation tables for reusable custom DSP curves.
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use std::sync::Arc;

const MAX_POINTS: usize = 1_048_576;

#[derive(Debug, Clone, Copy)]
pub struct ApproximationError {
    pub maximum_absolute: f64,
    pub rms_absolute: f64,
    pub maximum_relative: f64,
}

/// Sampled, finite transform over a closed input interval.
#[derive(Debug, Clone)]
pub struct LookupTransform {
    lower: f32,
    upper: f32,
    scale: f64,
    values: Arc<[f32]>,
}

impl LookupTransform {
    /// Sample the function at equally spaced points including both endpoints.
    pub fn prepare(
        lower: f32,
        upper: f32,
        points: usize,
        mut f: impl FnMut(f32) -> f32,
    ) -> Result<Self, Error> {
        if !lower.is_finite()
            || !upper.is_finite()
            || lower >= upper
            || !(2..=MAX_POINTS).contains(&points)
        {
            return Err(Error::InvalidParameter);
        }
        let span = f64::from(upper) - f64::from(lower);
        let values: Vec<_> = (0..points)
            .map(|i| f((f64::from(lower) + span * i as f64 / (points - 1) as f64) as f32))
            .collect();
        if values.iter().any(|x| !x.is_finite()) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            lower,
            upper,
            scale: (points - 1) as f64 / span,
            values: values.into(),
        })
    }

    pub fn domain(&self) -> (f32, f32) {
        (self.lower, self.upper)
    }

    pub fn points(&self) -> usize {
        self.values.len()
    }

    pub fn evaluate(&self, input: f32) -> Result<f32, Error> {
        if !(self.lower..=self.upper).contains(&input) {
            return Err(Error::InvalidParameter);
        }
        Ok(self.interpolate(input))
    }

    pub fn evaluate_clamped(&self, input: f32) -> Result<f32, Error> {
        if !input.is_finite() {
            return Err(Error::InvalidParameter);
        }
        Ok(self.interpolate(input.clamp(self.lower, self.upper)))
    }

    fn interpolate(&self, input: f32) -> f32 {
        if input <= self.lower {
            return self.values[0];
        }
        if input >= self.upper {
            return self.values[self.values.len() - 1];
        }
        let pos = (f64::from(input) - f64::from(self.lower)) * self.scale;
        let index = (pos as usize).min(self.values.len() - 2);
        let fraction = pos - index as f64;
        let a = f64::from(self.values[index]);
        let b = f64::from(self.values[index + 1]);
        (a + fraction * (b - a)) as f32
    }

    /// Measure errors on an evenly spaced grid. Relative error uses a positive
    /// minimum denominator so reference values near zero remain interpretable.
    pub fn measure_error(
        &self,
        points: usize,
        relative_floor: f64,
        mut reference: impl FnMut(f32) -> f32,
    ) -> Result<ApproximationError, Error> {
        if !(2..=MAX_POINTS).contains(&points)
            || !relative_floor.is_finite()
            || relative_floor <= 0.
        {
            return Err(Error::InvalidParameter);
        }
        let mut maximum_absolute = 0f64;
        let mut sum_squared = 0.;
        let mut maximum_relative = 0f64;
        let span = f64::from(self.upper) - f64::from(self.lower);
        for i in 0..points {
            let x = (f64::from(self.lower) + span * i as f64 / (points - 1) as f64) as f32;
            let expected = f64::from(reference(x));
            if !expected.is_finite() {
                return Err(Error::InvalidParameter);
            }
            let error = (f64::from(self.interpolate(x)) - expected).abs();
            maximum_absolute = maximum_absolute.max(error);
            maximum_relative = maximum_relative.max(error / expected.abs().max(relative_floor));
            sum_squared += error * error;
        }
        Ok(ApproximationError {
            maximum_absolute,
            rms_absolute: (sum_squared / points as f64).sqrt(),
            maximum_relative,
        })
    }
}

/// An in-place memoryless processor. Finite inputs clamp to the table domain;
/// nonfinite inputs are replaced with silence.
pub struct LookupWaveshaper {
    table: LookupTransform,
}

impl LookupWaveshaper {
    pub fn new(table: LookupTransform) -> Self {
        Self { table }
    }

    pub fn process_in_place(&self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.table.evaluate_clamped(*sample).unwrap_or(0.);
        }
    }
}

impl AudioProcessor for LookupWaveshaper {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate().map(|_| ())
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        self.process_in_place(audio.samples_mut());
    }
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_error_improves_with_density() {
        let f = |x: f32| (2.5 * x).tanh();
        let coarse = LookupTransform::prepare(-2., 2., 32, f).unwrap();
        let fine = LookupTransform::prepare(-2., 2., 256, f).unwrap();
        let a = coarse.measure_error(17003, 1e-3, f).unwrap();
        let b = fine.measure_error(17003, 1e-3, f).unwrap();
        assert!(a.maximum_absolute > 20. * b.maximum_absolute);
        assert!(b.maximum_absolute < 2e-4);
        assert!(b.maximum_relative < 0.001);
        assert_eq!(fine.evaluate(-2.).unwrap(), f(-2.));
        assert_eq!(fine.evaluate_clamped(3.).unwrap(), f(2.));
        assert!(fine.evaluate(3.).is_err());
        assert!(fine.evaluate_clamped(f32::NAN).is_err());
    }

    #[test]
    fn preserves_extreme_finite_values_and_rejects_nonfinite_curves() {
        let f = LookupTransform::prepare(-f32::MAX, f32::MAX, 2, |x| x).unwrap();
        assert_eq!(f.evaluate(0.).unwrap(), 0.);
        assert_eq!(f.evaluate(f32::MAX).unwrap(), f32::MAX);
        assert!(LookupTransform::prepare(0., 1., 2, |_| f32::INFINITY).is_err());
        let waveshaper = LookupWaveshaper::new(f);
        let mut data = [f32::NAN, f32::INFINITY, 0.];
        waveshaper.process_in_place(&mut data);
        assert_eq!(data, [0.; 3]);
    }
}
