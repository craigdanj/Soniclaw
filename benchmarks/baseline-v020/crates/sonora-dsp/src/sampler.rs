//! Preloaded mono/stereo sampling with fixed voices and linear interpolation.
//! No streaming, velocity layers, loops, multisamples, or band-limited resampling.
use crate::{Adsr, AdsrSettings};
use sonora_core::*;
use std::sync::Arc;

pub struct Sample {
    samples: Arc<[f32]>,
    channels: usize,
    sample_rate: f64,
    root_note: u8,
}
impl Sample {
    pub fn new(
        samples: impl Into<Arc<[f32]>>,
        channels: usize,
        sample_rate: f64,
        root_note: u8,
    ) -> Result<Self, Error> {
        let samples = samples.into();
        if !(1..=2).contains(&channels)
            || samples.is_empty()
            || samples.len() % channels != 0
            || samples.iter().any(|s| !s.is_finite())
            || !sample_rate.is_finite()
            || !(1.0..=768000.0).contains(&sample_rate)
            || root_note > 127
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(Self {
            samples,
            channels,
            sample_rate,
            root_note,
        })
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
}
#[derive(Clone, Copy, Default)]
struct SampleVoice {
    position: f64,
    increment: f64,
    envelope: Adsr,
    channel: u8,
    note: u8,
    velocity: f32,
    age: u64,
}
pub struct Sampler<const N: usize = 16> {
    sample: Sample,
    voices: [SampleVoice; N],
    sample_rate: f32,
    age: u64,
}
impl<const N: usize> Sampler<N> {
    pub fn new(sample: Sample) -> Self {
        Self {
            sample,
            voices: [SampleVoice::default(); N],
            sample_rate: 48000.0,
            age: 0,
        }
    }
    fn message(&mut self, message: MidiMessage) {
        match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity != 0 && N > 0 => {
                let i = self
                    .voices
                    .iter()
                    .position(|v| v.envelope.is_active() && v.note == note && v.channel == channel)
                    .or_else(|| self.voices.iter().position(|v| !v.envelope.is_active()))
                    .unwrap_or_else(|| {
                        self.voices
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| v.age)
                            .map(|(i, _)| i)
                            .unwrap_or(0)
                    });
                self.age = self.age.wrapping_add(1);
                let v = &mut self.voices[i];
                *v = SampleVoice {
                    increment: self.sample.sample_rate / self.sample_rate as f64
                        * 2.0_f64.powf((note as f64 - self.sample.root_note as f64) / 12.0),
                    channel,
                    note,
                    velocity: velocity as f32 / 127.0,
                    age: self.age,
                    ..SampleVoice::default()
                };
                let _ = v.envelope.configure(AdsrSettings {
                    attack: 0.0,
                    decay: 0.0,
                    sustain: 1.0,
                    release: 0.05,
                });
                v.envelope.note_on();
            }
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => {
                for v in &mut self.voices {
                    if v.channel == channel && v.note == note {
                        v.envelope.note_off(self.sample_rate);
                    }
                }
            }
            MidiMessage::AllNotesOff => self.reset(),
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } => {
                for v in &mut self.voices {
                    if v.channel == channel {
                        v.envelope.note_off(self.sample_rate);
                    }
                }
            }
            _ => {}
        }
    }
}
impl<const N: usize> AudioProcessor for Sampler<N> {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        self.sample_rate = spec.sample_rate as f32;
        self.reset();
        Ok(())
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let output_channels = audio.channels();
        let mut event = 0;
        for (index, output) in audio
            .samples_mut()
            .chunks_exact_mut(output_channels)
            .enumerate()
        {
            while event < ctx.events.len() && ctx.events[event].frame == index {
                self.message(ctx.events[event].message);
                event += 1;
            }
            output.fill(0.0);
            for v in &mut self.voices {
                if !v.envelope.is_active() {
                    continue;
                }
                let frame = v.position as usize;
                if frame >= self.sample.frames() {
                    v.envelope.reset();
                    continue;
                }
                let next = (frame + 1).min(self.sample.frames() - 1);
                let fraction = (v.position - frame as f64) as f32;
                let amplitude = v.envelope.next_sample(self.sample_rate) * v.velocity;
                let read = |channel: usize| {
                    let a = self.sample.samples[frame * self.sample.channels + channel];
                    let b = self.sample.samples[next * self.sample.channels + channel];
                    (a + (b - a) * fraction) * amplitude
                };
                let left = read(0);
                let right = if self.sample.channels == 2 {
                    read(1)
                } else {
                    left
                };
                if output_channels == 1 {
                    output[0] += (left + right) * 0.5;
                } else {
                    output[0] += left;
                    output[1] += right;
                    for channel in &mut output[2..] {
                        *channel += (left + right) * 0.5;
                    }
                }
                v.position += v.increment;
            }
        }
    }
    fn reset(&mut self) {
        self.voices = [SampleVoice::default(); N];
        self.age = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_rate_conversion_and_end_of_sample() {
        let sample = Sample::new(vec![0.0, 1.0, 0.0, -1.0], 1, 24000.0, 60).unwrap();
        let mut s = Sampler::<4>::new(sample);
        s.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 16,
            channels: 2,
        })
        .unwrap();
        let events = [Event {
            frame: 2,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 127,
            },
        }];
        let mut output = [0.0; 32];
        s.process(
            &mut AudioBuffer::new(&mut output, 2).unwrap(),
            &ProcessContext {
                events: &events,
                transport: Transport::default(),
            },
        );
        assert_eq!(&output[..4], &[0.0; 4]);
        assert_eq!(output[6], 0.5);
        assert_eq!(output[8], 1.0);
        assert!(output[20..].iter().all(|s| *s == 0.0));
        assert!(output.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
    }
    #[test]
    fn rejects_nonfinite_and_partial_frames() {
        assert!(Sample::new(vec![0.0; 3], 2, 48000.0, 60).is_err());
        assert!(Sample::new(vec![f32::NAN], 1, 48000.0, 60).is_err());
    }
}
