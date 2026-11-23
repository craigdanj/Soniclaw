//! A fixed pool of layered voices. Assets are shared and never replaced in a callback.
use crate::{
    advanced::wavetable::{WavetableBank, WavetableOscillator},
    sample_playback::{self, RegionPlayer},
    Adsr, AdsrSettings, Sample,
};
use serde::{Deserialize, Serialize};
use soniclaw_core::{
    validate_events, AudioBuffer, AudioProcessor, Error, MidiMessage, ProcessContext, ProcessSpec,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub schema: u32,
    pub voices: usize,
    pub wavetable_gain: f32,
    pub sample_gain: f32,
    pub morph: f32,
    pub cutoff_hz: f32,
    pub lfo_hz: f32,
    pub morph_depth: f32,
    pub envelope: AdsrSettings,
}

struct Voice {
    oscillator: WavetableOscillator,
    sample: RegionPlayer,
    envelope: Adsr,
    note: Option<(u8, u8)>,
    age: u64,
    frequency: f64,
    sample_speed: f64,
    sample_active: bool,
    level: f32,
    phase: f32,
    filtered: f32,
    previous: f32,
    steal: f32,
    fade: usize,
}
pub struct HybridSynth {
    patch: Patch,
    bank: Arc<WavetableBank>,
    asset: Arc<Sample>,
    voices: Vec<Voice>,
    spec: Option<ProcessSpec>,
    clock: u64,
    filter_coefficient: f32,
    lfo_increment: f32,
}
impl HybridSynth {
    pub fn new(patch: Patch, bank: Arc<WavetableBank>, asset: Arc<Sample>) -> Result<Self, Error> {
        if patch.schema != 1
            || !(1..=64).contains(&patch.voices)
            || [
                patch.wavetable_gain,
                patch.sample_gain,
                patch.morph,
                patch.morph_depth,
            ]
            .iter()
            .any(|x| !(0. ..=1.).contains(x))
            || !(20. ..=20000.).contains(&patch.cutoff_hz)
            || !(0. ..=30.).contains(&patch.lfo_hz)
            || asset.channels() != 1
            || asset.frames() > 16_777_216
        {
            return Err(Error::InvalidParameter);
        }
        let mut a = Adsr::default();
        a.configure(patch.envelope)?;
        Ok(Self {
            patch,
            bank,
            asset,
            voices: Vec::new(),
            spec: None,
            clock: 0,
            filter_coefficient: 0.,
            lfo_increment: 0.,
        })
    }
    pub fn patch(&self) -> Patch {
        self.patch
    }
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.envelope.is_active())
            .count()
    }
    fn event(&mut self, event: MidiMessage, rate: f32) {
        match event {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => {
                let index = self
                    .voices
                    .iter()
                    .position(|v| !v.envelope.is_active())
                    .unwrap_or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| v.age)
                            .expect("prepared voices")
                            .0
                    });
                let v = &mut self.voices[index];
                v.steal = v.previous;
                v.fade = 32;
                self.clock = self.clock.saturating_add(1);
                v.age = self.clock;
                v.note = Some((channel, note));
                v.level = f32::from(velocity) / 127.;
                v.frequency = f64::from(crate::midi_hz(note));
                v.sample_speed =
                    2f64.powf((f64::from(note) - f64::from(self.asset.root_note())) / 12.);
                v.sample_active =
                    v.sample.set_speed(v.sample_speed).is_ok() && v.sample.start().is_ok();
                v.oscillator.reset();
                v.envelope.reset();
                v.envelope.note_on();
                v.phase = 0.;
                v.filtered = 0.;
            }
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => {
                for v in &mut self.voices {
                    if v.note == Some((channel, note)) {
                        v.envelope.note_off(rate);
                        v.sample.release();
                        v.note = None;
                    }
                }
            }
            MidiMessage::AllNotesOff => {
                for v in &mut self.voices {
                    v.envelope.note_off(rate);
                    v.sample.release();
                    v.note = None;
                }
            }
            _ => {}
        }
    }
}
impl AudioProcessor for HybridSynth {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if !(8000. ..=192000.).contains(&s.sample_rate) || s.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        let mut voices = Vec::with_capacity(self.patch.voices);
        for _ in 0..self.patch.voices {
            let mut player = RegionPlayer::new(
                self.asset.clone(),
                sample_playback::Config {
                    schema: 1,
                    start: 0,
                    end: self.asset.frames(),
                    reverse: false,
                    speed: 1.,
                    gain: 1.,
                    interpolation: sample_playback::Interpolation::Linear,
                    loop_region: None,
                },
            )?;
            player.prepare(ProcessSpec { channels: 1, ..s })?;
            let mut envelope = Adsr::default();
            envelope.configure(self.patch.envelope)?;
            voices.push(Voice {
                oscillator: WavetableOscillator::new(self.bank.clone(), s.sample_rate)?,
                sample: player,
                envelope,
                note: None,
                age: 0,
                frequency: 0.,
                sample_speed: 1.,
                sample_active: false,
                level: 0.,
                phase: 0.,
                filtered: 0.,
                previous: 0.,
                steal: 0.,
                fade: 0,
            });
        }
        self.voices = voices;
        self.filter_coefficient = (1.
            - (-std::f64::consts::TAU * f64::from(self.patch.cutoff_hz) / s.sample_rate).exp())
            as f32;
        self.lfo_increment = (f64::from(self.patch.lfo_hz) / s.sample_rate) as f32;
        self.spec = Some(s);
        self.clock = 0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != s.channels
            || b.frames() > s.max_block_frames
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        let rate = s.sample_rate as f32;
        let mut events = ctx.events.iter().peekable();
        for (i, frame) in b.samples_mut().chunks_exact_mut(s.channels).enumerate() {
            while events.peek().is_some_and(|e| e.frame == i) {
                self.event(events.next().expect("peeked").message, rate);
            }
            let mut y = 0.;
            for v in &mut self.voices {
                let mut mixed = 0.;
                if v.envelope.is_active() {
                    let lfo = (std::f32::consts::TAU * v.phase).sin();
                    v.phase = (v.phase + self.lfo_increment).fract();
                    let morph = (self.patch.morph + self.patch.morph_depth * lfo).clamp(0., 1.);
                    let osc = v.oscillator.next_sample(v.frequency, morph, 0., 0.5, 1.);
                    let mut sample = [0.];
                    if v.sample_active {
                        v.sample.next_frame(&mut sample);
                    }
                    let value =
                        osc * self.patch.wavetable_gain + sample[0] * self.patch.sample_gain;
                    v.filtered += self.filter_coefficient * (value - v.filtered);
                    mixed = v.filtered * v.envelope.next_sample(rate) * v.level;
                }
                if v.fade > 0 {
                    let w = v.fade as f32 / 32.;
                    mixed = (1. - w) * mixed + w * v.steal;
                    v.fade -= 1;
                }
                v.previous = mixed;
                y += mixed;
            }
            frame.fill(y);
        }
    }
    fn reset(&mut self) {
        self.clock = 0;
        for v in &mut self.voices {
            v.note = None;
            v.envelope.reset();
            v.oscillator.reset();
            v.sample.reset();
            v.previous = 0.;
            v.fade = 0;
            v.filtered = 0.;
            v.sample_active = false;
        }
    }
}
