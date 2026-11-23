//! Four-operator polyphonic sine synthesis with prepared FM/PM routing.
use crate::{midi_hz, Adsr, AdsrSettings};
use serde::{Deserialize, Serialize};
use soniclaw_core::{
    validate_events, AudioBuffer, AudioProcessor, Error, MidiMessage, ProcessContext, ProcessSpec,
};
use std::f32::consts::TAU;

pub const OPERATORS: usize = 4;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Mode {
    Phase,
    Frequency,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub ratio: f32,
    pub output: f32,
    /// Self phase-feedback depth in cycles, evaluated from the previous sample.
    pub feedback: f32,
    pub envelope: AdsrSettings,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub mode: Mode,
    pub operators: [Operator; OPERATORS],
    /// `routes[from][to]`. Earlier operators may modulate later ones.
    /// Phase depths are in cycles. Frequency depths are multiples of carrier frequency.
    pub routes: [[f32; OPERATORS]; OPERATORS],
    pub gain: f32,
}

impl Default for Patch {
    fn default() -> Self {
        let mut operators = std::array::from_fn(|i| Operator {
            ratio: [2., 1., 3., 1.][i],
            output: 0.,
            feedback: 0.,
            envelope: AdsrSettings::default(),
        });
        operators[1].output = 0.7;
        operators[3].output = 0.3;
        let mut routes = [[0.; OPERATORS]; OPERATORS];
        routes[0][1] = 0.32;
        routes[2][3] = 0.16;
        Self {
            mode: Mode::Phase,
            operators,
            routes,
            gain: 0.35,
        }
    }
}

impl Patch {
    pub fn validate(self) -> Result<Self, Error> {
        if !(0. ..=1.).contains(&self.gain) || self.operators.iter().all(|op| op.output == 0.) {
            return Err(Error::InvalidParameter);
        }
        for (from, op) in self.operators.iter().enumerate() {
            if !(0.125..=16.).contains(&op.ratio)
                || !(0. ..=1.).contains(&op.output)
                || !(-0.45..=0.45).contains(&op.feedback)
                || [op.envelope.attack, op.envelope.decay, op.envelope.release]
                    .iter()
                    .any(|t| !(0. ..=10.).contains(t))
            {
                return Err(Error::InvalidParameter);
            }
            Adsr::default().configure(op.envelope)?;
            for (to, &depth) in self.routes[from].iter().enumerate() {
                if !(-4. ..=4.).contains(&depth) || (to <= from && depth != 0.) {
                    return Err(Error::InvalidRouting);
                }
            }
        }
        Ok(self)
    }
}

#[derive(Clone, Default)]
struct Voice {
    frequency: f32,
    channel: u8,
    note: u8,
    velocity: f32,
    held: bool,
    stamp: u64,
    phases: [f32; OPERATORS],
    previous: [f32; OPERATORS],
    envelopes: [Adsr; OPERATORS],
    last: f32,
    steal_tail: f32,
    fade: u8,
}

impl Voice {
    fn active(&self) -> bool {
        self.envelopes.iter().any(Adsr::is_active)
    }

    fn trigger(&mut self, patch: Patch, channel: u8, note: u8, velocity: f32, stamp: u64) {
        let tail = self.last;
        *self = Self {
            frequency: midi_hz(note),
            channel,
            note,
            velocity,
            held: true,
            stamp,
            steal_tail: tail,
            fade: if tail == 0. { 0 } else { 32 },
            ..Self::default()
        };
        for (envelope, operator) in self.envelopes.iter_mut().zip(patch.operators) {
            envelope
                .configure(operator.envelope)
                .expect("validated patch");
            envelope.note_on();
        }
    }

    fn release(&mut self, rate: f32) {
        self.held = false;
        for envelope in &mut self.envelopes {
            if envelope.is_active() {
                envelope.note_off(rate);
            }
        }
    }

    fn sample(&mut self, patch: &Patch, rate: f32) -> f32 {
        let mut samples = [0.; OPERATORS];
        let mut output = 0.;
        for (i, op) in patch.operators.iter().enumerate() {
            let modulation: f32 = samples[..i]
                .iter()
                .zip(&patch.routes)
                .map(|(&sample, row)| sample * row[i])
                .sum();
            let (pm, fm) = match patch.mode {
                Mode::Phase => (modulation, 0.),
                Mode::Frequency => (0., modulation),
            };
            let step = (self.frequency * op.ratio * (1. + fm) / rate).clamp(-0.49, 0.49);
            let phase = self.phases[i] + pm + self.previous[i] * op.feedback;
            let sample = (phase * TAU).sin() * self.envelopes[i].next_sample(rate);
            self.phases[i] = (self.phases[i] + step).rem_euclid(1.);
            self.previous[i] = sample;
            samples[i] = sample;
            output += sample * op.output;
        }
        let output = output * self.velocity;
        self.last = if self.fade == 0 {
            output
        } else {
            let old = f32::from(self.fade) / 32.;
            self.fade -= 1;
            self.steal_tail * old + output * (1. - old)
        };
        self.last
    }
}

