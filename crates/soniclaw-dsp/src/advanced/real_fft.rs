//! Prepared real FFT, window tables and calibrated one-sided analysis.
use super::Complex32;
use crate::filter_design::i0;
use rustfft::{Fft, FftPlanner};
use soniclaw_core::Error;
use std::{f64::consts::TAU, sync::Arc};

/// A real transform of N samples represented by the N/2+1 nonnegative bins.
pub struct RealFft {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    packed: Vec<Complex32>,
    twiddles: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

impl RealFft {
    pub fn new(size: usize) -> Result<Self, Error> {
        if !size.is_power_of_two() || !(16..=32_768).contains(&size) {
            return Err(Error::InvalidSpec);
        }
        let mut planner = FftPlanner::new();
        let forward = planner.plan_fft_forward(size / 2);
        let inverse = planner.plan_fft_inverse(size / 2);
        let scratch = vec![
            Complex32::default();
            forward
                .get_inplace_scratch_len()
                .max(inverse.get_inplace_scratch_len())
        ];
        let twiddles = (0..=size / 2)
            .map(|bin| Complex32::from_polar(1., (-TAU * bin as f64 / size as f64) as f32))
            .collect();
        Ok(Self {
            forward,
            inverse,
            packed: vec![Complex32::default(); size / 2],
            twiddles,
            scratch,
        })
    }

    pub fn size(&self) -> usize {
        self.packed.len() * 2
    }
    pub fn bins(&self) -> usize {
        self.twiddles.len()
    }

    /// The forward transform uses an unnormalized DFT convention.
    pub fn forward(&mut self, input: &[f32], spectrum: &mut [Complex32]) -> Result<(), Error> {
        if input.len() != self.size() || spectrum.len() != self.bins() {
            return Err(Error::InvalidBuffer);
        }
        if input.iter().any(|&sample| !sample.is_finite()) {
            return Err(Error::InvalidParameter);
        }
        self.transform(
            input
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[even, odd]| Complex32::new(even, odd)),
        );
        for (bin, value) in spectrum.iter_mut().enumerate() {
            *value = self.bin(bin);
        }
        Ok(())
    }

    /// The inverse transform includes 1/N normalization and requires real DC and Nyquist bins.
    pub fn inverse(&mut self, spectrum: &[Complex32], output: &mut [f32]) -> Result<(), Error> {
        if output.len() != self.size() || spectrum.len() != self.bins() {
            return Err(Error::InvalidBuffer);
        }
        let half = self.packed.len();
        if spectrum[0].im != 0.
            || spectrum[half].im != 0.
            || spectrum
                .iter()
                .any(|s| !s.re.is_finite() || !s.im.is_finite())
        {
            return Err(Error::InvalidParameter);
        }
        for (bin, packed) in self.packed.iter_mut().enumerate() {
            let front = spectrum[bin];
            let back = spectrum[half - bin].conj();
            *packed = 0.5
                * ((front + back) + Complex32::i() * self.twiddles[bin].conj() * (front - back));
        }
        self.inverse
            .process_with_scratch(&mut self.packed, &mut self.scratch);
        let scale = 1. / half as f32;
        for (value, pair) in self.packed.iter().zip(output.as_chunks_mut::<2>().0) {
            *pair = [value.re * scale, value.im * scale];
        }
        Ok(())
    }

    /// One-sided peak amplitude. Interior bins are doubled; DC and Nyquist are not.
    pub fn magnitudes(
        &mut self,
        input: &[f32],
        window: Option<&AnalysisWindow>,
        output: &mut [f32],
    ) -> Result<(), Error> {
        if input.len() != self.size()
            || output.len() != self.bins()
            || window.is_some_and(|w| w.coefficients.len() != self.size())
        {
            return Err(Error::InvalidBuffer);
        }
        if input.iter().any(|&sample| !sample.is_finite()) {
            return Err(Error::InvalidParameter);
        }
        let pairs = input.as_chunks::<2>().0;
        let gain = if let Some(window) = window {
            self.transform(
                pairs
                    .iter()
                    .zip(window.coefficients.as_chunks::<2>().0)
                    .map(|(&[even, odd], &[we, wo])| Complex32::new(even * we, odd * wo)),
            );
            window.sum
        } else {
            self.size() as f32
        };
        let half = self.packed.len();
        for (index, out) in output.iter_mut().enumerate() {
            let scale = if index == 0 || index == half { 1. } else { 2. };
            *out = self.bin(index).norm() * (scale / gain);
        }
        Ok(())
    }

    fn transform(&mut self, pairs: impl Iterator<Item = Complex32>) {
        for (slot, value) in self.packed.iter_mut().zip(pairs) {
            *slot = value;
        }
        self.forward
            .process_with_scratch(&mut self.packed, &mut self.scratch);
    }

