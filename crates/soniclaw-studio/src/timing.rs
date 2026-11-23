//! Explicit loopback calibration and nondestructive recording placement compensation.
use crate::project::ProjectError;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub schema: u32,
    pub sample_rate: u32,
    pub audio_offset_frames: i32,
    pub midi_offset_frames: i32,
    pub label: String,
}
impl Calibration {
    pub fn new(rate: u32) -> Self {
        Self {
            schema: 1,
            sample_rate: rate,
            audio_offset_frames: 0,
            midi_offset_frames: 0,
            label: "Manual calibration".into(),
        }
    }
    pub fn validate(&self, rate: u32) -> Result<(), ProjectError> {
        if self.schema != 1
            || self.sample_rate != rate
            || !(8000..=192000).contains(&rate)
            || self.audio_offset_frames.unsigned_abs() > rate
            || self.midi_offset_frames.unsigned_abs() > rate
            || self.label.len() > 256
        {
            return Err("invalid or stale recording calibration".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Measurement {
    pub offset_frames: i32,
    pub peak: f32,
    pub noise_rms: f64,
    pub peak_to_noise: f64,
}
/// Use a single electrical impulse, not music. Select the strongest returned peak in a bounded delay window.
pub fn measure_impulse(
    reference: &[f32],
    returned: &[f32],
    maximum_lag: usize,
) -> Result<Measurement, ProjectError> {
    if reference.is_empty()
        || reference.len() > 192000
        || returned.is_empty()
        || returned.len() > 384000
        || maximum_lag > 192000
        || reference.iter().chain(returned).any(|v| !v.is_finite())
    {
        return Err("invalid impulse calibration buffers".into());
    }
    let (at, &peak) = reference
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .ok_or("missing reference impulse")?;
    if peak.abs() < 0.01
        || reference
            .iter()
            .enumerate()
            .any(|(i, v)| i.abs_diff(at) > 2 && v.abs() > peak.abs() * 0.25)
    {
        return Err("reference must contain a single isolated impulse".into());
    }
    let end = at
        .saturating_add(maximum_lag)
        .saturating_add(1)
        .min(returned.len());
    if at >= end {
        return Err("returned impulse window is empty".into());
    }
    let (relative, &found) = returned[at..end]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .ok_or("missing return")?;
    let position = at + relative;
    let mut sum = 0.;
    let mut n = 0;
    for (i, &v) in returned.iter().enumerate() {
        if i.abs_diff(position) > 8 {
            sum += (v as f64).powi(2);
            n += 1;
        }
    }
    let noise = (sum / n.max(1) as f64).sqrt();
    let ratio = found.abs() as f64 / noise.max(1e-9);
    if found.abs() < 0.001 || ratio < 8. {
        return Err("returned impulse is missing or too noisy".into());
    }
    Ok(Measurement {
        offset_frames: relative as i32,
        peak: found,
        noise_rms: noise,
        peak_to_noise: ratio,
    })
}
pub fn measure_wav(
    reference: &Path,
    returned: &Path,
    rate: u32,
) -> Result<Measurement, ProjectError> {
    fn load(path: &Path, rate: u32, limit: usize) -> Result<Vec<f32>, ProjectError> {
        let r = hound::WavReader::open(path)?;
        let s = r.spec();
        if s.channels != 1
            || s.sample_rate != rate
            || s.bits_per_sample != 32
            || s.sample_format != hound::SampleFormat::Float
            || r.len() as usize > limit
        {
            return Err("calibration WAV must be bounded mono float32 at project rate".into());
        }
        Ok(r.into_samples::<f32>().collect::<Result<_, _>>()?)
    }
    measure_impulse(
        &load(reference, rate, 192000)?,
        &load(returned, rate, 384000)?,
        rate as usize,
    )
}
/// Positive offset means captured audio arrives late. Return timeline position and required source trim.
pub fn placement(start: u64, offset: i32) -> Result<(u64, u64), ProjectError> {
    let corrected = start as i128 - offset as i128;
    if corrected < 0 {
        Ok((0, u64::try_from(-corrected)?))
    } else {
        Ok((u64::try_from(corrected)?, 0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impulse_polarity_noise_and_zero_boundary() {
        let mut r = vec![0.; 1000];
        r[10] = 1.;
        let mut input = vec![0.; 2000];
        input[147] = -0.5;
        assert_eq!(
            measure_impulse(&r, &input, 1000).unwrap().offset_frames,
            137
        );
        assert_eq!(placement(100, 137).unwrap(), (0, 37));
        assert_eq!(placement(100, -10).unwrap(), (110, 0));
        input.fill(0.);
        assert!(measure_impulse(&r, &input, 1000).is_err());
        let c = Calibration::new(48000);
        assert!(c.validate(44100).is_err());
    }
}
