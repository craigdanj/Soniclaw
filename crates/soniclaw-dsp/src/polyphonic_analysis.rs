//! Offline pitched note transcription using bounded harmonic peak selection.
use crate::advanced::spectral::{
    AnalysisWindow, RealFft, WindowNormalization, WindowSampling, WindowShape,
};
use soniclaw_core::Error;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub sample_rate: u32,
    pub window_frames: usize,
    pub hop_frames: usize,
    pub min_midi: u8,
    pub max_midi: u8,
    pub polyphony: usize,
    pub min_hops: usize,
    /// Fraction of the strongest window bin that a note's fundamental must meet.
    pub relative_threshold: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            window_frames: 8192,
            hop_frames: 480,
            min_midi: 36,
            max_midi: 92,
            polyphony: 6,
            min_hops: 3,
            relative_threshold: 0.09,
        }
    }
}
impl Settings {
    fn validate(self) -> Result<(), Error> {
        if !(8_000..=192_000).contains(&self.sample_rate)
            || !(1024..=32_768).contains(&self.window_frames)
            || !self.window_frames.is_power_of_two()
            || !(self.window_frames / 64..=self.window_frames / 2).contains(&self.hop_frames)
            || !(24..=112).contains(&self.min_midi)
            || !(self.min_midi..=112).contains(&self.max_midi)
            || !(1..=12).contains(&self.polyphony)
            || !(1..=16).contains(&self.min_hops)
            || !(0.01..=0.9).contains(&self.relative_threshold)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub midi: u8,
    pub start_frame: usize,
    pub end_frame: usize,
    /// Relative spectral support, from zero through one. An estimate rather than a probability.
    pub confidence: f32,
}

#[derive(Clone, Copy)]
struct Peak {
    bin: f32,
    amplitude: f32,
}
#[derive(Clone, Copy, Default)]
struct Track {
    first: usize,
    last: usize,
    hits: usize,
    support: f32,
}

const HARMONICS: usize = 7;

/// Offline analysis of finite mono samples. Returned note ranges are half-open.
pub fn transcribe(mono: &[f32], settings: Settings) -> Result<Vec<Note>, Error> {
    settings.validate()?;
    if mono.len() > 8_388_608 || mono.iter().any(|x| !x.is_finite() || x.abs() > 16.) {
        return Err(Error::InvalidBuffer);
    }
    let size = settings.window_frames;
    let half = size / 2;
    let mut fft = RealFft::new(size)?;
    let window = AnalysisWindow::new(
        size,
        WindowShape::Hann,
        WindowSampling::Periodic,
        WindowNormalization::None,
    )?;
    let mut input = vec![0.; size];
    let mut spectrum = vec![0.; fft.bins()];
    let mut peaks = Vec::with_capacity(spectrum.len() / 2);
    let mut consumed = Vec::with_capacity(spectrum.len() / 2);
    let mut tracks = [Track::default(); 128];
    let pitches: Vec<_> = (settings.min_midi..=settings.max_midi)
        .map(|note| {
            (
                usize::from(note),
                crate::midi_hz(note) * size as f32 / settings.sample_rate as f32,
            )
        })
        .filter(|&(_, bin)| (2. ..half as f32 - 1.).contains(&bin))
        .collect();
    if pitches.is_empty() {
        return Err(Error::InvalidSpec);
    }
    let mut notes = Vec::new();
    for frame in (0..mono.len()).step_by(settings.hop_frames) {
        let start = frame.saturating_sub(half);
        let end = (frame + half).min(mono.len());
        let offset = half.saturating_sub(frame);
        input.fill(0.);
        input[offset..offset + end - start].copy_from_slice(&mono[start..end]);
        fft.magnitudes(&input, Some(&window), &mut spectrum)?;
        let strongest = spectrum.iter().copied().fold(0., f32::max);
        peaks.clear();
        for (k, window) in spectrum.windows(3).enumerate() {
            let [a, b, c] = <[f32; 3]>::try_from(window).expect("three bins");
            if b < 0.0001f32.max(strongest * 0.012) || b <= a || b < c {
                continue;
            }
            let (left, mid, right) = (a.max(1e-12).ln(), b.ln(), c.max(1e-12).ln());
            let denominator = left + right - 2. * mid;
            let offset = if denominator.abs() > 1e-8 {
                (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
            } else {
                0.
            };
            peaks.push(Peak {
                bin: (k + 1) as f32 + offset,
                amplitude: b,
            });
        }
        consumed.clear();
        consumed.resize(peaks.len(), false);
        let mut matches = [0.; 128];
        for _ in 0..settings.polyphony {
            let Some((note, score, match_peaks)) = strongest_pitch(
                &pitches,
                &peaks,
                &consumed,
                (strongest * settings.relative_threshold).max(0.0003),
            ) else {
                break;
            };
            for i in match_peaks.into_iter().flatten() {
                consumed[i] = true;
            }
            matches[note] = (score / strongest.max(1e-12)).clamp(0.001, 1.);
        }
        for (midi, (&confidence, track)) in matches.iter().zip(&mut tracks).enumerate() {
            if confidence > 0. {
                if track.hits == 0 {
                    track.first = frame;
                }
                track.last = frame;
                track.hits += 1;
                track.support += confidence;
            } else if track.hits > 0 && frame - track.last >= 2 * settings.hop_frames {
                finish(&mut notes, track, midi, settings, mono.len());
            }
        }
    }
    for (midi, track) in tracks.iter_mut().enumerate() {
        finish(&mut notes, track, midi, settings, mono.len());
    }
    notes.sort_by_key(|note| (note.start_frame, note.midi));
    Ok(notes)
}

type PeakMatches = [Option<usize>; HARMONICS];

fn strongest_pitch(
    pitches: &[(usize, f32)],
    peaks: &[Peak],
    consumed: &[bool],
    threshold: f32,
) -> Option<(usize, f32, PeakMatches)> {
    pitches
        .iter()
        .filter_map(|&(midi, bin)| {
            let fundamental = best_peak(peaks, consumed, bin)?;
            if peaks[fundamental].amplitude < threshold {
                return None;
            }
            let mut indices = [None; HARMONICS];
            indices[0] = Some(fundamental);
            let mut score = peaks[fundamental].amplitude;
            for (i, index) in indices.iter_mut().enumerate().skip(1) {
                let harmonic = (i + 1) as f32;
                *index = best_peak(peaks, consumed, bin * harmonic);
                if let Some(p) = *index {
                    score += peaks[p].amplitude * (0.7 / harmonic.sqrt());
                }
            }
            Some((midi, score, indices))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn best_peak(peaks: &[Peak], consumed: &[bool], expected: f32) -> Option<usize> {
    let tolerance = (expected * 0.012).max(0.52);
    let first = peaks.partition_point(|p| p.bin < expected - tolerance);
    peaks[first..]
        .iter()
        .enumerate()
        .take_while(|(_, p)| p.bin <= expected + tolerance)
        .filter(|(i, _)| !consumed[first + i])
        .min_by(|(_, a), (_, b)| {
            (a.bin - expected)
                .abs()
                .total_cmp(&(b.bin - expected).abs())
        })
        .map(|(i, _)| first + i)
}

fn finish(
    notes: &mut Vec<Note>,
    track: &mut Track,
    midi: usize,
    settings: Settings,
    input_frames: usize,
) {
    if track.hits >= settings.min_hops {
        let margin = settings.hop_frames / 2;
        notes.push(Note {
            midi: midi as u8,
            start_frame: track.first.saturating_sub(margin),
            end_frame: (track.last + margin + 1).min(input_frames),
            confidence: track.support / track.hits as f32,
        });
    }
    *track = Track::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_pitches_with_harmonics_resolve_as_notes() {
        let mut audio = vec![0.; 48_000];
        for (i, sample) in audio.iter_mut().enumerate().take(33_000).skip(8000) {
            let time = i as f32 / 48_000.;
            for midi in [57, 64, 71] {
                let phase = std::f32::consts::TAU * crate::midi_hz(midi) * time;
                *sample +=
                    0.13 * (phase.sin() + 0.35 * (2. * phase).sin() + 0.18 * (3. * phase).sin());
            }
        }
        let events = transcribe(&audio, Settings::default()).unwrap();
        for key in [57, 64, 71] {
            let e = events
                .iter()
                .find(|n| n.midi == key && n.start_frame < 12_500)
                .unwrap();
            assert!(e.start_frame.abs_diff(8000) <= 4096 && e.end_frame.abs_diff(33_000) <= 4096);
            assert!(e.confidence > 0.5);
        }
        assert!(events
            .iter()
            .filter(|n| ![57, 64, 71].contains(&n.midi))
            .all(|n| n.end_frame - n.start_frame < 6000));
        assert!(transcribe(&[0.; 4096], Settings::default())
            .unwrap()
            .is_empty());
    }
}
