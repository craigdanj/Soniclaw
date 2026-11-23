//! Prepared monophonic pitch correction with scale snapping and aligned bypass.
use crate::{
    advanced::analysis::PitchDetector,
    streaming::{FormantSettings, PitchSettings, PitchShifter},
};
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    /// Absolute pitch classes, bit 0 is C.
    pub note_mask: u16,
    pub confidence: f64,
    pub strength: f32,
    pub retune_ms: f32,
    pub hysteresis_semitones: f64,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            note_mask: 0xfff,
            confidence: 0.92,
            strength: 1.,
            retune_ms: 30.,
            hysteresis_semitones: 0.12,
        }
    }
}
impl Controls {
    fn validate(self) -> Result<Self, Error> {
        if !(1..=0xfff).contains(&self.note_mask)
            || !(0.85..=1.).contains(&self.confidence)
            || !(0. ..=1.).contains(&self.strength)
            || !(0. ..=500.).contains(&self.retune_ms)
            || !(0. ..=0.5).contains(&self.hysteresis_semitones)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }

    /// Build a pitch-class mask from a chromatic root and semitone intervals.
    pub fn scale(root: u8, intervals: &[u8]) -> Result<u16, Error> {
        if root > 11 || intervals.is_empty() || intervals.iter().any(|&n| n > 11) {
            return Err(Error::InvalidParameter);
        }
        Ok(intervals
            .iter()
            .fold(0, |mask, &i| mask | (1 << ((root + i) % 12))))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub analysis_window: usize,
    pub analysis_hop: usize,
    pub minimum_hz: f64,
    pub maximum_hz: f64,
    pub shifter: PitchSettings,
    pub formants: Option<FormantSettings>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            analysis_window: 2048,
            analysis_hop: 256,
            minimum_hz: 80.,
            maximum_hz: 1000.,
            shifter: PitchSettings {
                transient_threshold: None,
                ..PitchSettings::default()
            },
            formants: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Estimate {
    pub detected_hz: Option<f64>,
    pub target_midi: Option<i32>,
    pub confidence: f64,
    pub applied_semitones: f32,
}
struct State {
    spec: ProcessSpec,
    detector: PitchDetector,
    ring: Vec<f32>,
    scratch: Vec<f32>,
    dry: Vec<f32>,
    cursor: usize,
    dry_cursor: usize,
    clock: usize,
    target: Option<i32>,
    estimate: Estimate,
    wet: f32,
    blend_coefficient: f32,
}
pub struct PitchCorrector {
    settings: Settings,
    controls: Controls,
    shifter: PitchShifter,
    state: Option<State>,
}
impl PitchCorrector {
    pub fn new(settings: Settings, controls: Controls) -> Result<Self, Error> {
        if !(512..=8192).contains(&settings.analysis_window)
            || !(64..=settings.analysis_window / 2).contains(&settings.analysis_hop)
        {
            return Err(Error::InvalidSpec);
        }
        let mut shifter = PitchShifter::new(
            0.,
            PitchSettings {
                mix: 1.,
                ..settings.shifter
            },
        )?;
        shifter.set_formants(settings.formants)?;
        Ok(Self {
            settings,
            controls: controls.validate()?,
            shifter,
            state: None,
        })
    }
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        self.controls = controls.validate()?;
        Ok(())
    }
    pub fn estimate(&self) -> Estimate {
        self.state
            .as_ref()
            .map_or(Estimate::default(), |s| s.estimate)
    }

    fn analyze(state: &mut State, controls: Controls, hop: usize) {
        let (before, after) = state.ring.split_at(state.cursor);
        state.scratch[..after.len()].copy_from_slice(after);
        state.scratch[after.len()..].copy_from_slice(before);
        let estimate = state.detector.estimate(&state.scratch);
        let voiced = estimate.filter(|v| v.confidence >= controls.confidence);
        let previous = state.estimate.applied_semitones;
        let desired = if let Some(result) = voiced {
            let pitch = 69. + 12. * (result.hz / 440.).log2();
            let nearest = nearest_note(
                pitch,
                controls.note_mask,
                state.target,
                controls.hysteresis_semitones,
            );
            state.target = Some(nearest);
            ((f64::from(nearest) - pitch) as f32 * controls.strength).clamp(-12., 12.)
        } else {
            state.target = None;
            0.
        };
        let alpha = if controls.retune_ms == 0. {
            1.
        } else {
            1. - (-(hop as f64) / (0.001 * f64::from(controls.retune_ms) * state.spec.sample_rate))
                .exp() as f32
        };
        state.estimate = Estimate {
            detected_hz: estimate.map(|e| e.hz),
            confidence: estimate.map_or(0., |e| e.confidence),
            target_midi: state.target,
            applied_semitones: previous + (desired - previous) * alpha,
        };
    }
}
fn nearest_note(note: f64, mask: u16, previous: Option<i32>, hold: f64) -> i32 {
    let center = note.round() as i32;
    let allowed = |key: i32| mask & (1 << key.rem_euclid(12)) != 0;
    let distance = |key: i32| (f64::from(key) - note).abs();
    let nearest = (center - 12..=center + 12)
        .filter(|&key| allowed(key))
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .expect("valid scale");
    previous
        .filter(|&key| allowed(key) && distance(key) <= distance(nearest) + hold)
        .unwrap_or(nearest)
}
impl AudioProcessor for PitchCorrector {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels != 1 {
            return Err(Error::InvalidSpec);
        }
        let width = self.settings.analysis_window;
        let detector = PitchDetector::new(
            spec.sample_rate,
            width,
            self.settings.minimum_hz,
            self.settings.maximum_hz,
        )?;
        self.shifter.prepare(spec)?;
        self.shifter.set_semitones(0.)?;
        self.state = Some(State {
            spec,
            detector,
            ring: vec![0.; width],
            scratch: vec![0.; width],
            dry: vec![0.; self.latency_frames()],
            cursor: 0,
            dry_cursor: 0,
            clock: 0,
            target: None,
            estimate: Estimate::default(),
            wet: 0.,
            blend_coefficient: (1. - (-1. / (0.008 * spec.sample_rate)).exp()) as f32,
        });
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        let Some(state) = &mut self.state else {
            buffer.clear();
            return;
        };
        if buffer.channels() != 1 || buffer.frames() > state.spec.max_block_frames {
            buffer.clear();
            return;
        }
        let hop = self.settings.analysis_hop;
        for sample in buffer.samples_mut() {
            let input = if sample.is_finite() {
                sample.clamp(-16., 16.)
            } else {
                0.
            };
            let dry = std::mem::replace(&mut state.dry[state.dry_cursor], input);
            state.dry_cursor = (state.dry_cursor + 1) % state.dry.len();
            state.ring[state.cursor] = input;
            state.cursor = (state.cursor + 1) % state.ring.len();
            state.clock = state.clock.saturating_add(1);
            if state.clock >= state.ring.len()
                && (state.clock - state.ring.len()).is_multiple_of(hop)
            {
                Self::analyze(state, self.controls, hop);
                self.shifter
                    .set_semitones(state.estimate.applied_semitones)
                    .expect("bounded shift");
            }
            let active = state.target.is_some() && self.controls.strength > 0.;
            let target = if active { 1. } else { 0. };
            state.wet += (target - state.wet) * state.blend_coefficient;
            *sample = input;
            self.shifter.process(
                &mut AudioBuffer::new(std::slice::from_mut(sample), 1).expect("mono sample"),
                context,
            );
            *sample = dry + (*sample - dry) * state.wet;
        }
    }
    fn reset(&mut self) {
        self.shifter.reset();
        self.shifter.set_semitones(0.).expect("zero shift");
        if let Some(s) = &mut self.state {
            s.ring.fill(0.);
            s.dry.fill(0.);
            s.cursor = 0;
            s.dry_cursor = 0;
            s.clock = 0;
            s.target = None;
            s.estimate = Estimate::default();
            s.wet = 0.;
        }
    }
    fn latency_frames(&self) -> usize {
        self.shifter.latency_frames()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scale_ties_hysteresis_and_bypass() {
        let mask = Controls::scale(0, &[0, 2, 4, 5, 7, 9, 11]).unwrap();
        assert_eq!(nearest_note(61.6, mask, None, 0.), 62);
        assert_eq!(nearest_note(61.05, mask, Some(60), 0.15), 60);
        let mut dsp = PitchCorrector::new(
            Settings::default(),
            Controls {
                strength: 0.,
                ..Controls::default()
            },
        )
        .unwrap();
        dsp.prepare(ProcessSpec {
            sample_rate: 48_000.,
            channels: 1,
            max_block_frames: 2048,
        })
        .unwrap();
        let mut x = [0.; 2048];
        x[12] = 0.6;
        let a = allocation_counter::measure(|| {
            dsp.process(
                &mut AudioBuffer::new(&mut x, 1).unwrap(),
                &super::super::advanced::context(),
            )
        });
        assert_eq!(a.count_total, 0);
        assert_eq!(x[12 + dsp.latency_frames()], 0.6);
        assert_eq!(x.iter().filter(|&&y| y != 0.).count(), 1);
    }
}
