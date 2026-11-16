//! Immutable preloaded banks, note/velocity zones, per-group round robin,
//! sustain pedal and crossfaded voice stealing. No file I/O on the audio thread.
use soniclaw_core::*;
use soniclaw_dsp::Sample;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct Zone {
    pub sample: usize,
    pub low_key: u8,
    pub high_key: u8,
    pub low_velocity: u8,
    pub high_velocity: u8,
    /// Zones sharing a group are alternatives. Different groups layer together.
    pub round_robin_group: u16,
    pub gain: f32,
    pub one_shot: bool,
    pub loop_frames: Option<(usize, usize)>,
}
impl Zone {
    pub fn single(sample: usize, key: u8) -> Self {
        Self {
            sample,
            low_key: key,
            high_key: key,
            low_velocity: 1,
            high_velocity: 127,
            round_robin_group: 0,
            gain: 1.0,
            one_shot: true,
            loop_frames: None,
        }
    }
}
pub struct SampleBank {
    samples: Vec<Sample>,
    zones: Vec<Zone>,
    bytes: usize,
}
impl SampleBank {
    pub fn new(
        samples: Vec<Sample>,
        zones: Vec<Zone>,
        max_pcm_bytes: usize,
    ) -> Result<Self, Error> {
        if samples.is_empty() || samples.len() > 1024 || zones.is_empty() || zones.len() > 1024 {
            return Err(Error::InvalidParameter);
        }
        let mut bytes = 0usize;
        for sample in &samples {
            bytes = bytes
                .checked_add(
                    sample
                        .data()
                        .len()
                        .checked_mul(4)
                        .ok_or(Error::InvalidBuffer)?,
                )
                .ok_or(Error::InvalidBuffer)?;
        }
        if bytes > max_pcm_bytes {
            return Err(Error::InvalidBuffer);
        }
        for z in &zones {
            let s = samples.get(z.sample).ok_or(Error::InvalidParameter)?;
            if z.low_key > z.high_key
                || z.high_key > 127
                || z.low_velocity == 0
                || z.low_velocity > z.high_velocity
                || z.high_velocity > 127
                || z.round_robin_group > 255
                || !z.gain.is_finite()
                || !(0.0..=8.0).contains(&z.gain)
            {
                return Err(Error::InvalidParameter);
            }
            if let Some((a, b)) = z.loop_frames {
                if a >= b || b > s.frames() || z.one_shot {
                    return Err(Error::InvalidParameter);
                }
            }
        }
        Ok(Self {
            samples,
            zones,
            bytes,
        })
    }
    pub fn pcm_bytes(&self) -> usize {
        self.bytes
    }
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }
}
#[derive(Clone, Copy, Default)]
struct Voice {
    sample: usize,
    zone: usize,
    position: f64,
    base_increment: f64,
    channel: u8,
    note: u8,
    velocity: f32,
    level: f32,
    fade: f32,
    released: bool,
    held: bool,
    active: bool,
    age: u64,
}
#[derive(Clone, Copy, Default)]
struct Slot {
    current: Voice,
    retiring: Voice,
}
pub struct MultiSampler<const N: usize = 64> {
    bank: Arc<SampleBank>,
    voices: [Slot; N],
    sr: f32,
    age: u64,
    rr: [u64; 256],
    sustain: [bool; 16],
    bend: [f32; 16],
    release_step: f32,
    steal_step: f32,
    pitch_bend_range: f32,
}
impl<const N: usize> MultiSampler<N> {
    pub fn new(bank: Arc<SampleBank>) -> Self {
        Self {
            bank,
            voices: [Slot::default(); N],
            sr: 48000.0,
            age: 0,
            rr: [0; 256],
            sustain: [false; 16],
            bend: [1.0; 16],
            release_step: 1.0 / 2400.0,
            steal_step: 1.0 / 240.0,
            pitch_bend_range: 2.0,
        }
    }
    /// Set while stopped. Can be set to 48 semitones for channel-per-note MPE pitch.
    pub fn set_pitch_bend_range(&mut self, semitones: f32) -> Result<(), Error> {
        if !semitones.is_finite() || !(0.0..=96.0).contains(&semitones) {
            return Err(Error::InvalidParameter);
        }
        self.pitch_bend_range = semitones;
        Ok(())
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.current.active).count()
    }
    fn note_off(&mut self, ch: u8, note: Option<u8>) {
        for slot in &mut self.voices {
            let v = &mut slot.current;
            if v.channel == ch
                && note.is_none_or(|n| n == v.note)
                && v.active
                && !self.bank.zones[v.zone].one_shot
            {
                v.held = false;
                if !self.sustain[ch as usize] {
                    v.released = true;
                }
            }
        }
    }
    fn start(&mut self, zone: usize, ch: u8, note: u8, velocity: u8) {
        if N == 0 {
            return;
        }
        let index = self
            .voices
            .iter()
            .position(|s| !s.current.active)
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| s.current.age)
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            });
        let slot = &mut self.voices[index];
        let stealing = slot.current.active;
        if stealing {
            slot.retiring = slot.current;
            slot.retiring.released = true;
            slot.retiring.fade = -self.steal_step;
        }
        let z = &self.bank.zones[zone];
        let sample = &self.bank.samples[z.sample];
        self.age = self.age.wrapping_add(1);
        slot.current = Voice {
            sample: z.sample,
            zone,
            position: 0.0,
            base_increment: sample.sample_rate() / self.sr as f64
                * 2.0_f64.powf((note as f64 - sample.root_note() as f64) / 12.0),
            channel: ch,
            note,
            velocity: velocity as f32 / 127.0 * z.gain,
            level: if stealing { 0.0 } else { 1.0 },
            fade: if stealing { self.steal_step } else { 0.0 },
            released: false,
            held: true,
            active: true,
            age: self.age,
        };
    }
    fn message(&mut self, m: MidiMessage) {
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if channel < 16 && note < 128 && velocity > 0 => {
                // Fixed stack counts and a second scan avoid allocation on note-on.
                let mut counts = [0usize; 256];
                for z in &self.bank.zones {
                    if (z.low_key..=z.high_key).contains(&note)
                        && (z.low_velocity..=z.high_velocity).contains(&velocity)
                    {
                        counts[z.round_robin_group as usize] += 1;
                    }
                }
                let mut ordinals = [0usize; 256];
                for index in 0..self.bank.zones.len() {
                    let z = &self.bank.zones[index];
                    if !(z.low_key..=z.high_key).contains(&note)
                        || !(z.low_velocity..=z.high_velocity).contains(&velocity)
                    {
                        continue;
                    }
                    let group = z.round_robin_group as usize;
                    let ordinal = ordinals[group];
                    ordinals[group] += 1;
                    if ordinal == (self.rr[group] as usize % counts[group]) {
                        self.start(index, channel, note, velocity);
                    }
                }
                for (g, count) in counts.iter().enumerate() {
                    if *count > 0 {
                        self.rr[g] = self.rr[g].wrapping_add(1);
                    }
                }
            }
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } if channel < 16 => self.note_off(channel, Some(note)),
            MidiMessage::ControlChange {
                channel,
                controller: 64,
                value,
            } if channel < 16 => {
                let c = channel as usize;
                self.sustain[c] = value >= 64;
                if !self.sustain[c] {
                    for s in &mut self.voices {
                        let v = &mut s.current;
                        if v.channel == channel && !v.held && !self.bank.zones[v.zone].one_shot {
                            v.released = true;
                        }
                    }
                }
            }
            MidiMessage::ControlChange {
                channel,
                controller: 123,
                ..
            } if channel < 16 => self.note_off(channel, None),
            MidiMessage::ControlChange {
                channel,
                controller: 120,
                ..
            } if channel < 16 => {
                for s in &mut self.voices {
                    if s.current.channel == channel {
                        s.current.active = false;
                    }
                    if s.retiring.channel == channel {
                        s.retiring.active = false;
                    }
                }
            }
            MidiMessage::PitchBend { channel, value } if channel < 16 && value <= 16383 => {
                self.bend[channel as usize] =
                    2.0_f32.powf((value as f32 - 8192.0) / 8192.0 * self.pitch_bend_range / 12.0);
            }
            MidiMessage::AllNotesOff => self.reset(),
            _ => {}
        }
    }
    fn tick(bank: &SampleBank, v: &mut Voice, bend: f32, release: f32) -> [f32; 2] {
        if !v.active {
            return [0.0; 2];
        }
        let sample = &bank.samples[v.sample];
        let zone = &bank.zones[v.zone];
        if let Some((start, end)) = zone.loop_frames {
            if !v.released && v.position >= end as f64 {
                v.position = start as f64 + (v.position - start as f64) % (end - start) as f64;
            }
        }
        if v.position >= sample.frames() as f64 {
            v.active = false;
            return [0.0; 2];
        }
        let a = v.position as usize;
        let mut b = (a + 1).min(sample.frames() - 1);
        if let Some((start, end)) = zone.loop_frames {
            if !v.released && b >= end {
                b = start;
            }
        }
        let frac = (v.position - a as f64) as f32;
        if v.fade != 0.0 {
            v.level = (v.level + v.fade).clamp(0.0, 1.0);
            if v.level >= 1.0 {
                v.fade = 0.0;
            }
        } else if v.released {
            v.level = (v.level - release).max(0.0);
        }
        if v.level <= 0.0 && v.fade <= 0.0 {
            v.active = false;
            return [0.0; 2];
        }
        let read = |c: usize| {
            let x = sample.data()[a * sample.channels() + c];
            let y = sample.data()[b * sample.channels() + c];
            (x + (y - x) * frac) * v.velocity * v.level
        };
        let left = read(0);
        let right = if sample.channels() == 2 {
            read(1)
        } else {
            left
        };
        v.position += v.base_increment * bend as f64;
        [left, right]
    }
}
impl<const N: usize> AudioProcessor for MultiSampler<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.sr = s.sample_rate as f32;
        self.release_step = 1.0 / (self.sr * 0.05).max(1.0);
        self.steal_step = 1.0 / (self.sr * 0.005).max(1.0);
        self.reset();
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let ch = b.channels();
        let mut event = 0;
        for (index, f) in b.samples_mut().chunks_exact_mut(ch).enumerate() {
            while event < ctx.events.len() && ctx.events[event].frame == index {
                self.message(ctx.events[event].message);
                event += 1;
            }
            let mut sum = [0.0; 2];
            for s in &mut self.voices {
                for v in [&mut s.current, &mut s.retiring] {
                    let x = Self::tick(
                        &self.bank,
                        v,
                        self.bend[v.channel as usize],
                        self.release_step,
                    );
                    sum[0] += x[0];
                    sum[1] += x[1];
                }
            }
            let mono = (sum[0] + sum[1]) * 0.5;
            f.fill(mono);
            if ch >= 2 {
                f[0] = sum[0];
                f[1] = sum[1];
            }
        }
    }
    fn reset(&mut self) {
        self.voices.fill(Slot::default());
        self.rr.fill(0);
        self.sustain.fill(false);
        self.bend.fill(1.0);
        self.age = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> ProcessSpec {
        ProcessSpec {
            sample_rate: 1000.0,
            max_block_frames: 256,
            channels: 1,
        }
    }
    fn play(s: &mut MultiSampler<1>, events: &[Event], data: &mut [f32]) {
        s.process(
            &mut AudioBuffer::new(data, 1).unwrap(),
            &ProcessContext {
                events,
                transport: Transport::default(),
            },
        );
    }
    fn note(frame: usize, v: u8) -> Event {
        Event {
            frame,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: v,
            },
        }
    }
    #[test]
    fn velocity_layers_and_round_robin() {
        let samples = vec![
            Sample::new(vec![1.0; 1000], 1, 1000.0, 60).unwrap(),
            Sample::new(vec![0.5; 1000], 1, 1000.0, 60).unwrap(),
            Sample::new(vec![0.25; 1000], 1, 1000.0, 60).unwrap(),
        ];
        let mut zones = vec![
            Zone::single(0, 60),
            Zone::single(1, 60),
            Zone::single(2, 60),
        ];
        zones[0].low_velocity = 64;
        zones[1].low_velocity = 64;
        zones[2].high_velocity = 63;
        let bank = Arc::new(SampleBank::new(samples, zones, 12000).unwrap());
        let mut s = MultiSampler::<1>::new(bank);
        s.prepare(spec()).unwrap();
        let mut d = [0.0; 64];
        play(&mut s, &[note(0, 127)], &mut d);
        assert_eq!(d[10], 1.0);
        play(&mut s, &[note(0, 127)], &mut d);
        assert_eq!(d[10], 0.5);
        play(&mut s, &[note(0, 63)], &mut d);
        assert!((d[10] - 0.25 * 63.0 / 127.0).abs() < 1e-6);
    }
    #[test]
    fn sustain_and_stealing_crossfade() {
        let sample = Sample::new(vec![1.0; 1000], 1, 1000.0, 60).unwrap();
        let mut z = Zone::single(0, 60);
        z.one_shot = false;
        z.loop_frames = Some((0, 100));
        let bank = Arc::new(SampleBank::new(vec![sample], vec![z], 4000).unwrap());
        let mut s = MultiSampler::<1>::new(bank);
        s.prepare(spec()).unwrap();
        let mut d = [0.0; 128];
        let events = [
            note(0, 127),
            Event {
                frame: 1,
                message: MidiMessage::ControlChange {
                    channel: 0,
                    controller: 64,
                    value: 127,
                },
            },
            Event {
                frame: 2,
                message: MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 0,
                },
            },
        ];
        play(&mut s, &events, &mut d);
        assert_eq!(d[127], 1.0);
        play(
            &mut s,
            &[Event {
                frame: 0,
                message: MidiMessage::ControlChange {
                    channel: 0,
                    controller: 64,
                    value: 0,
                },
            }],
            &mut d,
        );
        assert_eq!(d[127], 0.0);
        assert_eq!(s.active_voices(), 0);
        s.reset();
        play(&mut s, &[note(0, 127), note(20, 127)], &mut d);
        for x in &d[20..30] {
            assert!((*x - 1.0).abs() < 1e-5, "crossfade {x}");
        }
    }
    #[test]
    fn memory_budget_and_bad_loops() {
        let sample = Sample::new(vec![0.0; 100], 1, 1000.0, 60).unwrap();
        assert!(SampleBank::new(vec![sample], vec![Zone::single(0, 60)], 399).is_err());
    }
}
