//! Bounded windowed-sinc interpolation. Kernel evaluation uses no allocation.
//! Offline conversion allocates output and must run outside the audio callback.
use soniclaw_core::Error;
use std::f64::consts::PI;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResampleQuality {
    Linear,
    SincFast,
    #[default]
    SincBest,
}
impl ResampleQuality {
    pub fn radius(self, increment: f64) -> isize {
        match self {
            Self::Linear => 1,
            Self::SincFast => (16.0 * increment.clamp(1.0, 8.0)).ceil() as isize,
            Self::SincBest => (48.0 * increment.clamp(1.0, 8.0)).ceil() as isize,
        }
    }
    /// `increment` is source frames per output frame, in (0,32]. Read supplies
    /// zero padding or loop wrapping. This API never owns or releases sample PCM.
    pub fn interpolate(
        self,
        position: f64,
        increment: f64,
        mut read: impl FnMut(isize) -> f32,
    ) -> f32 {
        if !position.is_finite()
            || position < 0.0
            || position > isize::MAX as f64 / 2.0
            || !increment.is_finite()
            || !(0.0..=32.0).contains(&increment)
            || increment == 0.0
        {
            return 0.0;
        }
        let base = position.floor() as isize;
        if self == Self::Linear {
            let t = (position - base as f64) as f32;
            let first = read(base);
            // At integer positions, the lookahead has zero weight. Do not demand its page.
            return if t == 0. {
                first
            } else {
                first * (1. - t) + read(base + 1) * t
            };
        }
        let radius = self.radius(increment);
        let cutoff = 0.94 / increment.max(1.0);
        let mut value = 0.0;
        let mut norm = 0.0;
        for i in (base - radius + 1)..=(base + radius) {
            let x = i as f64 - position;
            let u = x / radius as f64;
            let window = 0.42 + 0.5 * (PI * u).cos() + 0.08 * (2.0 * PI * u).cos();
            let sinc = if x.abs() < 1e-12 {
                cutoff
            } else {
                (PI * cutoff * x).sin() / (PI * x)
            };
            let weight = sinc * window;
            value += read(i) as f64 * weight;
            norm += weight;
        }
        if norm.abs() > 1e-12 {
            (value / norm) as f32
        } else {
            0.0
        }
    }
}
pub fn resample_interleaved(
    input: &[f32],
    channels: usize,
    input_rate: f64,
    output_rate: f64,
    quality: ResampleQuality,
) -> Result<Vec<f32>, Error> {
    if channels == 0
        || channels > 64
        || !input.len().is_multiple_of(channels)
        || input.iter().any(|x| !x.is_finite())
        || !input_rate.is_finite()
        || !output_rate.is_finite()
        || !(1.0..=1_536_000.0).contains(&input_rate)
        || !(1.0..=1_536_000.0).contains(&output_rate)
    {
        return Err(Error::InvalidSpec);
    }
    let increment = input_rate / output_rate;
    if !(1.0 / 32.0..=32.0).contains(&increment) {
        return Err(Error::InvalidSpec);
    }
    let frames = input.len() / channels;
    let output_frames = (frames as f64 / increment).ceil() as usize;
    let count = output_frames
        .checked_mul(channels)
        .ok_or(Error::InvalidBuffer)?;
    if count > 64 * 1024 * 1024 {
        return Err(Error::InvalidBuffer);
    }
    let mut output = vec![0.0; count];
    for (f, frame) in output.chunks_exact_mut(channels).enumerate() {
        for (c, x) in frame.iter_mut().enumerate() {
            *x = quality.interpolate(f as f64 * increment, increment, |i| {
                if i >= 0 && (i as usize) < frames {
                    input[i as usize * channels + c]
                } else {
                    0.0
                }
            });
        }
    }
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }
    #[test]
    fn integer_linear_samples_do_not_read_a_zero_weight_page() {
        let mut reads = 0;
        let x = ResampleQuality::Linear.interpolate(1023., 1., |frame| {
            assert_eq!(frame, 1023);
            reads += 1;
            0.25
        });
        assert_eq!(x, 0.25);
        assert_eq!(reads, 1);
        let mut indices = Vec::new();
        let x = ResampleQuality::Linear.interpolate(1023.5, 1., |frame| {
            indices.push(frame);
            if frame == 1023 {
                0.25
            } else {
                0.75
            }
        });
        assert_eq!(indices, [1023, 1024]);
        assert_eq!(x, 0.5);
    }
    #[test]
    fn downsample_rejects_alias_and_preserves_passband() {
        let tone = |hz: f64| {
            (0..9600)
                .map(|i| (2.0 * PI * hz * i as f64 / 48000.0).sin() as f32)
                .collect::<Vec<_>>()
        };
        let high = tone(18000.0);
        let low = tone(1000.0);
        let a =
            resample_interleaved(&high, 1, 48000.0, 24000.0, ResampleQuality::SincBest).unwrap();
        let b = resample_interleaved(&high, 1, 48000.0, 24000.0, ResampleQuality::Linear).unwrap();
        let pass =
            resample_interleaved(&low, 1, 48000.0, 24000.0, ResampleQuality::SincBest).unwrap();
        assert!(rms(&a[128..a.len() - 128]) < rms(&b[128..b.len() - 128]) * 0.001);
        assert!(
            (rms(&pass[128..pass.len() - 128]) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.002
        );
    }
    #[test]
    fn channels_duration_dc_and_bad_inputs() {
        let x = vec![0.5; 2000];
        let out = resample_interleaved(&x, 2, 44100.0, 48000.0, ResampleQuality::SincFast).unwrap();
        assert_eq!(out.len(), 2178);
        assert!(out[256..out.len() - 256]
            .iter()
            .all(|v| (*v - 0.5).abs() < 1e-5));
        assert!(
            resample_interleaved(&[f32::NAN], 1, 48000.0, 44100.0, ResampleQuality::Linear)
                .is_err()
        );
    }
}
