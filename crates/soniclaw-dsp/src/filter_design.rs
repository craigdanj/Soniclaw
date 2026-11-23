//! Offline coefficient design and prepared FIR and Butterworth processing.
#[path = "advanced_design.rs"]
mod advanced_design;
use crate::precision::Sample;
pub use advanced_design::*;
use rustfft::num_complex::Complex64;
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};
use soniclaw_core::{AudioBuffer64, AudioProcessor64};
use std::f64::consts::{PI, TAU};

const MAX_TAPS: usize = 4095;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Response {
    LowPass,
    HighPass,
}

fn check_frequency(rate: f64, hz: f64) -> Result<(), Error> {
    if !rate.is_finite()
        || !(8_000. ..=384_000.).contains(&rate)
        || !hz.is_finite()
        || !(rate * 0.00001..rate * 0.49999).contains(&hz)
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}

pub(crate) fn i0(x: f64) -> f64 {
    let mut term = 1.;
    let mut sum = term;
    for k in 1..32 {
        term *= (x * 0.5 / f64::from(k)).powi(2);
        sum += term;
        if term < sum * 1e-16 {
            break;
        }
    }
    sum
}

/// Odd-length symmetric FIR from an estimated Kaiser transition specification.
pub fn kaiser(
    rate: f64,
    cutoff: f64,
    transition: f64,
    attenuation_db: f64,
    response: Response,
) -> Result<Vec<f64>, Error> {
    check_frequency(rate, cutoff)?;
    if !transition.is_finite()
        || transition <= 0.
        || cutoff - transition * 0.5 <= 0.
        || cutoff + transition * 0.5 >= rate * 0.5
        || !(21. ..=120.).contains(&attenuation_db)
    {
        return Err(Error::InvalidParameter);
    }
    let estimated = ((attenuation_db - 8.) / (2.285 * TAU * transition / rate)).ceil();
    if estimated > (MAX_TAPS - 1) as f64 {
        return Err(Error::InvalidParameter);
    }
    let taps = (estimated as usize + 1).max(3) | 1;
    let beta = if attenuation_db > 50. {
        0.1102 * (attenuation_db - 8.7)
    } else {
        0.5842 * (attenuation_db - 21.).powf(0.4) + 0.07886 * (attenuation_db - 21.)
    };
    let center = taps / 2;
    let norm = i0(beta);
    let fc = cutoff / rate;
    let mut coeffs: Vec<_> = (0..taps)
        .map(|i| {
            let distance = i as f64 - center as f64;
            let ratio = distance / center as f64;
            let window = i0(beta * (1. - ratio * ratio).max(0.).sqrt()) / norm;
            let sinc = if i == center {
                2. * fc
            } else {
                (TAU * fc * distance).sin() / (PI * distance)
            };
            window * sinc
        })
        .collect();
    let dc: f64 = coeffs.iter().sum();
    if !dc.is_finite() || dc.abs() < 1e-12 {
        return Err(Error::InvalidParameter);
    }
    for a in &mut coeffs {
        *a /= dc;
    }
    if matches!(response, Response::HighPass) {
        for a in &mut coeffs {
            *a = -*a;
        }
        coeffs[center] += 1.;
    }
    Ok(coeffs)
}

#[derive(Clone, Copy, Debug)]
pub struct Section {
    b: [f64; 3],
    a: [f64; 2],
}

impl Section {
    /// Normalize a biquad by a0 and reject poles on or outside the unit circle.
    pub fn new(numerator: [f64; 3], denominator: [f64; 3]) -> Result<Self, Error> {
        let a0 = denominator[0];
        let b = numerator.map(|coefficient| coefficient / a0);
        let a = [denominator[1] / a0, denominator[2] / a0];
        if !a0.is_finite()
            || b.iter().chain(&a).any(|x| !x.is_finite())
            || a[1].abs() >= 1.
            || 1. + a[0] + a[1] <= 0.
            || 1. - a[0] + a[1] <= 0.
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self { b, a })
    }

    pub fn numerator(self) -> [f64; 3] {
        self.b
    }
    pub fn denominator(self) -> [f64; 3] {
        [1., self.a[0], self.a[1]]
    }
}

