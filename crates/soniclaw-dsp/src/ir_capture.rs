//! Offline generation and deconvolution of exponential sweep measurements.
use rustfft::{num_complex::Complex64, FftPlanner};
use soniclaw_core::Error;
use std::f64::consts::TAU;

const MAX_FFT: usize = 1 << 22;

#[derive(Clone, Copy, Debug)]
pub struct Sweep {
    pub sample_rate: u32,
    pub frames: usize,
    pub first_hz: f64,
    pub last_hz: f64,
    pub amplitude: f32,
    pub fade_frames: usize,
}

impl Sweep {
    /// Return one mono sweep. Append silence for the measured system's tail.
    pub fn generate(self) -> Result<Vec<f32>, Error> {
        let rate = f64::from(self.sample_rate);
        if !(8_000..=192_000).contains(&self.sample_rate)
            || !(64..=MAX_FFT / 2).contains(&self.frames)
            || !(5. ..self.last_hz).contains(&self.first_hz)
            || !(self.first_hz..0.5 * rate).contains(&self.last_hz)
            || !(0.001..=1.).contains(&self.amplitude)
            || self.fade_frames > self.frames / 2
        {
            return Err(Error::InvalidParameter);
        }
        let growth = (self.last_hz / self.first_hz).ln();
        let scale = TAU * self.first_hz * (self.frames - 1) as f64 / (rate * growth);
        Ok((0..self.frames)
            .map(|i| {
                let progress = i as f64 / (self.frames - 1) as f64;
                let distance = i.min(self.frames - i - 1);
                let envelope = if distance < self.fade_frames {
                    (0.25 * TAU * distance as f64 / self.fade_frames as f64)
                        .sin()
                        .powi(2)
                } else {
                    1.
                };
                (f64::from(self.amplitude)
                    * (envelope * (scale * (growth * progress).exp_m1()).sin()))
                    as f32
            })
            .collect())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CaptureOptions {
    pub impulse_frames: usize,
    /// Search among this many nonnegative offsets for the strongest sample.
    /// Set to zero to retain the original time origin.
    pub search_frames: usize,
    pub pre_roll_frames: usize,
    /// Regularization power relative to the strongest reference bin, in dB.
    pub floor_db: f64,
    pub tail_fade_frames: usize,
}

#[derive(Debug)]
pub struct Capture {
    pub impulse: Vec<f32>,
    pub strongest_frame: usize,
    pub start_frame: usize,
}

/// Recover a linear response by regularized frequency-domain deconvolution.
/// The reference and capture must use the same sample clock and rate.
pub fn recover(
    reference: &[f32],
    recording: &[f32],
    options: CaptureOptions,
) -> Result<Capture, Error> {
    let length = reference
        .len()
        .checked_add(recording.len())
        .and_then(usize::checked_next_power_of_two)
        .filter(|&n| n <= MAX_FFT)
        .ok_or(Error::InvalidBuffer)?;
    if reference.len() < 16
        || recording.len() < reference.len()
        || reference
            .iter()
            .chain(recording)
            .any(|x| !x.is_finite() || x.abs() > 16.)
        || options.impulse_frames == 0
        || options.tail_fade_frames > options.impulse_frames
        || options.pre_roll_frames > options.search_frames
        || !(-160. ..=-20.).contains(&options.floor_db)
        || options.search_frames.saturating_add(options.impulse_frames) > recording.len()
    {
        return Err(Error::InvalidParameter);
    }
    let padded = |input: &[f32]| {
        let mut values = vec![Complex64::default(); length];
        for (bin, &sample) in values.iter_mut().zip(input) {
            bin.re = f64::from(sample);
        }
        values
    };
    let mut x = padded(reference);
    let mut y = padded(recording);
    let mut planner = FftPlanner::new();
    let forward = planner.plan_fft_forward(length);
    forward.process(&mut x);
    forward.process(&mut y);
    let max_power = x.iter().map(Complex64::norm_sqr).fold(0., f64::max);
    if max_power < 1e-16 || recording.iter().all(|x| *x == 0.) {
        return Err(Error::InvalidBuffer);
    }
    let epsilon = max_power * 10f64.powf(options.floor_db / 10.);
    for (observed, original) in y.iter_mut().zip(x) {
        *observed = *observed * original.conj() / (original.norm_sqr() + epsilon);
    }
    planner.plan_fft_inverse(length).process(&mut y);
    let strongest_frame = y[..options.search_frames.max(1)]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.re.abs().total_cmp(&b.1.re.abs()))
        .map_or(0, |(i, _)| i);
    let start_frame = strongest_frame.saturating_sub(options.pre_roll_frames);
    let fade_start = options.impulse_frames - options.tail_fade_frames;
    let impulse = y[start_frame..start_frame + options.impulse_frames]
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let window = if i >= fade_start {
                (0.25 * TAU * (i + 1 - fade_start) as f64 / options.tail_fade_frames as f64)
                    .cos()
                    .powi(2)
            } else {
                1.
            };
            (x.re * window / length as f64) as f32
        })
        .collect();
    Ok(Capture {
        impulse,
        strongest_frame,
        start_frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_signed_echoes_and_preserves_pre_roll() {
        let x = Sweep {
            sample_rate: 8000,
            frames: 4096,
            first_hz: 12.,
            last_hz: 3900.,
            amplitude: 0.5,
            fade_frames: 64,
        }
        .generate()
        .unwrap();
        let mut y = vec![0.; x.len() + 512];
        for (i, &x) in x.iter().enumerate() {
            y[i + 100] += 0.8 * x;
            y[i + 139] -= 0.35 * x;
        }
        let options = CaptureOptions {
            impulse_frames: 256,
            search_frames: 250,
            pre_roll_frames: 8,
            floor_db: -110.,
            tail_fade_frames: 32,
        };
        let result = recover(&x, &y, options).unwrap();
        assert_eq!((result.strongest_frame, result.start_frame), (100, 92));
        assert!((result.impulse[8] - 0.8).abs() < 0.04);
        assert!((result.impulse[47] + 0.35).abs() < 0.04);
        assert!(result.impulse[255].abs() < 1e-10);
        assert!(recover(&x, &vec![0.; y.len()], options).is_err());
    }
}
