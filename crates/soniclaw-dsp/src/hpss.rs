//! Offline stereo-linked harmonic and percussive separation.
use crate::advanced::spectral::{
    AnalysisWindow, Complex32, RealFft, WindowNormalization, WindowSampling, WindowShape,
};
use soniclaw_core::Error;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub window_frames: usize,
    pub hop_frames: usize,
    pub time_radius: usize,
    pub frequency_radius: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            window_frames: 1024,
            hop_frames: 256,
            time_radius: 8,
            frequency_radius: 8,
        }
    }
}

pub struct Stems {
    pub harmonic: Vec<f32>,
    pub percussive: Vec<f32>,
}

fn median(data: impl Iterator<Item = f32>, scratch: &mut [f32; 33]) -> f32 {
    let mut count = 0;
    for (slot, value) in scratch.iter_mut().zip(data) {
        *slot = value;
        count += 1;
    }
    *scratch[..count]
        .select_nth_unstable_by(count / 2, f32::total_cmp)
        .1
}

fn frame_range(start: isize, window: usize, total: usize) -> std::ops::Range<usize> {
    start.saturating_neg().max(0).min(window as isize) as usize
        ..(total as isize - start).clamp(0, window as isize) as usize
}

/// Use one mask for all channels to preserve the stereo image. Stems use
/// the original timeline and channel layout.
pub fn separate(input: &[f32], channels: usize, settings: Settings) -> Result<Stems, Error> {
    if !(1..=2).contains(&channels)
        || input.is_empty()
        || !input.len().is_multiple_of(channels)
        || input.len() / channels > 8_388_608
        || input.iter().any(|v| !v.is_finite() || v.abs() > 16.)
    {
        return Err(Error::InvalidBuffer);
    }
    let width = settings.window_frames;
    let hop = settings.hop_frames;
    if !(256..=4096).contains(&width)
        || !width.is_power_of_two()
        || !(width / 8..=width / 2).contains(&hop)
        || !(1..=16).contains(&settings.time_radius)
        || !(1..=16).contains(&settings.frequency_radius)
    {
        return Err(Error::InvalidSpec);
    }
    let total = input.len() / channels;
    let windows = (total + width / 2).div_ceil(hop);
    let bins = width / 2 + 1;
    if windows * bins > 8_388_608 {
        return Err(Error::InvalidBuffer);
    }
    let mut fft = RealFft::new(width)?;
    let window: Vec<_> = AnalysisWindow::new(
        width,
        WindowShape::Hann,
        WindowSampling::Periodic,
        WindowNormalization::None,
    )?
    .coefficients()
    .iter()
    .map(|&x| x.sqrt())
    .collect();
    let mut scratch = vec![0.; width];
    let mut spectrum = vec![Complex32::default(); bins];
    let mut power = vec![0.; windows * bins];
    let time_origin = |frame: usize| (frame * hop) as isize - width as isize / 2;
    let load = |buf: &mut [f32], start: isize, ch: usize| {
        buf.fill(0.);
        for i in frame_range(start, width, total) {
            let source = (start + i as isize) as usize;
            buf[i] = input[source * channels + ch] * window[i];
        }
    };
    for (f, frame_power) in power.chunks_exact_mut(bins).enumerate() {
        for ch in 0..channels {
            load(&mut scratch, time_origin(f), ch);
            fft.forward(&scratch, &mut spectrum)?;
            for (sum, value) in frame_power.iter_mut().zip(&spectrum) {
                *sum += value.norm_sqr() / channels as f32;
            }
        }
    }
    let mut mask = vec![0.; bins];
    let mut harmonic = vec![0.; input.len()];
    let mut gain = vec![0.; total];
    let mut median_work = [0.; 33];
    for frame in 0..windows {
        let frequency_power = &power[frame * bins..(frame + 1) * bins];
        let first = frame.saturating_sub(settings.time_radius);
        let last = (frame + settings.time_radius).min(windows - 1);
        for (k, value) in mask.iter_mut().enumerate() {
            let h = median(
                (first..=last).map(|i| power[i * bins + k]),
                &mut median_work,
            );
            let p = median(
                frequency_power[k.saturating_sub(settings.frequency_radius)
                    ..=(k + settings.frequency_radius).min(bins - 1)]
                    .iter()
                    .copied(),
                &mut median_work,
            );
            *value = if h + p > 1e-18 { h / (h + p) } else { 0.5 };
        }
        let start = time_origin(frame);
        let valid = frame_range(start, width, total);
        for i in valid.clone() {
            gain[(start + i as isize) as usize] += window[i] * window[i];
        }
        for ch in 0..channels {
            load(&mut scratch, start, ch);
            fft.forward(&scratch, &mut spectrum)?;
            for (value, gain) in spectrum.iter_mut().zip(&mask) {
                *value *= *gain;
            }
            fft.inverse(&spectrum, &mut scratch)?;
            for i in valid.clone() {
                harmonic[(start + i as isize) as usize * channels + ch] += scratch[i] * window[i];
            }
        }
    }
    for (frame, g) in harmonic.chunks_exact_mut(channels).zip(gain) {
        for x in frame {
            *x /= g.max(1e-10);
        }
    }
    let percussive = input
        .iter()
        .zip(&harmonic)
        .map(|(source, tonal)| source - tonal)
        .collect();
    Ok(Stems {
        harmonic,
        percussive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_combined_tone_and_burst_with_stereo_coherence() {
        let mut signal = vec![0.; 12_288 * 2];
        for (i, pair) in signal.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let tone = (std::f32::consts::TAU * 750. * i as f32 / 48_000.).sin() * 0.18;
            let burst = if (6200..=6203).contains(&i) { 0.9 } else { 0. };
            *pair = [tone + burst, 0.5 * (tone + burst)];
        }
        let stems = separate(&signal, 2, Settings::default()).unwrap();
        let mut tone_error = 0.;
        for (i, &v) in signal.iter().enumerate() {
            assert!((v - stems.harmonic[i] - stems.percussive[i]).abs() < 2e-7);
            if i % 2 == 0 {
                assert!((stems.harmonic[i] * 0.5 - stems.harmonic[i + 1]).abs() < 1e-8);
            }
            if (4000..8000).contains(&i) {
                tone_error += stems.percussive[i].abs();
            }
        }
        assert!(tone_error < 2.);
        assert!(stems.percussive[12_400] > 0.6);
    }
}