/// Bilinear-transform Butterworth cascade, ordered from lowest to highest Q.
pub fn butterworth(
    rate: f64,
    cutoff: f64,
    order: usize,
    response: Response,
) -> Result<Vec<Section>, Error> {
    check_frequency(rate, cutoff)?;
    if !(1..=20).contains(&order) {
        return Err(Error::InvalidParameter);
    }
    let k = (PI * cutoff / rate).tan();
    let low = matches!(response, Response::LowPass);
    let mut sections = Vec::with_capacity(order.div_ceil(2));
    if order % 2 == 1 {
        let gain = if low { k } else { 1. } / (1. + k);
        sections.push(Section {
            b: [gain, if low { gain } else { -gain }, 0.],
            a: [(k - 1.) / (1. + k), 0.],
        });
    }
    for pair in (0..order / 2).rev() {
        let inverse_q = 2. * (PI * (2 * pair + 1) as f64 / (2 * order) as f64).sin();
        let k2 = k * k;
        let norm = 1. / (1. + inverse_q * k + k2);
        let gain = if low { k2 } else { 1. } * norm;
        sections.push(Section {
            b: [gain, if low { 2. * gain } else { -2. * gain }, gain],
            a: [2. * (k2 - 1.) * norm, (1. - inverse_q * k + k2) * norm],
        });
    }
    Ok(sections)
}

pub fn fir_at(coefficients: &[f64], normalized_frequency: f64) -> Complex64 {
    let z = Complex64::from_polar(1., -TAU * normalized_frequency);
    coefficients
        .iter()
        .rev()
        .fold(Complex64::default(), |sum, &tap| sum * z + tap)
}

pub fn iir_at(sections: &[Section], normalized_frequency: f64) -> Complex64 {
    let z = Complex64::from_polar(1., -TAU * normalized_frequency);
    sections.iter().fold(Complex64::new(1., 0.), |product, s| {
        product * (s.b[0] + z * (s.b[1] + z * s.b[2])) / (1. + z * (s.a[0] + z * s.a[1]))
    })
}

/// Local phase derivative in samples. At response zeros phase is undefined.
pub fn group_delay(at: impl Fn(f64) -> Complex64, normalized_frequency: f64) -> Option<f64> {
    let df = 1e-6;
    if !normalized_frequency.is_finite() {
        return None;
    }
    let a = at(normalized_frequency - df);
    let b = at(normalized_frequency + df);
    (a.norm() > 1e-10 && b.norm() > 1e-10).then(|| -(b * a.conj()).arg() / (2. * TAU * df))
}

enum Design {
    Fir(Vec<f64>),
    Iir(Vec<Section>),
}

pub struct Filter {
    design: Design,
    sample_rate: f64,
    spec: Option<ProcessSpec>,
    state: Vec<f64>,
    cursor: usize,
}

impl Filter {
    fn new(sample_rate: f64, design: Design) -> Self {
        Self {
            design,
            sample_rate,
            spec: None,
            state: Vec::new(),
            cursor: 0,
        }
    }

    /// Accept an odd symmetric kernel so its reported latency is exact.
    pub fn fir(rate: f64, taps: Vec<f64>) -> Result<Self, Error> {
        check_frequency(rate, rate * 0.25)?;
        if taps.is_empty()
            || taps.len() > MAX_TAPS
            || taps.len().is_multiple_of(2)
            || taps
                .iter()
                .zip(taps.iter().rev())
                .any(|(a, b)| !a.is_finite() || a.abs() > 16. || (a - b).abs() > 1e-10)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self::new(rate, Design::Fir(taps)))
    }

    pub fn iir(rate: f64, sections: Vec<Section>) -> Result<Self, Error> {
        check_frequency(rate, rate * 0.25)?;
        if sections.is_empty() || sections.len() > 10 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self::new(rate, Design::Iir(sections)))
    }
}

