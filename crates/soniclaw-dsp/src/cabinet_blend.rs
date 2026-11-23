//! Offline alignment and blending of equal-rate mono cabinet impulse responses.
use crate::resample::ResampleQuality;
use serde::{Deserialize, Serialize};
use soniclaw_core::Error;

const MAX_IR: usize = 1_048_576;
const MAX_SHIFT: f64 = 4096.;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Alignment {
    /// Positive means the candidate arrived later and should be advanced.
    pub advance_frames: f64,
    pub invert_candidate: bool,
    /// Absolute normalized correlation at the selected integer lag.
    pub correlation: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blend {
    pub balance: f32,
    pub reference_gain: f32,
    pub candidate_gain: f32,
    pub advance_frames: f64,
    pub invert_candidate: bool,
    pub frames: usize,
}

fn validate(reference: &[f32], candidate: &[f32]) -> Result<(), Error> {
    if reference.is_empty()
        || candidate.is_empty()
        || reference.len() > MAX_IR
        || candidate.len() > MAX_IR
        || reference
            .iter()
            .chain(candidate)
            .any(|x| !x.is_finite() || x.abs() > 1000.)
    {
        return Err(Error::InvalidBuffer);
    }
    Ok(())
}

/// Compare up to the first 8192 frames of each impulse and refine the strongest lag.
pub fn estimate(
    reference: &[f32],
    candidate: &[f32],
    max_shift: usize,
) -> Result<Alignment, Error> {
    validate(reference, candidate)?;
    if max_shift > MAX_SHIFT as usize || reference.len().min(candidate.len()) < 32 {
        return Err(Error::InvalidParameter);
    }
    let a = &reference[..reference.len().min(8192)];
    let b = &candidate[..candidate.len().min(8192)];
    let min_overlap = (a.len().min(b.len()) / 4).max(32);
    let correlation = |lag: i32| {
        let (from_a, from_b) = if lag < 0 {
            ((-lag) as usize, 0)
        } else {
            (0, lag as usize)
        };
        let count = a
            .len()
            .saturating_sub(from_a)
            .min(b.len().saturating_sub(from_b));
        if count < min_overlap {
            return 0.;
        }
        let (mut aa, mut bb, mut ab) = (0., 0., 0.);
        for (&x, &y) in a[from_a..from_a + count]
            .iter()
            .zip(&b[from_b..from_b + count])
        {
            let (x, y) = (f64::from(x), f64::from(y));
            aa += x * x;
            bb += y * y;
            ab += x * y;
        }
        if aa * bb <= 1e-24 {
            0.
        } else {
            ab / (aa * bb).sqrt()
        }
    };
    let max = max_shift as i32;
    let (lag, score) = (-max..=max)
        .map(|i| (i, correlation(i)))
        .max_by(|a, b| {
            a.1.abs()
                .total_cmp(&b.1.abs())
                .then_with(|| b.0.abs().cmp(&a.0.abs()))
        })
        .expect("nonempty search");
    if score.abs() < 1e-8 {
        return Err(Error::InvalidBuffer);
    }
    let delta = if lag.abs() == max {
        0.
    } else {
        let (left, center, right) = (
            correlation(lag - 1).abs(),
            score.abs(),
            correlation(lag + 1).abs(),
        );
        let denominator = left - 2. * center + right;
        if denominator < -1e-9 {
            (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
        } else {
            0.
        }
    };
    Ok(Alignment {
        advance_frames: f64::from(lag) + delta,
        invert_candidate: score < 0.,
        correlation: score.abs().min(1.),
    })
}

/// Produce an IR with the reference timeline as its origin.
pub fn blend(reference: &[f32], candidate: &[f32], plan: Blend) -> Result<Vec<f32>, Error> {
    validate(reference, candidate)?;
    if !(0. ..=1.).contains(&plan.balance)
        || !(0. ..=4.).contains(&plan.reference_gain)
        || !(0. ..=4.).contains(&plan.candidate_gain)
        || !(-MAX_SHIFT..=MAX_SHIFT).contains(&plan.advance_frames)
        || !(1..=MAX_IR).contains(&plan.frames)
    {
        return Err(Error::InvalidParameter);
    }
    let gain_a = plan.reference_gain * (1. - plan.balance);
    let gain_b = plan.candidate_gain * plan.balance * if plan.invert_candidate { -1. } else { 1. };
    let sample_at = |position: f64| {
        let tap = |i: isize| {
            usize::try_from(i)
                .ok()
                .and_then(|j| candidate.get(j))
                .copied()
                .unwrap_or(0.)
        };
        if position.fract() == 0. {
            tap(position as isize)
        } else if position < -32. || position >= candidate.len() as f64 + 32. {
            0.
        } else {
            ResampleQuality::SincFast.interpolate(position + 32., 1., |i| tap(i - 32))
        }
    };
    Ok((0..plan.frames)
        .map(|i| {
            let a = reference.get(i).copied().unwrap_or(0.);
            let b = if gain_b == 0. {
                0.
            } else {
                sample_at(i as f64 + plan.advance_frames)
            };
            gain_a * a + gain_b * b
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_delay_and_polarity() {
        let mut a = [0.; 128];
        a[30] = 1.;
        a[34] = -0.3;
        a[46] = 0.15;
        let mut b = [0.; 128];
        for (i, x) in a[..116].iter().enumerate() {
            b[i + 12] = -0.6 * x;
        }
        let fit = estimate(&a, &b, 20).unwrap();
        assert!(
            (fit.advance_frames - 12.).abs() < 1e-9
                && fit.invert_candidate
                && fit.correlation > 0.999
        );
        let ir = blend(
            &a,
            &b,
            Blend {
                balance: 0.5,
                reference_gain: 1.,
                candidate_gain: 1. / 0.6,
                advance_frames: fit.advance_frames,
                invert_candidate: true,
                frames: 128,
            },
        )
        .unwrap();
        assert!(ir.iter().zip(a).all(|(a, b)| (a - b).abs() < 1e-6));
        assert!((estimate(&b, &a, 20).unwrap().advance_frames + 12.).abs() < 1e-9);
    }
}
