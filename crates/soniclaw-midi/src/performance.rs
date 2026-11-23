//! Fixed-capacity note transforms, arpeggiation and offline beat sequencing.
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
const CAP: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArpMode {
    Off,
    Up,
    Down,
    UpDown,
    Random,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceConfig {
    pub mode: ArpMode,
    pub chord: Vec<i8>,
    pub scale_root: u8,
    pub scale_mask: u16,
    pub step_beats: f64,
    pub gate: f64,
    pub swing: f64,
    pub humanize_ms: f64,
    pub velocity_jitter: u8,
    pub seed: u32,
}
impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            mode: ArpMode::Off,
            chord: vec![0],
            scale_root: 0,
            scale_mask: 0xfff,
            step_beats: 0.25,
            gate: 0.6,
            swing: 0.0,
            humanize_ms: 0.0,
            velocity_jitter: 0,
            seed: 1,
        }
    }
}
impl PerformanceConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.chord.is_empty()
            || self.chord.len() > 8
            || self.chord.iter().any(|x| !(-48..=48).contains(x))
            || self.scale_root > 11
            || self.scale_mask == 0
            || self.scale_mask > 0xfff
            || !self.step_beats.is_finite()
            || !(0.0625..=16.0).contains(&self.step_beats)
            || !self.gate.is_finite()
            || !(0.05..=0.95).contains(&self.gate)
            || !self.swing.is_finite()
            || !(0.0..=0.49).contains(&self.swing)
            || !self.humanize_ms.is_finite()
            || !(0.0..=50.0).contains(&self.humanize_ms)
            || self.velocity_jitter > 64
        {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    pub fn map_key(&self, key: u8) -> u8 {
        for d in 0..12 {
            for n in [key as i16 - d, key as i16 + d] {
                if (0..=127).contains(&n)
                    && self.scale_mask & (1 << ((n - self.scale_root as i16).rem_euclid(12))) != 0
                {
                    return n as u8;
                }
            }
        }
        key
    }
}
#[derive(Clone, Copy)]
struct Pending {
    at: u64,
    message: MidiMessage,
}
const EMPTY: Pending = Pending {
    at: 0,
    message: MidiMessage::AllNotesOff,
};
#[derive(Clone, Copy)]
struct Held {
    used: bool,
    channel: u8,
    key: u8,
    id: i32,
    velocity: u8,
    notes: [u8; 8],
    ids: [i32; 8],
    count: usize,
    onset: u64,
}
impl Default for Held {
    fn default() -> Self {
        Self {
            used: false,
            channel: 0,
            key: 0,
            id: -1,
            velocity: 0,
            notes: [0; 8],
            ids: [-1; 8],
            count: 0,
            onset: 0,
        }
    }
}
pub struct MidiPerformance {
    config: PerformanceConfig,
    held: [Held; 128],
    pending: [Pending; CAP],
    count: usize,
    clock: u64,
    next: f64,
    step: u64,
    random: u32,
    id: i32,
    rate: f64,
    was_playing: bool,
    position: Option<u64>,
    pub overflows: u64,
}
impl MidiPerformance {
    pub fn new(config: PerformanceConfig, rate: f64) -> Result<Self, Error> {
        config.validate()?;
        if !rate.is_finite() || !(8000.0..=192000.0).contains(&rate) {
            return Err(Error::InvalidSpec);
        }
        let seed = config.seed.max(1);
        Ok(Self {
            config,
            held: [Held::default(); 128],
            pending: [EMPTY; CAP],
            count: 0,
            clock: 0,
            next: 0.0,
            step: 0,
            random: seed,
            id: 0,
            rate,
            was_playing: false,
            position: None,
            overflows: 0,
        })
    }
    pub fn reset(&mut self) {
        self.held.fill(Held::default());
        self.count = 0;
        self.clock = 0;
        self.next = 0.0;
        self.step = 0;
        self.random = self.config.seed.max(1);
        self.was_playing = false;
        self.position = None;
    }
    fn random(&mut self) -> f64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        self.random as f64 / u32::MAX as f64
    }
    fn next_id(&mut self) -> i32 {
        self.id = self.id.wrapping_add(1) & 0x7fff_ffff;
        self.id
    }
    fn push(&mut self, at: u64, message: MidiMessage) -> bool {
        if self.count == CAP {
            self.overflows += 1;
            return false;
        }
        let i = self.pending[..self.count].partition_point(|p| p.at <= at);
        self.pending.copy_within(i..self.count, i + 1);
        self.pending[i] = Pending { at, message };
        self.count += 1;
        true
    }
    fn release(&mut self, target: NoteTarget) -> bool {
        for i in 0..128 {
            let h = self.held[i];
            if h.used && target.matches(h.id, h.channel, h.key) {
                self.held[i].used = false;
                if self.config.mode == ArpMode::Off {
                    for j in 0..h.count {
                        if !self.push(
                            self.clock.max(h.onset.saturating_add(1)),
                            MidiMessage::NoteOffId {
                                target: NoteTarget {
                                    note_id: h.ids[j],
                                    channel: h.channel as i16,
                                    key: h.notes[j] as i16,
                                },
                            },
                        ) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
    fn note(&mut self, channel: u8, key: u8, velocity: u8, id: i32) -> bool {
        let Some(slot) = self.held.iter().position(|h| !h.used) else {
            self.overflows += 1;
            return false;
        };
        let mut h = Held {
            used: true,
            channel,
            key,
            id,
            velocity,
            ..Default::default()
        };
        for i in 0..self.config.chord.len() {
            let n = key as i16 + self.config.chord[i] as i16;
            if !(0..=127).contains(&n) {
                continue;
            }
            h.notes[h.count] = self.config.map_key(n as u8);
            h.ids[h.count] = self.next_id();
            h.count += 1;
        }
        if self.config.mode == ArpMode::Off {
            let delay =
                (self.random() * self.config.humanize_ms * self.rate / 1000.0).round() as u64;
            h.onset = self.clock + delay;
            let jitter =
                ((self.random() * 2.0 - 1.0) * self.config.velocity_jitter as f64).round() as i16;
            let v = (velocity as i16 + jitter).clamp(1, 127) as f32 / 127.0;
            for j in 0..h.count {
                if !self.push(
                    h.onset,
                    MidiMessage::NoteOnId {
                        channel,
                        note: h.notes[j],
                        velocity: v,
                        note_id: h.ids[j],
                    },
                ) {
                    return false;
                }
            }
        }
        self.held[slot] = h;
        true
    }
    fn message(&mut self, m: MidiMessage) -> bool {
        match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => self.note(channel, note, velocity, -1),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity,
                note_id,
            } if velocity > 0.0 => self.note(
                channel,
                note,
                (velocity * 127.0).round().clamp(1.0, 127.0) as u8,
                note_id,
            ),
            MidiMessage::NoteOff { channel, note, .. }
            | MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => self.release(NoteTarget {
                channel: channel as i16,
                key: note as i16,
                note_id: -1,
            }),
            MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => {
                self.release(target)
            }
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity: 0.0,
                note_id,
            } => self.release(NoteTarget {
                channel: channel as i16,
                key: note as i16,
                note_id,
            }),
            MidiMessage::AllNotesOff => {
                self.held.fill(Held::default());
                self.count = 0;
                self.push(self.clock, m)
            }
            _ => self.push(self.clock, m),
        }
    }
    fn arp(&mut self, tempo: f64) -> bool {
        let duration = self.rate * 60.0 / tempo * self.config.step_beats;
        if self.clock as f64 + 1e-8 < self.next {
            return true;
        }
        let mut notes = [(0_u8, 0_u8, 0_u8); 1024];
        let mut n = 0;
        for h in &self.held {
            if h.used {
                for j in 0..h.count {
                    notes[n] = (h.notes[j], h.channel, h.velocity);
                    n += 1;
                }
            }
        }
        notes[..n].sort_unstable();
        let swing = if self.step % 2 == 1 {
            duration * self.config.swing
        } else {
            0.0
        };
        if n > 0 {
            let index = match self.config.mode {
                ArpMode::Down => n - 1 - self.step as usize % n,
                ArpMode::UpDown => {
                    let cycle = (2 * n).saturating_sub(2).max(1);
                    let at = self.step as usize % cycle;
                    if at < n {
                        at
                    } else {
                        cycle - at
                    }
                }
                ArpMode::Random => (self.random() * n as f64) as usize % n,
                _ => self.step as usize % n,
            };
            let (note, channel, velocity) = notes[index];
            let delay = self.random() * self.config.humanize_ms * self.rate / 1000.0;
            let at = self.next.round() as u64 + (swing + delay).round() as u64;
            let jitter =
                ((self.random() * 2.0 - 1.0) * self.config.velocity_jitter as f64).round() as i16;
            let velocity = (velocity as i16 + jitter).clamp(1, 127) as f32 / 127.0;
            let note_id = self.next_id();
            if !self.push(
                at,
                MidiMessage::NoteOnId {
                    channel,
                    note,
                    velocity,
                    note_id,
                },
            ) || !self.push(
                at + (duration * self.config.gate).round().max(1.0) as u64,
                MidiMessage::NoteOffId {
                    target: NoteTarget {
                        note_id,
                        channel: channel as i16,
                        key: note as i16,
                    },
                },
            ) {
                return false;
            }
        }
        self.next += duration;
        self.step += 1;
        true
    }
    pub fn process(
        &mut self,
        frames: usize,
        input: &[Event],
        transport: Transport,
        output: &mut [Event],
    ) -> Result<usize, Error> {
        if frames == 0
            || frames > 8192
            || input.len() > 1024
            || output.is_empty()
            || !transport.tempo.is_finite()
            || !(20.0..=400.0).contains(&transport.tempo)
        {
            return Err(Error::InvalidEvents);
        }
        validate_events(input, frames)?;
        let discontinuity = self
            .position
            .is_some_and(|p| p != transport.sample_position);
        if (!transport.playing && self.was_playing) || discontinuity {
            self.reset();
            output[0] = Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            };
            self.position = Some(transport.sample_position + frames as u64);
            self.was_playing = transport.playing;
            return Ok(1);
        }
        self.position = Some(transport.sample_position + frames as u64);
        self.was_playing = transport.playing;
        if !transport.playing {
            return Ok(0);
        }
        let (mut at, mut written) = (0, 0);
        for frame in 0..frames {
            while at < input.len() && input[at].frame == frame {
                if !self.message(input[at].message) {
                    return Ok(self.panic(output, frame));
                }
                at += 1;
            }
            if self.config.mode != ArpMode::Off && !self.arp(transport.tempo) {
                return Ok(self.panic(output, frame));
            }
            while self.count > 0 && self.pending[0].at <= self.clock {
                if written == output.len() {
                    return Ok(self.panic(output, frame));
                }
                output[written] = Event {
                    frame,
                    message: self.pending[0].message,
                };
                written += 1;
                self.pending.copy_within(1..self.count, 0);
                self.count -= 1;
            }
            self.clock += 1;
        }
        Ok(written)
    }
    fn panic(&mut self, output: &mut [Event], frame: usize) -> usize {
        self.overflows += 1;
        self.reset();
        output[0] = Event {
            frame,
            message: MidiMessage::AllNotesOff,
        };
        1
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceNote {
    pub beat: f64,
    pub length: f64,
    pub key: u8,
    pub velocity: u8,
    pub channel: u8,
}
pub fn quantize_sequence(
    notes: &mut [SequenceNote],
    grid: f64,
    strength: f64,
) -> Result<(), Error> {
    if !grid.is_finite()
        || !(0.015625..=16.0).contains(&grid)
        || !strength.is_finite()
        || !(0.0..=1.0).contains(&strength)
        || notes.iter().any(|n| {
            !n.beat.is_finite()
                || n.beat < 0.0
                || !n.length.is_finite()
                || n.length <= 0.0
                || n.key > 127
                || n.velocity == 0
                || n.velocity > 127
                || n.channel > 15
        })
    {
        return Err(Error::InvalidParameter);
    }
    for n in notes {
        n.beat += ((n.beat / grid).round() * grid - n.beat) * strength;
    }
    Ok(())
}
pub fn sequence_events(
    notes: &[SequenceNote],
    rate: f64,
    tempo: f64,
) -> Result<Vec<(u64, MidiMessage)>, Error> {
    if notes.len() > 65536
        || !rate.is_finite()
        || !(8000.0..=192000.0).contains(&rate)
        || !tempo.is_finite()
        || !(20.0..=400.0).contains(&tempo)
    {
        return Err(Error::InvalidSpec);
    }
    let mut checked = notes.to_vec();
    quantize_sequence(&mut checked, 0.25, 0.0)?;
    let f = rate * 60.0 / tempo;
    let mut out = vec![];
    for (i, n) in notes.iter().enumerate() {
        let end = (n.beat + n.length) * f;
        if !end.is_finite() || end > u64::MAX as f64 {
            return Err(Error::InvalidEvents);
        }
        out.push((
            (n.beat * f).round() as u64,
            MidiMessage::NoteOnId {
                channel: n.channel,
                note: n.key,
                velocity: n.velocity as f32 / 127.0,
                note_id: i as i32,
            },
        ));
        out.push((
            end.round() as u64,
            MidiMessage::NoteOffId {
                target: NoteTarget {
                    note_id: i as i32,
                    channel: n.channel as i16,
                    key: n.key as i16,
                },
            },
        ));
    }
    out.sort_by_key(|e| e.0);
    Ok(out)
}
pub struct MidiProcessor {
    inner: Box<dyn AudioProcessor>,
    config: PerformanceConfig,
    performance: Option<MidiPerformance>,
    scratch: Vec<Event>,
}
impl MidiProcessor {
    pub fn new(inner: Box<dyn AudioProcessor>, config: PerformanceConfig) -> Self {
        Self {
            inner,
            config,
            performance: None,
            scratch: vec![],
        }
    }
}
impl AudioProcessor for MidiProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.max_block_frames > 8192 {
            return Err(Error::InvalidSpec);
        }
        self.performance = Some(MidiPerformance::new(self.config.clone(), s.sample_rate)?);
        self.scratch = vec![
            Event {
                frame: 0,
                message: MidiMessage::AllNotesOff
            };
            1024
        ];
        self.inner.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        let Some(p) = &mut self.performance else {
            b.clear();
            return;
        };
        match p.process(b.frames(), c.events, c.transport, &mut self.scratch) {
            Ok(n) => self.inner.process(
                b,
                &ProcessContext {
                    events: &self.scratch[..n],
                    transport: c.transport,
                },
            ),
            Err(_) => b.clear(),
        }
    }
    fn reset(&mut self) {
        if let Some(p) = &mut self.performance {
            p.reset();
        }
        self.inner.reset();
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arp_partition_independence_and_transport_stop() {
        let render = |size: usize| {
            let mut p = MidiPerformance::new(
                PerformanceConfig {
                    mode: ArpMode::Up,
                    chord: vec![0, 4, 7],
                    swing: 0.2,
                    humanize_ms: 2.0,
                    ..Default::default()
                },
                48000.0,
            )
            .unwrap();
            let mut all = vec![];
            let mut at = 0;
            let mut out = [Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            }; 2048];
            while at < 24000 {
                let frames = size.min(24000 - at);
                let e = [Event {
                    frame: 0,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                }];
                let n = p
                    .process(
                        frames,
                        if at == 0 { &e } else { &[] },
                        Transport {
                            sample_position: at as u64,
                            tempo: 120.0,
                            playing: true,
                        },
                        &mut out,
                    )
                    .unwrap();
                all.extend(out[..n].iter().map(|e| (at + e.frame, e.message)));
                at += frames;
            }
            let n = p
                .process(
                    17,
                    &[],
                    Transport {
                        sample_position: 24000,
                        tempo: 120.0,
                        playing: false,
                    },
                    &mut out,
                )
                .unwrap();
            assert_eq!(n, 1);
            assert_eq!(out[0].message, MidiMessage::AllNotesOff);
            all
        };
        assert_eq!(render(257), render(64));
    }
    #[test]
    fn chord_release_keeps_note_identity_after_scale_mapping() {
        let mut p = MidiPerformance::new(
            PerformanceConfig {
                chord: vec![0, 4, 7],
                scale_mask: 0b101010110101,
                ..Default::default()
            },
            48000.0,
        )
        .unwrap();
        let mut out = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }; 16];
        let e = [
            Event {
                frame: 0,
                message: MidiMessage::NoteOnId {
                    channel: 1,
                    note: 61,
                    velocity: 0.8,
                    note_id: 90,
                },
            },
            Event {
                frame: 4,
                message: MidiMessage::NoteOffId {
                    target: NoteTarget {
                        channel: -1,
                        key: -1,
                        note_id: 90,
                    },
                },
            },
        ];
        let n = p
            .process(
                16,
                &e,
                Transport {
                    playing: true,
                    ..Default::default()
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(n, 6);
        for i in 0..3 {
            if let (MidiMessage::NoteOnId { note_id, .. }, MidiMessage::NoteOffId { target }) =
                (out[i].message, out[i + 3].message)
            {
                assert_eq!(note_id, target.note_id)
            } else {
                panic!("note mapping")
            }
        }
    }
}