impl Filter {
    fn run<T: Sample>(&mut self, samples: &mut [T], spec: ProcessSpec) {
        for frame in samples.chunks_exact_mut(spec.channels) {
            match &self.design {
                Design::Fir(taps) => {
                    let n = taps.len();
                    for (sample, history) in frame.iter_mut().zip(self.state.chunks_exact_mut(n)) {
                        let input = sample.to_f64();
                        history[self.cursor] = if input.is_finite() { input } else { 0. };
                        let (before, after) = history.split_at(self.cursor + 1);
                        *sample = T::from_f64(
                            taps.iter()
                                .zip(before.iter().rev().chain(after.iter().rev()))
                                .map(|(a, b)| a * b)
                                .sum::<f64>(),
                        );
                    }
                    self.cursor = (self.cursor + 1) % n;
                }
                Design::Iir(sections) => {
                    for (sample, states) in frame
                        .iter_mut()
                        .zip(self.state.chunks_exact_mut(2 * sections.len()))
                    {
                        let input = sample.to_f64();
                        let mut x = if input.is_finite() { input } else { 0. };
                        for (s, d) in sections.iter().zip(states.as_chunks_mut::<2>().0) {
                            let y = s.b[0] * x + d[0];
                            d[0] = s.b[1] * x - s.a[0] * y + d[1];
                            d[1] = s.b[2] * x - s.a[1] * y;
                            x = y;
                        }
                        *sample = T::from_f64(x);
                    }
                }
            }
        }
    }
}

impl AudioProcessor for Filter {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if (spec.sample_rate - self.sample_rate).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        let size = match &self.design {
            Design::Fir(h) => h.len(),
            Design::Iir(s) => 2 * s.len(),
        };
        self.state = vec![0.; size * spec.channels];
        self.cursor = 0;
        self.spec = Some(spec);
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            buffer.clear();
            return;
        };
        if buffer.channels() != spec.channels || buffer.frames() > spec.max_block_frames {
            buffer.clear();
            return;
        }
        self.run(buffer.samples_mut(), spec);
    }

    fn reset(&mut self) {
        self.state.fill(0.);
        self.cursor = 0;
    }
    fn latency_frames(&self) -> usize {
        match &self.design {
            Design::Fir(h) => h.len() / 2,
            Design::Iir(_) => 0,
        }
    }
}

/// Double-precision version of the prepared FIR or second-order-section filter.
pub struct Filter64(Filter);
impl Filter64 {
    pub fn fir(rate: f64, taps: Vec<f64>) -> Result<Self, Error> {
        Filter::fir(rate, taps).map(Self)
    }
    pub fn iir(rate: f64, sections: Vec<Section>) -> Result<Self, Error> {
        Filter::iir(rate, sections).map(Self)
    }
}
impl AudioProcessor64 for Filter64 {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        self.0.prepare(spec)
    }
    fn process(&mut self, buffer: &mut AudioBuffer64<'_>, _: &ProcessContext<'_>) {
        let Some(spec) = self.0.spec else {
            buffer.clear();
            return;
        };
        if buffer.channels() != spec.channels || buffer.frames() > spec.max_block_frames {
            buffer.clear();
            return;
        }
        self.0.run(buffer.samples_mut(), spec);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_frames(&self) -> usize {
        self.0.latency_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_cutoff_and_fir_delay() {
        for response in [Response::LowPass, Response::HighPass] {
            let fir = kaiser(48_000., 6_000., 2_000., 70., response).unwrap();
            let pass = if matches!(response, Response::LowPass) {
                0.
            } else {
                0.5
            };
            let stop = 0.5 - pass;
            assert!((fir_at(&fir, pass).norm() - 1.).abs() < 0.001);
            assert!(fir_at(&fir, stop).norm() < 0.001);
            assert!(
                (group_delay(|f| fir_at(&fir, f), pass).unwrap() - (fir.len() / 2) as f64).abs()
                    < 1e-5
            );
            for order in [1, 2, 3, 8, 20] {
                let sections = butterworth(48_000., 6_000., order, response).unwrap();
                assert!(
                    (iir_at(&sections, 0.125).norm() - std::f64::consts::FRAC_1_SQRT_2).abs()
                        < 1e-9
                );
                assert!((iir_at(&sections, pass).norm() - 1.).abs() < 1e-9);
            }
        }
        assert!(kaiser(48_000., 6_000., 0.01, 90., Response::LowPass).is_err());
    }
}
