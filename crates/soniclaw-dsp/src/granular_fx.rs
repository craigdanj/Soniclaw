//! Prepared tempo-synced granular processing of rolling mono or stereo audio.
use serde::{Deserialize, Serialize};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub grain_ms: f32,
    pub delay_ms: f32,
    pub interval_beats: f64,
    pub pitch_semitones: f32,
    pub mix: f32,
    pub freeze: bool,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            grain_ms: 75.,
            delay_ms: 160.,
            interval_beats: 0.25,
            pitch_semitones: 0.,
            mix: 0.65,
            freeze: false,
        }
    }
}
impl Controls {
    fn validate(self) -> Result<Self, Error> {
        if !(5. ..=300.).contains(&self.grain_ms)
            || !(1. ..=3500.).contains(&self.delay_ms)
            || !(1. / 32. ..=4.).contains(&self.interval_beats)
            || !(-24. ..=24.).contains(&self.pitch_semitones)
            || !(0. ..=1.).contains(&self.mix)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(self)
    }
}
#[derive(Clone, Copy)]
struct Grain {
    read: f64,
    step: f64,
    length: usize,
    age: usize,
}
#[derive(Clone, Copy, Default, Debug, Serialize)]
pub struct Stats {
    pub launched: u64,
    pub dropped: u64,
}
struct State {
    spec: ProcessSpec,
    history: Vec<f32>,
    frames: usize,
    written: u64,
    grains: Vec<Option<Grain>>,
    phase: f64,
    previously_playing: bool,
    expected_frame: Option<u64>,
    stats: Stats,
}
impl State {
    fn clear(&mut self) {
        self.written = 0;
        self.grains.fill(None);
        self.phase = 1.;
        self.previously_playing = false;
        self.expected_frame = None;
        self.stats = Stats::default();
    }
    fn spawn(&mut self, controls: Controls, step: f64) {
        let length = (self.spec.sample_rate * f64::from(controls.grain_ms) * 0.001).ceil() as usize;
        let delay = (self.spec.sample_rate * f64::from(controls.delay_ms) * 0.001)
            .max(length as f64 * step + 2.);
        if delay > self.written.min(self.frames as u64) as f64 {
            self.stats.dropped = self.stats.dropped.saturating_add(1);
            return;
        }
        if let Some(slot) = self.grains.iter_mut().find(|x| x.is_none()) {
            *slot = Some(Grain {
                read: self.written as f64 - delay,
                step,
                length,
                age: 0,
            });
            self.stats.launched = self.stats.launched.saturating_add(1);
        } else {
            self.stats.dropped = self.stats.dropped.saturating_add(1);
        }
    }
}

