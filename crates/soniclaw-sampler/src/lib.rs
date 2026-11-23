//! Immutable preloaded banks, note/velocity zones, per-group round robin,
//! sustain pedal and crossfaded voice stealing. No file I/O on the audio thread.
use soniclaw_core::*;
use soniclaw_dsp::{resample::ResampleQuality, Sample};
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct Zone {
    pub release_trigger: bool,
    pub velocity_fade_in: Option<[u8; 2]>,
    pub velocity_fade_out: Option<[u8; 2]>,
    pub velocity_power: bool,
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
    pub start_frame: usize,
    pub end_frame: Option<usize>,
    pub loop_crossfade: usize,
}
impl Zone {
    pub fn single(sample: usize, key: u8) -> Self {
        Self {
            release_trigger: false,
            velocity_fade_in: None,
            velocity_fade_out: None,
            velocity_power: false,
            sample,
            low_key: key,
            high_key: key,
            low_velocity: 1,
            high_velocity: 127,
            round_robin_group: 0,
            gain: 1.0,
            one_shot: true,
            loop_frames: None,
            start_frame: 0,
            end_frame: None,
            loop_crossfade: 0,
        }
    }
}
fn velocity_weight(z: &Zone, v: u8) -> f32 {
    let mut gain = 1.0;
    if let Some([a, b]) = z.velocity_fade_in {
        gain *= ((v as f32 - a as f32) / (b - a) as f32).clamp(0.0, 1.0);
    }
    if let Some([a, b]) = z.velocity_fade_out {
        gain *= ((b as f32 - v as f32) / (b - a) as f32).clamp(0.0, 1.0);
    }
    if z.velocity_power {
        gain.sqrt()
    } else {
        gain
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
            if (z.release_trigger && (!z.one_shot || z.loop_frames.is_some()))
                || [z.velocity_fade_in, z.velocity_fade_out]
                    .iter()
                    .flatten()
                    .any(|[a, b]| a >= b || *b > 127)
                || z.low_key > z.high_key
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
            let end = z.end_frame.unwrap_or(s.frames());
            if z.start_frame >= end
                || end > s.frames()
                || z.loop_crossfade > 65536
                || (z.loop_crossfade > 0 && z.loop_frames.is_none())
            {
                return Err(Error::InvalidParameter);
            }
            if let Some((a, b)) = z.loop_frames {
                if a < z.start_frame
                    || a >= b
                    || b > end
                    || z.one_shot
                    || z.loop_crossfade > (b - a) / 2
                {
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
    id: i32,
    expression: VoiceExpression,
    filter: [f32; 2],
    looped: bool,
}
#[derive(Clone, Copy, Default)]
struct Slot {
    current: Voice,
    retiring: Voice,
}
#[derive(Clone, Copy, Default)]
struct Held {
    channel: u8,
    note: u8,
    velocity: u8,
    id: i32,
    down: bool,
    age: u64,
}
pub struct MultiSampler<const N: usize = 64> {
    tuning: Option<Arc<soniclaw_tuning::Tuning>>,
    bank: Arc<SampleBank>,
    voices: [Slot; N],
    sr: f32,
    age: u64,
    rr: [u64; 256],
    rr_release: [u64; 256],
    held: [Held; 128],
    sustain: [bool; 16],
    bend: [f32; 16],
    release_step: f32,
    steal_step: f32,
    pitch_bend_range: f32,
    channels: ExpressionChannels,
    quality: ResampleQuality,
    attack_ms: f32,
    release_ms: f32,
    pending_note_id: i32,
}
impl<const N: usize> MultiSampler<N> {
    pub fn new(bank: Arc<SampleBank>) -> Self {
        Self {
            tuning: None,
            bank,
            voices: [Slot::default(); N],
            sr: 48000.0,
            age: 0,
            rr: [0; 256],
            rr_release: [0; 256],
            held: [Held::default(); 128],
            sustain: [false; 16],
            bend: [1.0; 16],
            release_step: 1.0 / 2400.0,
            steal_step: 1.0 / 240.0,
            pitch_bend_range: 2.0,
            channels: ExpressionChannels::default(),
            quality: ResampleQuality::Linear,
            attack_ms: 0.0,
            release_ms: 50.0,
            pending_note_id: -1,
        }
    }
    /// Set while stopped. Can be set to 48 semitones for channel-per-note MPE pitch.
    /// Set while stopped; releases all voices before replacing the immutable table.
    pub fn set_tuning(
        &mut self,
        tuning: Option<Arc<soniclaw_tuning::Tuning>>,
    ) -> Result<(), Error> {
        if let Some(t) = &tuning {
            t.validate()?;
        }
        self.reset();
        self.tuning = tuning;
        Ok(())
    }
    pub fn set_pitch_bend_range(&mut self, semitones: f32) -> Result<(), Error> {
        if !semitones.is_finite() || !(0.0..=96.0).contains(&semitones) {
            return Err(Error::InvalidParameter);
        }
        self.pitch_bend_range = semitones;
        self.channels.bend_range = semitones;
        Ok(())
    }
    /// Configure while stopped. The legacy default remains Linear.
    pub fn set_quality(&mut self, quality: ResampleQuality) {
        self.quality = quality;
    }
    pub fn set_mpe_zone(&mut self, zone: Option<MpeZone>) -> Result<(), Error> {
        self.channels.zone = zone.map(MpeZone::validate).transpose()?;
        Ok(())
    }
    pub fn set_envelope(&mut self, attack_ms: f32, release_ms: f32) -> Result<(), Error> {
        if !attack_ms.is_finite()
            || !(0.0..=10000.0).contains(&attack_ms)
            || !release_ms.is_finite()
            || !(1.0..=30000.0).contains(&release_ms)
        {
            return Err(Error::InvalidParameter);
        }
        self.attack_ms = attack_ms;
        self.release_ms = release_ms;
        Ok(())
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.current.active).count()
    }
    fn note_off(&mut self, ch: u8, note: Option<u8>) {
        self.end_notes(
            NoteTarget {
                channel: ch as i16,
                key: note.map_or(-1, |n| n as i16),
                note_id: -1,
            },
            false,
        );
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
        if self
            .tuning
            .as_ref()
            .is_some_and(|t| t.frequency(note).is_none())
        {
            return;
        }
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
            position: z.start_frame as f64,
            base_increment: sample.sample_rate() / self.sr as f64
                * self
                    .tuning
                    .as_ref()
                    .and_then(|t| t.frequency(note))
                    .map(|hz| hz / (440. * 2f64.powf((sample.root_note() as f64 - 69.) / 12.)))
                    .unwrap_or_else(|| {
                        2.0_f64.powf((note as f64 - sample.root_note() as f64) / 12.0)
                    }),
            channel: ch,
            note,
            velocity: velocity as f32 / 127.0 * z.gain * velocity_weight(z, velocity),
            level: if stealing || self.attack_ms > 0.0 {
                0.0
            } else {
                1.0
            },
            fade: if stealing {
                self.steal_step
            } else if self.attack_ms > 0.0 {
                1.0 / (self.sr * self.attack_ms * 0.001).max(1.0)
            } else {
                0.0
            },
            released: false,
            held: true,
            active: true,
            age: self.age,
            id: self.pending_note_id,
            expression: VoiceExpression::default(),
            filter: [0.0; 2],
            looped: false,
        };
    }
    fn trigger(&mut self, ch: u8, note: u8, velocity: u8, release: bool) {
        let mut counts = [0usize; 256];
        let mut ordinals = [0usize; 256];
        let matches = |z: &Zone| {
            z.release_trigger == release
                && (z.low_key..=z.high_key).contains(&note)
                && (z.low_velocity..=z.high_velocity).contains(&velocity)
                && velocity_weight(z, velocity) > 0.0
        };
        for z in &self.bank.zones {
            if matches(z) {
                counts[z.round_robin_group as usize] += 1;
            }
        }
        for i in 0..self.bank.zones.len() {
            let z = &self.bank.zones[i];
            if !matches(z) {
                continue;
            }
            let g = z.round_robin_group as usize;
            let ordinal = ordinals[g];
            ordinals[g] += 1;
            let counter = if release {
                self.rr_release[g]
            } else {
                self.rr[g]
            };
            if ordinal == counter as usize % counts[g] {
                self.start(i, ch, note, velocity);
            }
        }
        let counters = if release {
            &mut self.rr_release
        } else {
            &mut self.rr
        };
        for (g, n) in counts.iter().enumerate() {
            if *n > 0 {
                counters[g] = counters[g].wrapping_add(1);
            }
        }
    }
    fn end_notes(&mut self, target: NoteTarget, kill: bool) {
        let mut releases = [Held::default(); 128];
        let mut count = 0;
        for h in &mut self.held {
            if h.velocity > 0 && target.matches(h.id, h.channel, h.note) {
                h.down = false;
                if kill {
                    h.velocity = 0;
                } else if !self.sustain[h.channel as usize] {
                    releases[count] = *h;
                    count += 1;
                    h.velocity = 0;
                }
            }
        }
        for h in &releases[..count] {
            self.pending_note_id = h.id;
            self.trigger(h.channel, h.note, h.velocity, true);
        }
    }
    fn flush_releases(&mut self, ch: u8) {
        let mut releases = [Held::default(); 128];
        let mut count = 0;
        for h in &mut self.held {
            if h.velocity > 0 && h.channel == ch && !h.down {
                releases[count] = *h;
                count += 1;
                h.velocity = 0;
            }
        }
        for h in &releases[..count] {
            self.pending_note_id = h.id;
            self.trigger(ch, h.note, h.velocity, true);
        }
    }
    fn message(&mut self, m: MidiMessage) {
        if self.channels.update(m) {
            return;
        }
        self.pending_note_id = -1;
        let m = if let MidiMessage::NoteOnId {
            channel,
            note,
            velocity,
            note_id,
        } = m
        {
            if !velocity.is_finite() || !(0.0..=1.0).contains(&velocity) || note_id < -1 {
                return;
            }
            self.pending_note_id = note_id;
            MidiMessage::NoteOn {
                channel,
                note,
                velocity: (velocity * 127.0).round() as u8,
            }
        } else {
            m
        };
        match m {
            MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => {
                if !target.valid() {
                    return;
                }
                self.end_notes(target, matches!(m, MidiMessage::NoteChoke { .. }));
                for slot in &mut self.voices {
                    for v in [&mut slot.current, &mut slot.retiring] {
                        if target.matches(v.id, v.channel, v.note) {
                            if matches!(m, MidiMessage::NoteChoke { .. }) {
                                v.active = false;
                            } else if !self.bank.zones[v.zone].one_shot {
                                v.held = false;
                                if !self.sustain[v.channel as usize] {
                                    v.released = true;
                                }
                            }
                        }
                    }
                }
            }
            MidiMessage::NoteExpression {
                target,
                expression,
                value,
            } => {
                for slot in &mut self.voices {
                    for v in [&mut slot.current, &mut slot.retiring] {
                        if target.valid() && target.matches(v.id, v.channel, v.note) {
                            v.expression.set(expression, value);
                        }
                    }
                }
            }
            MidiMessage::PolyPressure {
                channel,
                note,
                value,
            } => {
                if value <= 127 {
                    for slot in &mut self.voices {
                        if slot.current.channel == channel && slot.current.note == note {
                            slot.current.expression.pressure = value as f32 / 127.0;
                        }
                    }
                }
            }
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if channel < 16 && note < 128 && velocity > 0 => {
                let slot = self
                    .held
                    .iter()
                    .position(|h| {
                        h.velocity == 0
                            || (h.channel == channel
                                && h.note == note
                                && h.id == self.pending_note_id)
                    })
                    .unwrap_or_else(|| {
                        self.held
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, h)| h.age)
                            .map(|(i, _)| i)
                            .unwrap_or(0)
                    });
                self.held[slot] = Held {
                    channel,
                    note,
                    velocity,
                    id: self.pending_note_id,
                    down: true,
                    age: self.age,
                };
                self.trigger(channel, note, velocity, false);
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
                    self.flush_releases(channel);
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
                for h in &mut self.held {
                    if h.channel == channel {
                        h.velocity = 0;
                    }
                }
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
    fn tick(
        bank: &SampleBank,
        v: &mut Voice,
        controls: (f32, f32, f32),
        release: f32,
        quality: ResampleQuality,
        sr: f32,
    ) -> [f32; 2] {
        if !v.active {
            return [0.0; 2];
        }
        let sample = &bank.samples[v.sample];
        let zone = &bank.zones[v.zone];
        if let Some((start, end)) = zone.loop_frames {
            if !v.released && v.position >= end as f64 {
                v.looped = true;
                let start = start + zone.loop_crossfade;
                v.position = start as f64 + (v.position - end as f64) % (end - start) as f64;
            }
        }
        if v.position >= zone.end_frame.unwrap_or(sample.frames()) as f64 {
            v.active = false;
            return [0.0; 2];
        }
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
        let increment = (v.base_increment
            * 2.0_f64.powf((controls.0 + v.expression.tuning) as f64 / 12.0))
        .clamp(1.0 / 32.0, 32.0);
        let read_at = |c: usize, position: f64, wrap: bool| {
            quality.interpolate(position, increment, |i| {
                let i = if let Some((start, end)) = zone.loop_frames {
                    let start = if v.looped {
                        start + zone.loop_crossfade
                    } else {
                        start
                    };
                    if wrap
                        && !v.released
                        && (i >= end as isize || (v.looped && i < start as isize))
                    {
                        start as isize + (i - start as isize).rem_euclid((end - start) as isize)
                    } else {
                        i
                    }
                } else {
                    i
                };
                if i >= zone.start_frame as isize
                    && (i as usize) < zone.end_frame.unwrap_or(sample.frames())
                {
                    sample.data()[i as usize * sample.channels() + c]
                } else {
                    0.0
                }
            })
        };
        let read = |c| {
            let x = read_at(c, v.position, true);
            if let Some((start, end)) = zone.loop_frames {
                if !v.released
                    && zone.loop_crossfade > 0
                    && v.position >= (end - zone.loop_crossfade) as f64
                {
                    let offset = v.position - (end - zone.loop_crossfade) as f64;
                    let t = (offset / zone.loop_crossfade as f64).clamp(0.0, 1.0) as f32;
                    return x * (1.0 - t) + read_at(c, start as f64 + offset, false) * t;
                }
            }
            x
        };
        let mut out = [read(0), read(if sample.channels() == 2 { 1 } else { 0 })];
        let timbre = (controls.2 + v.expression.timbre - 0.5).clamp(0.0, 1.0);
        for (c, x) in out.iter_mut().enumerate() {
            if (timbre - 0.5).abs() > 0.001 {
                let cutoff = (200.0 * 100.0_f32.powf(timbre)).min(sr * 0.45);
                let pole = (-std::f32::consts::TAU * cutoff / sr).exp();
                v.filter[c] = (1.0 - pole) * *x + pole * v.filter[c];
                *x = v.filter[c];
            } else {
                v.filter[c] = *x;
            }
            *x *= v.velocity
                * v.level
                * v.expression.volume
                * (1.0 + 0.5 * controls.1.max(v.expression.pressure));
        }
        let left = out[0] * (1.0 - v.expression.pan) * 2.0;
        let right = out[1] * v.expression.pan * 2.0;
        v.position += increment;
        [left, right]
    }
}
impl<const N: usize> AudioProcessor for MultiSampler<N> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        self.sr = s.sample_rate as f32;
        self.release_step = 1.0 / (self.sr * self.release_ms * 0.001).max(1.0);
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
                        self.channels.values(v.channel),
                        self.release_step,
                        self.quality,
                        self.sr,
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
        self.rr_release.fill(0);
        self.held.fill(Held::default());
        self.sustain.fill(false);
        self.bend.fill(1.0);
        self.channels.reset();
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
    fn sinc_loop_reads_periodic_history_after_first_wrap() {
        let mut pcm = vec![0.0; 300];
        pcm[100..200].fill(1.0);
        let mut z = Zone::single(0, 60);
        z.one_shot = false;
        z.loop_frames = Some((100, 200));
        let bank = Arc::new(
            SampleBank::new(
                vec![Sample::new(pcm, 1, 1000.0, 60).unwrap()],
                vec![z],
                1200,
            )
            .unwrap(),
        );
        let mut sampler = MultiSampler::<1>::new(bank);
        sampler.set_quality(ResampleQuality::SincBest);
        sampler.prepare(spec()).unwrap();
        let mut out = [0.0; 256];
        play(&mut sampler, &[note(0, 127)], &mut out);
        assert!(out[200..].iter().all(|x| (*x - 1.0).abs() < 1e-5));
    }
    #[test]
    fn memory_budget_and_bad_loops() {
        let sample = Sample::new(vec![0.0; 100], 1, 1000.0, 60).unwrap();
        assert!(SampleBank::new(vec![sample], vec![Zone::single(0, 60)], 399).is_err());
    }
}
