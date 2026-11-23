//! Offline compilation of selected recorded cycles to a harmonic-mipmapped bank.
use crate::{
    advanced::{
        spectral::{Complex32, FftProcessor},
        wavetable::WavetableBank,
    },
    resample::ResampleQuality,
};
use soniclaw_core::Error;
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub size: usize,
    pub repair_seams: bool,
    pub align_phase: bool,
    pub normalize: bool,
}
pub struct AuthoredTable {
    pub bank: WavetableBank,
    pub cycles: Vec<Vec<f32>>,
    pub phase_shifts: Vec<usize>,
}

pub fn compile(
    pcm: &[f32],
    regions: &[Range<usize>],
    settings: Settings,
) -> Result<AuthoredTable, Error> {
    let n = settings.size;
    if regions.is_empty() || regions.len() > 16 || !n.is_power_of_two() || !(64..=4096).contains(&n)
    {
        return Err(Error::InvalidParameter);
    }
    let mut cycles: Vec<Vec<f32>> = Vec::with_capacity(regions.len());
    let mut shifts = Vec::with_capacity(regions.len());
    let mut fft = FftProcessor::new(n)?;
    let mut reference = Vec::new();
    for r in regions {
        let cycle = pcm.get(r.clone()).ok_or(Error::InvalidBuffer)?;
        if !(16..=n * 32).contains(&cycle.len())
            || cycle.iter().any(|x| !x.is_finite() || x.abs() > 16.)
        {
            return Err(Error::InvalidBuffer);
        }
        let mut source = cycle.to_vec();
        if settings.repair_seams {
            let last = source.len() - 1;
            let slope = 0.5 * ((source[1] - source[0]) + (source[last] - source[last - 1]));
            let trend = source[last] - source[0] + slope;
            for (i, s) in source.iter_mut().enumerate() {
                *s -= trend * (i as f32 / (last + 1) as f32 - 0.5);
            }
        }
        let step = source.len() as f64 / n as f64;
        let mut frame: Vec<f32> = (0..n)
            .map(|i| {
                ResampleQuality::SincFast.interpolate(i as f64 * step, step, |j| {
                    source[j.rem_euclid(source.len() as isize) as usize]
                })
            })
            .collect();
        let mean = frame.iter().sum::<f32>() / n as f32;
        let mut peak = 0f32;
        for x in &mut frame {
            *x -= mean;
            peak = peak.max(x.abs());
        }
        if peak < 1e-8 {
            return Err(Error::InvalidBuffer);
        }
        if settings.normalize {
            for x in &mut frame {
                *x /= peak;
            }
        }
        let mut shift = 0;
        if settings.align_phase {
            let mut spectrum: Vec<_> = frame.iter().map(|&x| Complex32::new(x, 0.)).collect();
            fft.forward(&mut spectrum)?;
            if reference.is_empty() {
                reference = spectrum;
            } else {
                for (x, a) in spectrum.iter_mut().zip(&reference) {
                    *x *= a.conj();
                }
                fft.inverse(&mut spectrum)?;
                let best = spectrum
                    .iter()
                    .enumerate()
                    .max_by(|(i, a), (j, b)| a.re.total_cmp(&b.re).then(j.cmp(i)))
                    .ok_or(Error::InvalidBuffer)?;
                shift = best.0;
                frame.rotate_left(shift);
            }
        }
        shifts.push(shift);
        cycles.push(frame);
    }
    let bank = WavetableBank::new(cycles.clone())?;
    Ok(AuthoredTable {
        bank,
        cycles,
        phase_shifts: shifts,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_circular_phase_offset() {
        let a: Vec<_> = (0..128)
            .map(|i| (std::f32::consts::TAU * i as f32 / 128.).sin())
            .collect();
        let mut b = a.clone();
        b.rotate_right(31);
        let data = [a, b].concat();
        let t = compile(
            &data,
            &[0..128, 128..256],
            Settings {
                size: 128,
                repair_seams: false,
                align_phase: true,
                normalize: true,
            },
        )
        .unwrap();
        assert_eq!(t.phase_shifts, [0, 31]);
        assert!(t.cycles[0]
            .iter()
            .zip(&t.cycles[1])
            .all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