    fn bin(&self, index: usize) -> Complex32 {
        let half = self.packed.len();
        if index == 0 {
            return Complex32::new(self.packed[0].re + self.packed[0].im, 0.);
        }
        if index == half {
            return Complex32::new(self.packed[0].re - self.packed[0].im, 0.);
        }
        let a = self.packed[index];
        let b = self.packed[half - index].conj();
        0.5 * ((a + b) - Complex32::i() * self.twiddles[index] * (a - b))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum WindowShape {
    Rectangular,
    Triangular,
    Hann,
    Hamming,
    Blackman,
    BlackmanHarris,
    FlatTop,
    Kaiser { beta: f64 },
}
#[derive(Clone, Copy, Debug)]
pub enum WindowSampling {
    Periodic,
    Symmetric,
}
#[derive(Clone, Copy, Debug)]
pub enum WindowNormalization {
    None,
    UnityCoherentGain,
}

/// Precomputed coefficients for windowing or one-sided magnitude normalization.
pub struct AnalysisWindow {
    coefficients: Vec<f32>,
    sum: f32,
}

impl AnalysisWindow {
    pub fn new(
        size: usize,
        shape: WindowShape,
        sampling: WindowSampling,
        normalization: WindowNormalization,
    ) -> Result<Self, Error> {
        if !(16..=32_768).contains(&size) {
            return Err(Error::InvalidSpec);
        }
        let denominator = if let WindowShape::Kaiser { beta } = shape {
            if !(0. ..=24.).contains(&beta) {
                return Err(Error::InvalidParameter);
            }
            i0(beta)
        } else {
            1.
        };
        let period = match sampling {
            WindowSampling::Periodic => size,
            WindowSampling::Symmetric => size - 1,
        };
        let mut coefficients: Vec<_> = (0..size)
            .map(|i| {
                let position = i as f64 / period as f64;
                let phase = TAU * position;
                let c1 = phase.cos();
                let c2 = (2. * phase).cos();
                let c3 = (3. * phase).cos();
                let value = match shape {
                    WindowShape::Rectangular => 1.,
                    WindowShape::Triangular => 1. - (2. * position - 1.).abs(),
                    WindowShape::Hann => 0.5 - 0.5 * c1,
                    WindowShape::Hamming => 0.54 - 0.46 * c1,
                    WindowShape::Blackman => 0.42 - 0.5 * c1 + 0.08 * c2,
                    WindowShape::BlackmanHarris => {
                        0.35875 - 0.48829 * c1 + 0.14128 * c2 - 0.01168 * c3
                    }
                    WindowShape::FlatTop => {
                        0.21557895 - 0.41663158 * c1 + 0.277263158 * c2 - 0.083578947 * c3
                            + 0.006947368 * (4. * phase).cos()
                    }
                    WindowShape::Kaiser { beta } => {
                        i0(beta * (1. - (2. * position - 1.).powi(2)).max(0.).sqrt()) / denominator
                    }
                };
                value as f32
            })
            .collect();
        let sum = || -> f64 { coefficients.iter().map(|&x| f64::from(x)).sum() };
        let total = sum();
        if matches!(normalization, WindowNormalization::UnityCoherentGain) {
            let gain = (size as f64 / total) as f32;
            for sample in &mut coefficients {
                *sample *= gain;
            }
        }
        let sum = coefficients.iter().map(|&x| f64::from(x)).sum::<f64>() as f32;
        Ok(Self { coefficients, sum })
    }

    pub fn coefficients(&self) -> &[f32] {
        &self.coefficients
    }

    pub fn coherent_gain(&self) -> f32 {
        self.sum / self.coefficients.len() as f32
    }

    pub fn apply(&self, samples: &mut [f32]) -> Result<(), Error> {
        if samples.len() != self.coefficients.len() {
            return Err(Error::InvalidBuffer);
        }
        for (sample, window) in samples.iter_mut().zip(&self.coefficients) {
            *sample *= *window;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::FftProcessor;
    use super::*;

    #[test]
    fn real_transform_matches_complex_reference() {
        for size in [16, 512, 1024] {
            let mut fft = RealFft::new(size).unwrap();
            let mut reference = FftProcessor::new(size).unwrap();
            let input: Vec<f32> = (0..size)
                .map(|i| (i as f32 * 3.491).sin() + 0.25 * (i as f32 * 0.172).cos())
                .collect();
            let mut complex: Vec<_> = input.iter().map(|&v| Complex32::new(v, 0.)).collect();
            let mut bins = vec![Complex32::default(); fft.bins()];
            let mut reconstructed = vec![0.; size];
            reference.forward(&mut complex).unwrap();
            let allocations = allocation_counter::measure(|| {
                fft.forward(&input, &mut bins).unwrap();
                fft.inverse(&bins, &mut reconstructed).unwrap();
            });
            assert_eq!(allocations.count_total, 0);
            assert!(bins
                .iter()
                .zip(complex)
                .all(|(&a, b)| (a - b).norm() < 2e-4));
            assert!(input
                .iter()
                .zip(reconstructed)
                .all(|(&a, b)| (a - b).abs() < 2e-6));
        }
    }

    #[test]
    fn calibrates_tone_dc_and_nyquist_and_checks_buffers() {
        let mut fft = RealFft::new(1024).unwrap();
        let mut out = vec![0.; fft.bins()];
        let signal: Vec<_> = (0..1024)
            .map(|i| {
                0.125
                    + 0.5 * (TAU * i as f64 * 79. / 1024.).cos() as f32
                    + if i % 2 == 0 { 0.25 } else { -0.25 }
            })
            .collect();
        for shape in [
            WindowShape::FlatTop,
            WindowShape::BlackmanHarris,
            WindowShape::Kaiser { beta: 9. },
        ] {
            let window = AnalysisWindow::new(
                1024,
                shape,
                WindowSampling::Periodic,
                WindowNormalization::UnityCoherentGain,
            )
            .unwrap();
            assert!((window.coherent_gain() - 1.).abs() < 2e-7);
            assert_eq!(
                allocation_counter::measure(|| fft
                    .magnitudes(&signal, Some(&window), &mut out)
                    .unwrap())
                .count_total,
                0
            );
            for (bin, target) in [(0, 0.125), (79, 0.5), (512, 0.25)] {
                assert!(
                    (out[bin] - target).abs() < 8e-5,
                    "{shape:?} {bin} {}",
                    out[bin]
                );
            }
        }
        let prior = out.clone();
        assert!(fft.magnitudes(&signal[..1000], None, &mut out).is_err());
        assert_eq!(prior, out);
    }
}
