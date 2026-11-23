//! Fixed-voice MPE/CLAP-note instrument with per-voice ADSR and a timbre low-pass.
use crate::*;
#[derive(Clone, Copy, Default)]
struct Voice {
    oscillator: Oscillator,
    envelope: Adsr,
    note: u8,
    channel: u8,
    id: i32,
    velocity: f32,
    expression: soniclaw_core::VoiceExpression,
    held: bool,
    age: u64,
    filter: f32,
    modulation: crate::voice_mod::VoiceModState,
}
pub struct ExpressiveSynth<const N: usize = 32> {
    tuning: Option<std::sync::Arc<soniclaw_tuning::Tuning>>,
    voices: [Voice; N],
    channels: ExpressionChannels,
    sustain: [bool; 16],
    sr: f32,
    age: u64,
    pub voice_modulation: crate::voice_mod::VoiceModulationConfig,
    pub cutoff_hz: Option<f32>,
    pub envelope: AdsrSettings,
    pub waveform: Waveform,
    pub pressure_gain: f32,
    pub timbre_depth: f32,
}
impl<const N: usize> Default for ExpressiveSynth<N> {
    fn default() -> Self {
        Self {
            tuning: None,
            voices: [Voice::default(); N],
            channels: ExpressionChannels::default(),
            sustain: [false; 16],
            sr: 48000.0,
            age: 0,
            voice_modulation: Default::default(),
            cutoff_hz: None,
            envelope: AdsrSettings::default(),
            waveform: Waveform::Saw,
            pressure_gain: 0.5,
            timbre_depth: 1.0,
        }
    }
}
impl<const N: usize> ExpressiveSynth<N> {
    pub fn new(waveform: Waveform, envelope: AdsrSettings) -> Result<Self, Error> {
        Adsr::default().configure(envelope)?;
        Ok(Self {
            waveform,
            envelope,
            ..Self::default()
        })
    }
    /// Replace on the control thread while stopped; clears voices. Existing untuned behavior is preserved with None.
    pub fn set_tuning(
        &mut self,
        tuning: Option<std::sync::Arc<soniclaw_tuning::Tuning>>,
    ) -> Result<(), Error> {
        if let Some(t) = &tuning {
            t.validate()?;
        }
        self.reset();
        self.tuning = tuning;
        Ok(())
    }
    pub fn set_mpe_zone(&mut self, zone: Option<MpeZone>) -> Result<(), Error> {
        self.channels.zone = zone.map(MpeZone::validate).transpose()?;
        Ok(())
    }
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.envelope.is_active())
            .count()
    }
    fn start(&mut self, channel: u8, note: u8, velocity: f32, id: i32) {
        if self
            .tuning
            .as_ref()
            .is_some_and(|t| t.frequency(note).is_none())
        {
            return;
        }
        if N == 0
            || channel > 15
            || note > 127
            || !velocity.is_finite()
            || !(0.0..=1.0).contains(&velocity)
            || id < -1
        {
            return;
        }
        let index = self
            .voices
            .iter()
            .position(|v| !v.envelope.is_active())
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|x| x.0)
                    .unwrap_or(0)
            });
        self.age = self.age.wrapping_add(1);
        let v = &mut self.voices[index];
        *v = Voice {
            channel,
            note,
            id,
            velocity,
            held: true,
            age: self.age,
            ..Voice::default()
        };
        let _ = v.envelope.configure(self.envelope);
        v.envelope.note_on();
        v.modulation.start(&self.voice_modulation, self.age as u32);
    }
    fn release(&mut self, target: NoteTarget, choke: bool) {
        if !target.valid() {
            return;
        }
        for v in &mut self.voices {
            if target.matches(v.id, v.channel, v.note) {
                v.held = false;
                if choke {
                    v.envelope.reset();
                } else if !self.sustain[v.channel as usize] {
                    v.envelope.note_off(self.sr);
                    v.modulation.release(self.sr);
                }
            }
        }
    }
    fn message(&mut self, m: MidiMessage) {
        if self.channels.update(m) {
            return;
        }
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => self.start(channel, note, velocity as f32 / 127.0, -1),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity,
                note_id,
            } if velocity > 0.0 => self.start(channel, note, velocity, note_id),
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => self.release(
                NoteTarget {
                    channel: channel as i16,
                    key: note as i16,
                    note_id: -1,
                },
                false,
            ),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity: 0.0,
                note_id,
            } => self.release(
                NoteTarget {
                    note_id,
                    channel: channel as i16,
                    key: note as i16,
                },
                false,
            ),
            MidiMessage::NoteOffId { target } => self.release(target, false),
            MidiMessage::NoteChoke { target } => self.release(target, true),
            MidiMessage::NoteExpression {
                target,
                expression,
                value,
            } => {
                for v in &mut self.voices {
                    if target.valid() && target.matches(v.id, v.channel, v.note) {
                        v.expression.set(expression, value);
                    }
                }
            }
            MidiMessage::PolyPressure {
                channel,
                note,
                value,
            } => {
                for v in &mut self.voices {
                    if v.channel == channel && v.note == note && value <= 127 {
                        v.expression.pressure = value as f32 / 127.0;
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 64,
                value,
            } if channel < 16 => {
                self.sustain[channel as usize] = value >= 64;
                if value < 64 {
                    for v in &mut self.voices {
                        if v.channel == channel && !v.held {
                            v.envelope.note_off(self.sr);
                            v.modulation.release(self.sr);
                        }
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 120,
                ..
            } => self.release(
                NoteTarget {
                    channel: channel as i16,
                    key: -1,
                    note_id: -1,
                },
                true,
            ),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => self.release(
                NoteTarget {
                    channel: channel as i16,
                    key: -1,
                    note_id: -1,
                },
                false,
            ),
            MidiMessage::AllNotesOff => self.reset(),
            _ => {}
        }
    }
}
impl<const N: usize> AudioProcessor for ExpressiveSynth<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.voice_modulation.validate()?;
        if self
            .cutoff_hz
            .is_some_and(|x| !x.is_finite() || !(20.0..=20000.0).contains(&x))
        {
            return Err(Error::InvalidParameter);
        }
        if !self.pressure_gain.is_finite()
            || !(0.0..=2.0).contains(&self.pressure_gain)
            || !self.timbre_depth.is_finite()
            || !(0.0..=1.0).contains(&self.timbre_depth)
        {
            return Err(Error::InvalidParameter);
        }
        Adsr::default().configure(self.envelope)?;
        self.sr = s.sample_rate as f32;
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let ch = b.channels();
        let mut e = 0;
        for (index, frame) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            while e < ctx.events.len() && ctx.events[e].frame == index {
                self.message(ctx.events[e].message);
                e += 1;
            }
            let mut left = 0.0;
            let mut right = 0.0;
            for v in &mut self.voices {
                if !v.envelope.is_active() {
                    continue;
                }
                let (pitch, pressure, timbre) = self.channels.values(v.channel);
                let offsets = v.modulation.tick(
                    &self.voice_modulation,
                    self.sr,
                    v.note,
                    v.velocity,
                    pressure.max(v.expression.pressure),
                    (timbre + v.expression.timbre - 0.5).clamp(0.0, 1.0),
                );
                let frequency = self
                    .tuning
                    .as_ref()
                    .and_then(|t| t.frequency(v.note))
                    .map(|f| f as f32)
                    .unwrap_or_else(|| midi_hz(v.note))
                    * 2.0_f32.powf((pitch + v.expression.tuning + offsets.pitch_semitones) / 12.0);
                let x = v.oscillator.next_sample(frequency, self.sr, self.waveform);
                let brightness = (timbre + v.expression.timbre - 0.5).clamp(0.0, 1.0);
                let cutoff = (self
                    .cutoff_hz
                    .unwrap_or_else(|| 200.0 * 100.0_f32.powf(brightness))
                    * 2.0_f32.powf(offsets.cutoff_octaves))
                .clamp(20.0_f32.min(self.sr * 0.45), self.sr * 0.45);
                let pole = (-std::f32::consts::TAU * cutoff / self.sr).exp();
                v.filter = (1.0 - pole) * x + pole * v.filter;
                let x = (x * (1.0 - self.timbre_depth) + v.filter * self.timbre_depth)
                    * v.envelope.next_sample(self.sr)
                    * v.velocity
                    * (1.0 + self.pressure_gain * pressure.max(v.expression.pressure))
                    * v.expression.volume
                    * offsets.level
                    * 0.12;
                let pan = (v.expression.pan + offsets.pan).clamp(0.0, 1.0);
                left += x * (1.0 - pan) * 2.0;
                right += x * pan * 2.0;
            }
            frame.fill((left + right) * 0.5);
            if ch >= 2 {
                frame[0] = left;
                frame[1] = right;
            }
        }
    }
    fn reset(&mut self) {
        self.voices.fill(Voice::default());
        self.channels.reset();
        self.sustain.fill(false);
        self.age = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn note_ids_isolate_unison_and_choke() {
        let mut s = ExpressiveSynth::<4>::default();
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 64,
            channels: 2,
        })
        .unwrap();
        let on = |id| Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 1.0,
                note_id: id,
            },
        };
        let mut d = [0.0; 128];
        let mut play = |s: &mut ExpressiveSynth<4>, events: &[Event]| {
            s.process(
                &mut AudioBuffer::new(&mut d, 2).unwrap(),
                &ProcessContext {
                    events,
                    transport: Transport::default(),
                },
            )
        };
        play(&mut s, &[on(1), on(2)]);
        assert_eq!(s.active_voices(), 2);
        play(
            &mut s,
            &[Event {
                frame: 0,
                message: MidiMessage::NoteChoke {
                    target: NoteTarget {
                        note_id: 1,
                        channel: -1,
                        key: -1,
                    },
                },
            }],
        );
        assert_eq!(s.active_voices(), 1);
        assert_eq!(
            s.voices.iter().find(|v| v.envelope.is_active()).unwrap().id,
            2
        );
    }
    #[test]
    fn mpe_combines_master_and_member_pitch() {
        let mut c = ExpressionChannels::default();
        c.zone = Some(
            MpeZone {
                master: 0,
                first_member: 1,
                last_member: 15,
                member_bend: 48.0,
                master_bend: 2.0,
            }
            .validate()
            .unwrap(),
        );
        c.update(MidiMessage::PitchBend {
            channel: 1,
            value: 16383,
        });
        c.update(MidiMessage::PitchBend {
            channel: 0,
            value: 16383,
        });
        assert_eq!(c.values(1).0, 50.0);
        assert_eq!(c.values(2).0, 2.0);
    }
}