pub struct FmSynth {
    patch: Patch,
    voices: Vec<Voice>,
    stamp: u64,
    spec: Option<ProcessSpec>,
    output_scale: f32,
}

impl FmSynth {
    pub fn new(patch: Patch, voices: usize) -> Result<Self, Error> {
        let patch = patch.validate()?;
        if !(1..=64).contains(&voices) {
            return Err(Error::InvalidParameter);
        }
        let outputs = patch.operators.iter().map(|op| op.output).sum::<f32>();
        Ok(Self {
            patch,
            voices: vec![Voice::default(); voices],
            stamp: 0,
            spec: None,
            output_scale: patch.gain / (outputs.max(1.) * (voices as f32).sqrt()),
        })
    }

    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    fn event(&mut self, message: MidiMessage, rate: f32) {
        match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity: velocity @ 1..=127,
            } => {
                let index = self
                    .voices
                    .iter()
                    .position(|v| !v.active())
                    .unwrap_or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| (v.held, v.stamp))
                            .expect("nonempty pool")
                            .0
                    });
                self.stamp = self.stamp.saturating_add(1);
                self.voices[index].trigger(
                    self.patch,
                    channel,
                    note,
                    f32::from(velocity) / 127.,
                    self.stamp,
                );
            }
            MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            }
            | MidiMessage::NoteOff { channel, note, .. } => {
                if let Some(voice) = self
                    .voices
                    .iter_mut()
                    .filter(|v| v.held && (v.channel, v.note) == (channel, note))
                    .min_by_key(|v| v.stamp)
                {
                    voice.release(rate);
                }
            }
            MidiMessage::AllNotesOff => {
                for voice in self.voices.iter_mut().filter(|v| v.held) {
                    voice.release(rate);
                }
            }
            _ => {}
        }
    }
}

impl AudioProcessor for FmSynth {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        let spec = spec.validate()?;
        if !(8_000. ..=192_000.).contains(&spec.sample_rate) || spec.channels > 2 {
            return Err(Error::InvalidSpec);
        }
        self.spec = Some(spec);
        self.reset();
        Ok(())
    }

    fn process(&mut self, buffer: &mut AudioBuffer<'_>, context: &ProcessContext<'_>) {
        let Some(spec) = self.spec else {
            buffer.clear();
            return;
        };
        if buffer.channels() != spec.channels
            || buffer.frames() > spec.max_block_frames
            || validate_events(context.events, buffer.frames()).is_err()
        {
            buffer.clear();
            return;
        }
        let rate = spec.sample_rate as f32;
        let mut events = context.events.iter().peekable();
        for (i, frame) in buffer
            .samples_mut()
            .chunks_exact_mut(spec.channels)
            .enumerate()
        {
            while events.peek().is_some_and(|e| e.frame == i) {
                self.event(events.next().expect("peeked event").message, rate);
            }
            let sum: f32 = self
                .voices
                .iter_mut()
                .filter(|v| v.active() || v.fade > 0)
                .map(|v| v.sample(&self.patch, rate))
                .sum();
            frame.fill((sum * self.output_scale).clamp(-1., 1.));
        }
    }

    fn reset(&mut self) {
        self.voices.fill(Voice::default());
        self.stamp = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soniclaw_core::Event;

    fn render(patch: Patch, block: usize) -> Vec<f32> {
        let mut synth = FmSynth::new(patch, 2).unwrap();
        synth
            .prepare(ProcessSpec {
                sample_rate: 48_000.,
                max_block_frames: 512,
                channels: 1,
            })
            .unwrap();
        let mut data = vec![0.; 2048];
        for (i, part) in data.chunks_mut(block).enumerate() {
            let start = i * block;
            let events = if (start..start + part.len()).contains(&35) {
                vec![Event {
                    frame: 35 - start,
                    message: MidiMessage::NoteOn {
                        channel: 1,
                        note: 69,
                        velocity: 127,
                    },
                }]
            } else {
                Vec::new()
            };
            let result = allocation_counter::measure(|| {
                synth.process(
                    &mut AudioBuffer::new(part, 1).unwrap(),
                    &ProcessContext {
                        events: &events,
                        transport: Default::default(),
                    },
                )
            });
            assert_eq!(result.count_total, 0);
        }
        data
    }

    #[test]
    fn modulation_changes_tone_and_note_timing_matches_across_blocks() {
        let patch = Patch::default();
        let phase = render(patch, 97);
        assert_eq!(phase, render(patch, 256));
        assert!(phase[..36].iter().all(|&x| x == 0.));
        assert!(phase[80..].iter().any(|x| x.abs() > 0.05));
        let plain = render(
            Patch {
                routes: [[0.; 4]; 4],
                ..patch
            },
            97,
        );
        assert!(phase.iter().zip(plain).any(|(x, y)| (x - y).abs() > 0.05));
        let fm = render(
            Patch {
                mode: Mode::Frequency,
                ..patch
            },
            97,
        );
        assert!(phase.iter().zip(fm).any(|(x, y)| (x - y).abs() > 0.03));
        let mut invalid = patch;
        invalid.routes[1][0] = 1.;
        assert!(FmSynth::new(invalid, 8).is_err());
    }
}