pub struct LiveGranular {
    history_ms: f64,
    max_grains: usize,
    controls: Controls,
    state: Option<State>,
}
impl LiveGranular {
    pub fn new(history_ms: f64, max_grains: usize, controls: Controls) -> Result<Self, Error> {
        if !(100. ..=4000.).contains(&history_ms) || !(1..=64).contains(&max_grains) {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            history_ms,
            max_grains,
            controls: controls.validate()?,
            state: None,
        })
    }
    pub fn set_controls(&mut self, controls: Controls) -> Result<(), Error> {
        self.controls = controls.validate()?;
        Ok(())
    }
    pub fn stats(&self) -> Stats {
        self.state.as_ref().map_or(Stats::default(), |s| s.stats)
    }
    pub fn captured_frames(&self) -> usize {
        self.state
            .as_ref()
            .map_or(0, |s| s.written.min(s.frames as u64) as usize)
    }
}
impl AudioProcessor for LiveGranular {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if !(8000. ..=192_000.).contains(&spec.sample_rate)
            || spec.channels > 2
            || spec.max_block_frames > 8192
        {
            return Err(Error::InvalidSpec);
        }
        let frames = (spec.sample_rate * self.history_ms * 0.001).ceil() as usize;
        self.state = Some(State {
            spec,
            frames,
            history: vec![0.; frames * spec.channels],
            written: 0,
            grains: vec![None; self.max_grains],
            phase: 1.,
            previously_playing: false,
            expected_frame: None,
            stats: Stats::default(),
        });
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        let Some(state) = &mut self.state else {
            buffer.clear();
            return;
        };
        let channels = state.spec.channels;
        if buffer.channels() != channels || buffer.frames() > state.spec.max_block_frames {
            buffer.clear();
            return;
        }
        let clock = context.transport;
        if state.expected_frame.is_some_and(|end| {
            end != clock.sample_position && state.previously_playing && clock.playing
        }) {
            state.clear();
        }
        state.expected_frame = Some(clock.sample_position.saturating_add(buffer.frames() as u64));
        let playing = clock.playing && (20. ..=400.).contains(&clock.tempo);
        if playing && !state.previously_playing {
            state.phase = 1.;
        }
        state.previously_playing = playing;
        let controls = self.controls;
        let increment = clock.tempo / (state.spec.sample_rate * 60. * controls.interval_beats);
        let step = 2f64.powf(f64::from(controls.pitch_semitones) / 12.);
        for frame in buffer.samples_mut().chunks_exact_mut(channels) {
            for x in frame.iter_mut() {
                *x = if x.is_finite() {
                    x.clamp(-16., 16.)
                } else {
                    0.
                };
            }
            if !controls.freeze {
                let start = state.written as usize % state.frames * channels;
                state.history[start..start + channels].copy_from_slice(frame);
                state.written = state.written.saturating_add(1);
            }
            if playing {
                if state.phase >= 1. - 1e-10 {
                    state.spawn(controls, step);
                    state.phase = (state.phase - 1.).max(0.);
                }
                state.phase += increment;
            }
            let earliest = state.written.saturating_sub(state.frames as u64);
            let mut wet = [0.; 2];
            let mut total_gain = 0.;
            for active in &mut state.grains {
                let Some(grain) = active else {
                    continue;
                };
                let start = grain.read.floor() as u64;
                if start < earliest || start + 1 >= state.written {
                    *active = None;
                    continue;
                }
                let gain = (std::f32::consts::PI * grain.age as f32 / (grain.length - 1) as f32)
                    .sin()
                    .powi(2);
                let i = start as usize % state.frames * channels;
                let j = (start + 1) as usize % state.frames * channels;
                let fraction = grain.read.fract() as f32;
                for (c, value) in wet.iter_mut().take(channels).enumerate() {
                    let first = state.history[i + c];
                    *value += (first + (state.history[j + c] - first) * fraction) * gain;
                }
                total_gain += gain;
                grain.read += grain.step;
                grain.age += 1;
                if grain.age == grain.length {
                    *active = None;
                }
            }
            for (x, wet) in frame.iter_mut().zip(wet) {
                *x = *x * (1. - controls.mix) + wet * controls.mix / total_gain.max(1.);
            }
        }
    }
    fn reset(&mut self) {
        if let Some(state) = &mut self.state {
            state.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tempo_triggers_and_freeze_are_partition_independent() {
        let run = |block: usize| {
            let mut engine = LiveGranular::new(
                1000.,
                16,
                Controls {
                    grain_ms: 20.,
                    delay_ms: 30.,
                    interval_beats: 0.125,
                    mix: 1.,
                    ..Default::default()
                },
            )
            .unwrap();
            let spec = ProcessSpec {
                sample_rate: 8000.,
                channels: 1,
                max_block_frames: 512,
            };
            engine.prepare(spec).unwrap();
            let mut pcm: Vec<_> = (0..8000).map(|i| (i as f32 * 0.13).sin()).collect();
            for (begin, end, frozen) in [(0, 4000, false), (4000, 8000, true)] {
                engine
                    .set_controls(Controls {
                        freeze: frozen,
                        ..engine.controls
                    })
                    .unwrap();
                let mut pos = begin;
                for b in pcm[begin..end].chunks_mut(block) {
                    let c = ProcessContext {
                        events: &[],
                        transport: soniclaw_core::Transport {
                            playing: true,
                            tempo: 120.,
                            sample_position: pos as u64,
                        },
                    };
                    let a = allocation_counter::measure(|| {
                        engine.process(&mut AudioBuffer::new(b, 1).unwrap(), &c)
                    });
                    assert_eq!(a.count_total, 0);
                    pos += b.len();
                }
            }
            assert_eq!(engine.captured_frames(), 4000);
            assert_eq!(engine.stats().launched, 15);
            assert_eq!(engine.stats().dropped, 1);
            assert!(pcm[6000..].iter().any(|v| v.abs() > 0.1));
            pcm
        };
        assert_eq!(run(97), run(128));
    }
}
