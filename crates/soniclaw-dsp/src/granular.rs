//! Deterministic fixed-grain sample source. PCM stays owned for the processor's
//! lifetime; note/grain creation never clones/drops sample owners or allocates.
use crate::{advanced::spec, resample::ResampleQuality, Adsr, AdsrSettings, Sample};
use soniclaw_core::*;
use std::sync::Arc;
#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    voice: usize,
    generation: u64,
    position: f64,
    age: usize,
    length: usize,
    pan: f32,
    increment: f64,
}
#[derive(Clone, Copy, Default)]
struct Voice {
    envelope: Adsr,
    channel: u8,
    note: u8,
    id: i32,
    velocity: f32,
    generation: u64,
    countdown: f64,
    scan: f64,
    age: u64,
    held: bool,
    expression: VoiceExpression,
}
pub struct GranularSynth<const N: usize = 128> {
    sample: Sample,
    grains: [Grain; N],
    voices: [Voice; 16],
    parameters: Vec<Arc<Parameter>>,
    smooth: Vec<SmoothedValue>,
    sr: f64,
    random: u32,
    clock: u64,
    channels: ExpressionChannels,
    sustain: [bool; 16],
    pub dropped_grains: u64,
}
impl<const N: usize> GranularSynth<N> {
    pub fn new(sample: Sample, prefix: &str) -> Result<Self, Error> {
        if N == 0 || N > 512 || prefix.is_empty() {
            return Err(Error::InvalidParameter);
        }
        let specs = [
            ("density", 1.0, 80.0, 12.0),
            ("size_ms", 5.0, 250.0, 80.0),
            ("position", 0.0, 1.0, 0.25),
            ("scan", -2.0, 2.0, 0.1),
            ("jitter", 0.0, 1.0, 0.1),
            ("pitch", -24.0, 24.0, 0.0),
            ("spread", 0.0, 1.0, 0.7),
            ("gain", 0.0, 1.0, 0.25),
        ];
        let parameters = specs
            .iter()
            .map(|&(key, min, max, default)| {
                Ok(Arc::new(Parameter::new(ParameterInfo {
                    id: &format!("{prefix}.{key}"),
                    name: key,
                    min,
                    max,
                    default,
                    unit: "",
                })?))
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            sample,
            grains: [Grain::default(); N],
            voices: [Voice::default(); 16],
            parameters,
            smooth: vec![],
            sr: 48000.0,
            random: 0x736f6e69,
            clock: 0,
            channels: ExpressionChannels::default(),
            sustain: [false; 16],
            dropped_grains: 0,
        })
    }
    pub fn parameters(&self) -> &[Arc<Parameter>] {
        &self.parameters
    }
    pub fn reuse_parameters(&mut self, existing: &[Arc<Parameter>]) -> Result<(), Error> {
        for p in &mut self.parameters {
            if let Some(old) = existing.iter().find(|q| q.info().id == p.info().id) {
                if old.info().min != p.info().min || old.info().max != p.info().max {
                    return Err(Error::InvalidParameter);
                }
                *p = old.clone();
            } else if !existing.is_empty() {
                return Err(Error::InvalidParameter);
            }
        }
        Ok(())
    }
    pub fn active_grains(&self) -> usize {
        self.grains.iter().filter(|g| g.active).count()
    }
    pub fn set_mpe_zone(&mut self, zone: Option<MpeZone>) -> Result<(), Error> {
        self.channels.zone = zone.map(MpeZone::validate).transpose()?;
        Ok(())
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
                    v.envelope.note_off(self.sr as f32);
                }
            }
        }
    }
    fn message(&mut self, message: MidiMessage) {
        if self.channels.update(message) {
            return;
        }
        let m = if let MidiMessage::NoteOnId {
            channel,
            note,
            velocity,
            note_id,
        } = message
        {
            if velocity > 0.0 {
                self.start(channel, note, velocity, note_id);
                return;
            }
            MidiMessage::NoteOffId {
                target: NoteTarget {
                    note_id,
                    channel: channel as i16,
                    key: note as i16,
                },
            }
        } else {
            message
        };
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => self.start(channel, note, velocity as f32 / 127.0, -1),
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => self.release(
                NoteTarget {
                    note_id: -1,
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
            MidiMessage::ControlChange {
                channel,
                controller: 64,
                value,
            } if channel < 16 => {
                self.sustain[channel as usize] = value >= 64;
                if value < 64 {
                    for v in &mut self.voices {
                        if v.channel == channel && !v.held {
                            v.envelope.note_off(self.sr as f32);
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
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                true,
            ),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => self.release(
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: -1,
                },
                false,
            ),
            MidiMessage::AllNotesOff => self.reset(),
            _ => {}
        }
    }
    fn start(&mut self, channel: u8, note: u8, velocity: f32, id: i32) {
        if channel > 15 || note > 127 || !velocity.is_finite() || !(0.0..=1.0).contains(&velocity) {
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
        self.clock = self.clock.wrapping_add(1);
        let v = &mut self.voices[index];
        let generation = v.generation.wrapping_add(1);
        *v = Voice {
            channel,
            note,
            id,
            velocity,
            generation,
            age: self.clock,
            held: true,
            ..Default::default()
        };
        let _ = v.envelope.configure(AdsrSettings {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.8,
            release: 0.15,
        });
        v.envelope.note_on();
    }
    fn random(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        self.random as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}
impl<const N: usize> AudioProcessor for GranularSynth<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        spec(s)?;
        self.sr = s.sample_rate;
        self.smooth = self
            .parameters
            .iter()
            .map(|p| SmoothedValue::new(p.modulated(), (self.sr * 0.01).round() as usize))
            .collect();
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self.smooth.len() != 8 {
            b.clear();
            return;
        }
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.set_target(p.modulated());
        }
        let ch = b.channels();
        let mut event = 0;
        for (i, frame) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            while event < ctx.events.len() && ctx.events[event].frame == i {
                self.message(ctx.events[event].message);
                event += 1;
            }
            let mut c = [0.0; 8];
            for (v, s) in c.iter_mut().zip(&mut self.smooth) {
                *v = s.next_value();
            }
            let mut levels = [0.0; 16];
            for voice in 0..16 {
                if !self.voices[voice].envelope.is_active() {
                    continue;
                }
                levels[voice] = self.voices[voice].envelope.next_sample(self.sr as f32);
                self.voices[voice].countdown -= 1.0;
                self.voices[voice].scan =
                    (self.voices[voice].scan + c[3] as f64 / self.sr).rem_euclid(1.0);
                if (self.voices[voice].held || self.sustain[self.voices[voice].channel as usize])
                    && self.voices[voice].countdown <= 0.0
                {
                    self.voices[voice].countdown += self.sr / c[0] as f64;
                    let position = ((c[2] as f64
                        + self.voices[voice].scan
                        + self.random() as f64 * c[4] as f64 * 0.5)
                        .rem_euclid(1.0))
                        * self.sample.frames() as f64;
                    let pan = 0.5 + self.random() * c[6] * 0.5;
                    if let Some(g) = self.grains.iter_mut().find(|g| !g.active) {
                        let v = self.voices[voice];
                        *g = Grain {
                            active: true,
                            voice,
                            generation: v.generation,
                            position,
                            age: 0,
                            length: (c[1] as f64 * self.sr * 0.001).round().max(2.0) as usize,
                            pan,
                            increment: self.sample.sample_rate() / self.sr
                                * 2.0_f64.powf(
                                    (v.note as f64 - self.sample.root_note() as f64 + c[5] as f64)
                                        / 12.0,
                                ),
                        };
                    } else {
                        self.dropped_grains = self.dropped_grains.saturating_add(1);
                    }
                }
            }
            let mut sum = [0.0; 2];
            for g in &mut self.grains {
                if !g.active {
                    continue;
                }
                let v = &self.voices[g.voice];
                if g.generation != v.generation || !v.envelope.is_active() || g.age >= g.length {
                    g.active = false;
                    continue;
                }
                let (pitch, pressure, _) = self.channels.values(v.channel);
                let increment = (g.increment
                    * 2.0_f64.powf((pitch + v.expression.tuning) as f64 / 12.0))
                .clamp(1.0 / 32.0, 32.0);
                let envelope = 0.5
                    - 0.5 * (std::f32::consts::TAU * g.age as f32 / (g.length - 1) as f32).cos();
                let normalizer = (c[0] * c[1] * 0.001).max(1.0).sqrt();
                let gain = envelope
                    * levels[g.voice]
                    * v.velocity
                    * c[7]
                    * v.expression.volume
                    * (1.0 + pressure.max(v.expression.pressure) * 0.5)
                    / normalizer;
                let pan = (g.pan + v.expression.pan - 0.5).clamp(0.0, 1.0);
                for (channel, out) in sum.iter_mut().enumerate() {
                    let source_channel = if self.sample.channels() == 1 {
                        0
                    } else {
                        channel
                    };
                    let sample =
                        ResampleQuality::SincFast.interpolate(g.position, increment, |idx| {
                            let index = idx.rem_euclid(self.sample.frames() as isize) as usize;
                            self.sample.data()[index * self.sample.channels() + source_channel]
                        });
                    *out += sample
                        * gain
                        * if channel == 0 {
                            (1.0 - pan) * 2.0
                        } else {
                            pan * 2.0
                        };
                }
                g.position = (g.position + increment).rem_euclid(self.sample.frames() as f64);
                g.age += 1;
            }
            frame.fill((sum[0] + sum[1]) * 0.5);
            if ch >= 2 {
                frame[0] = sum[0];
                frame[1] = sum[1];
            }
        }
    }
    fn reset(&mut self) {
        self.grains.fill(Grain::default());
        self.voices.fill(Voice::default());
        self.sustain.fill(false);
        self.channels.reset();
        self.clock = 0;
        self.random = 0x736f6e69;
        self.dropped_grains = 0;
        for (s, p) in self.smooth.iter_mut().zip(&self.parameters) {
            s.reset(p.modulated());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grains_are_deterministic_and_release() {
        let make = |block| {
            let sample = Sample::new(
                (0..48000)
                    .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / 48000.0).sin() * 0.2)
                    .collect::<Vec<_>>(),
                1,
                48000.0,
                60,
            )
            .unwrap();
            let mut p = GranularSynth::<32>::new(sample, "g").unwrap();
            p.prepare(ProcessSpec {
                sample_rate: 48000.0,
                max_block_frames: block,
                channels: 2,
            })
            .unwrap();
            let mut engine = soniclaw_core::Transport::default();
            let mut output = vec![0.0; 48000];
            for (offset, d) in output.chunks_mut(block * 2).enumerate() {
                let frame = offset * block;
                let events = if frame == 0 {
                    vec![Event {
                        frame: 0,
                        message: MidiMessage::NoteOn {
                            channel: 0,
                            note: 60,
                            velocity: 127,
                        },
                    }]
                } else {
                    vec![]
                };
                p.process(
                    &mut AudioBuffer::new(d, 2).unwrap(),
                    &ProcessContext {
                        events: &events,
                        transport: engine,
                    },
                );
                engine.sample_position += d.len() as u64 / 2;
            }
            output
        };
        let a = make(64);
        let b = make(127);
        assert_eq!(a, b);
        assert!(a.iter().any(|x| x.abs() > 0.01));
    }
    #[test]
    fn zero_velocity_id_releases_only_the_matching_note() {
        let mut synth =
            GranularSynth::<8>::new(Sample::new(vec![0.1; 1000], 1, 48000.0, 60).unwrap(), "g")
                .unwrap();
        for note_id in [7, 8] {
            synth.message(MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id,
            });
        }
        synth.message(MidiMessage::NoteOnId {
            channel: 1,
            note: 60,
            velocity: 0.0,
            note_id: 7,
        });
        assert!(!synth.voices.iter().find(|v| v.id == 7).unwrap().held);
        assert!(synth.voices.iter().find(|v| v.id == 8).unwrap().held);
    }
}
