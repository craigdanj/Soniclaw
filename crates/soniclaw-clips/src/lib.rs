//! Control-side clip editing and bounded prepared arrangement playback. Audio uses frames; MIDI uses quarter-note beats.
pub mod control;
pub use control::{Control, ControlEvent};
mod audio_edit;
pub use audio_edit::{FadeShape, GainPoint, RegionEdits};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use soniclaw_transport::{TempoMap, Timeline};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
const MAX_FRAME: u64 = 1 << 48;
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn beat(v: f64) -> bool {
    v.is_finite() && (0.0..=1_000_000.).contains(&v)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioClip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<Box<RegionEdits>>,
    pub id: String,
    pub asset: String,
    pub track: u16,
    pub start_frame: u64,
    pub length_frames: u64,
    pub source_offset: u64,
    pub source_length: u64,
    pub looping: bool,
    pub gain: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: u32,
    pub beat: f64,
    pub length: f64,
    pub channel: u8,
    pub key: u8,
    pub velocity: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MidiClip {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ControlEvent>,
    pub id: String,
    pub track: u16,
    pub start_beat: f64,
    pub length_beats: f64,
    pub source_offset: f64,
    pub source_length: f64,
    pub looping: bool,
    pub notes: Vec<Note>,
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub timeline: Timeline,
    pub audio: Vec<AudioClip>,
    pub midi: Vec<MidiClip>,
}
#[derive(Clone, Debug)]
pub struct AudioAsset {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Arc<[f32]>,
}
impl AudioAsset {
    pub fn frames(&self) -> u64 {
        if self.channels == 0 {
            0
        } else {
            (self.samples.len() / self.channels as usize) as u64
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if self.sample_rate == 0
            || self.channels == 0
            || self.channels > 64
            || !self.samples.len().is_multiple_of(self.channels as usize)
            || self.samples.len() > 64 * 1024 * 1024
            || self.samples.iter().any(|v| !v.is_finite())
        {
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
}
pub type Assets = BTreeMap<String, AudioAsset>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Edit {
    SplitAudio {
        id: String,
        new_id: String,
        frame: u64,
    },
    JoinAudio {
        left: String,
        right: String,
    },
    SlipAudio {
        id: String,
        frames: i64,
    },
    GainEnvelope {
        id: String,
        points: Vec<GainPoint>,
    },
    LinkAudio {
        ids: Vec<String>,
        group: Option<String>,
    },
    CrossfadeAudio {
        left: String,
        right: String,
        shape: FadeShape,
    },
    MoveAudio {
        id: String,
        frame: u64,
        snap_frames: u64,
    },
    TrimAudio {
        id: String,
        source_offset: u64,
        length_frames: u64,
    },
    ResizeAudio {
        id: String,
        length_frames: u64,
    },
    FadeAudio {
        id: String,
        fade_in: u64,
        fade_out: u64,
    },
    DuplicateAudio {
        id: String,
        new_id: String,
        start_frame: u64,
    },
    MoveMidi {
        id: String,
        beat: f64,
        snap_beats: f64,
    },
    ResizeMidi {
        id: String,
        length_beats: f64,
    },
    DuplicateMidi {
        id: String,
        new_id: String,
        start_beat: f64,
    },
    UpsertControl {
        clip: String,
        point: ControlEvent,
    },
    RemoveControl {
        clip: String,
        id: u32,
    },
    UpsertNote {
        clip: String,
        note: Note,
    },
    RemoveNote {
        clip: String,
        note: u32,
    },
    RemoveClip {
        id: String,
    },
}
impl Document {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema != 1
            || !(8000..=384000).contains(&self.sample_rate)
            || !(1..=64).contains(&self.channels)
            || self.audio.len() + self.midi.len() > 256
        {
            return Err(Error::InvalidSpec);
        }
        TempoMap::compile(&self.timeline)?;
        let mut ids = BTreeSet::new();
        let mut envelope_points = 0;
        for c in &self.audio {
            if let Some(m) = &c.edits {
                m.validate(c.length_frames, c.fade_in, c.fade_out)?;
                envelope_points += m.envelope.len();
                if envelope_points > 65536 {
                    return Err(Error::InvalidParameter);
                }
            }
            if !valid_id(&c.id)
                || !valid_id(&c.asset)
                || !ids.insert(&c.id)
                || c.track >= 64
                || c.length_frames == 0
                || c.start_frame
                    .checked_add(c.length_frames)
                    .is_none_or(|end| end > MAX_FRAME)
                || c.source_length == 0
                || c.source_offset
                    .checked_add(c.source_length)
                    .is_none_or(|end| end > MAX_FRAME)
                || !c.gain.is_finite()
                || c.gain.abs() > 8.
                || c.fade_in
                    .checked_add(c.fade_out)
                    .is_none_or(|n| n > c.edits.as_ref().map_or(c.length_frames, |m| m.fade_length))
            {
                return Err(Error::InvalidParameter);
            }
        }
        let mut total_notes = 0;
        for c in &self.midi {
            total_notes += c.notes.len() + c.controls.len();
            let mut control_ids = BTreeSet::new();
            for point in &c.controls {
                point.validate(c.source_length)?;
                if !control_ids.insert(point.id) {
                    return Err(Error::InvalidEvents);
                }
            }
            if total_notes > 65536
                || !valid_id(&c.id)
                || !ids.insert(&c.id)
                || c.track >= 64
                || !beat(c.start_beat)
                || !beat(c.length_beats)
                || c.length_beats <= 0.
                || !beat(c.start_beat + c.length_beats)
                || !beat(c.source_length)
                || c.source_length <= 0.
                || !beat(c.source_offset)
                || c.source_offset >= c.source_length
            {
                return Err(Error::InvalidParameter);
            }
            let mut note_ids = BTreeSet::new();
            for n in &c.notes {
                if !note_ids.insert(n.id)
                    || !beat(n.beat)
                    || !beat(n.length)
                    || n.length <= 0.
                    || n.beat + n.length > c.source_length
                    || n.channel >= 16
                    || n.key >= 128
                    || !n.velocity.is_finite()
                    || !(0.0..=1.0).contains(&n.velocity)
                {
                    return Err(Error::InvalidEvents);
                }
            }
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, Error> {
        self.validate()?;
        let text = serde_json::to_string(self).map_err(|_| Error::InvalidParameter)?;
        if text.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidParameter);
        }
        Ok(text)
    }
    pub fn from_json(text: &str) -> Result<Self, Error> {
        if text.len() > 16 * 1024 * 1024 {
            return Err(Error::InvalidParameter);
        }
        let doc: Self = serde_json::from_str(text).map_err(|_| Error::InvalidParameter)?;
        doc.validate()?;
        Ok(doc)
    }
    /// Transactional draft edit. Returns the previous document for application undo/history.
    pub fn apply(&mut self, edit: Edit) -> Result<Document, Error> {
        self.validate()?;
        let mut next = self.clone();
        if audio_edit::apply_extra(&mut next, &edit)? {
            next.validate()?;
            return Ok(std::mem::replace(self, next));
        }
        match edit {
            Edit::UpsertControl { clip, point } => {
                let target = next
                    .midi
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(Error::InvalidParameter)?;
                if let Some(old) = target.controls.iter_mut().find(|p| p.id == point.id) {
                    *old = point;
                } else {
                    target.controls.push(point);
                }
            }
            Edit::RemoveControl { clip, id } => {
                let target = next
                    .midi
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(Error::InvalidParameter)?;
                let index = target
                    .controls
                    .iter()
                    .position(|p| p.id == id)
                    .ok_or(Error::InvalidParameter)?;
                target.controls.remove(index);
            }
            Edit::MoveAudio {
                id,
                frame,
                snap_frames,
            } => {
                let c = next
                    .audio
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?;
                c.start_frame = if snap_frames == 0 {
                    frame
                } else {
                    u64::try_from(
                        (frame as u128 + snap_frames as u128 / 2) / snap_frames as u128
                            * snap_frames as u128,
                    )
                    .map_err(|_| Error::InvalidParameter)?
                };
            }
            Edit::TrimAudio {
                id,
                source_offset,
                length_frames,
            } => {
                if length_frames == 0 || length_frames > MAX_FRAME {
                    return Err(Error::InvalidParameter);
                }
                let c = next
                    .audio
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?;
                if let Some(m) = c.edits.as_deref_mut() {
                    let end_gain = m.envelope_value(length_frames);
                    let start_gain = m.envelope_value(0);
                    let mut points = m
                        .envelope
                        .iter()
                        .filter(|p| {
                            p.frame > m.envelope_offset
                                && p.frame < m.envelope_offset.saturating_add(length_frames)
                        })
                        .map(|p| GainPoint {
                            frame: p.frame - m.envelope_offset,
                            gain: p.gain,
                        })
                        .collect::<Vec<_>>();
                    if !m.envelope.is_empty() {
                        points.insert(
                            0,
                            GainPoint {
                                frame: 0,
                                gain: start_gain,
                            },
                        );
                        points.push(GainPoint {
                            frame: length_frames,
                            gain: end_gain,
                        });
                    }
                    m.envelope = points;
                    m.source_phase = 0;
                    m.fade_offset = 0;
                    m.fade_length = length_frames;
                    m.envelope_offset = 0;
                    m.envelope_length = length_frames;
                }
                c.source_offset = source_offset;
                c.source_length = length_frames;
                c.length_frames = length_frames;
            }
            Edit::ResizeAudio { id, length_frames } => {
                let c = next
                    .audio
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?;
                c.length_frames = length_frames;
                if let Some(m) = c.edits.as_deref_mut() {
                    m.fade_offset = 0;
                    m.fade_length = length_frames;
                }
            }
            Edit::FadeAudio {
                id,
                fade_in,
                fade_out,
            } => {
                let c = next
                    .audio
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?;
                if let Some(m) = c.edits.as_deref_mut() {
                    m.fade_offset = 0;
                    m.fade_length = c.length_frames;
                }
                c.fade_in = fade_in;
                c.fade_out = fade_out;
            }
            Edit::DuplicateAudio {
                id,
                new_id,
                start_frame,
            } => {
                let mut c = next
                    .audio
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?
                    .clone();
                c.id = new_id;
                c.start_frame = start_frame;
                next.audio.push(c);
            }
            Edit::MoveMidi {
                id,
                beat: value,
                snap_beats,
            } => {
                if !beat(value) || !snap_beats.is_finite() || snap_beats < 0. {
                    return Err(Error::InvalidParameter);
                }
                next.midi
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?
                    .start_beat = if snap_beats == 0. {
                    value
                } else {
                    (value / snap_beats).round() * snap_beats
                };
            }
            Edit::ResizeMidi { id, length_beats } => {
                next.midi
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?
                    .length_beats = length_beats
            }
            Edit::DuplicateMidi {
                id,
                new_id,
                start_beat,
            } => {
                let mut c = next
                    .midi
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or(Error::InvalidParameter)?
                    .clone();
                c.id = new_id;
                c.start_beat = start_beat;
                next.midi.push(c);
            }
            Edit::UpsertNote { clip, note } => {
                let c = next
                    .midi
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(Error::InvalidParameter)?;
                if let Some(old) = c.notes.iter_mut().find(|n| n.id == note.id) {
                    *old = note;
                } else {
                    c.notes.push(note);
                }
            }
            Edit::RemoveNote { clip, note } => {
                let c = next
                    .midi
                    .iter_mut()
                    .find(|c| c.id == clip)
                    .ok_or(Error::InvalidParameter)?;
                let index = c
                    .notes
                    .iter()
                    .position(|n| n.id == note)
                    .ok_or(Error::InvalidParameter)?;
                c.notes.remove(index);
            }
            Edit::RemoveClip { id } => {
                let before = next.audio.len() + next.midi.len();
                next.audio.retain(|c| c.id != id);
                next.midi.retain(|c| c.id != id);
                if before == next.audio.len() + next.midi.len() {
                    return Err(Error::InvalidParameter);
                }
            }
            _ => return Err(Error::InvalidParameter),
        }
        next.validate()?;
        Ok(std::mem::replace(self, next))
    }
    pub fn compile(
        &self,
        assets: &Assets,
        max_block_frames: usize,
        instrument: Option<Box<dyn AudioProcessor>>,
    ) -> Result<Player, Error> {
        self.validate()?;
        let spec = ProcessSpec {
            sample_rate: self.sample_rate as f64,
            max_block_frames,
            channels: self.channels as usize,
        };
        spec.validate()?;
        if max_block_frames > 8192 {
            return Err(Error::InvalidSpec);
        }
        let map = TempoMap::compile(&self.timeline)?;
        let mut used = BTreeSet::new();
        let mut samples = 0;
        let mut audio = Vec::new();
        for c in self.audio.iter().filter(|c| c.enabled) {
            let asset = assets.get(&c.asset).ok_or(Error::InvalidParameter)?;
            if asset.sample_rate != self.sample_rate
                || !(asset.channels == 1 || asset.channels == self.channels)
                || c.source_offset + c.source_length > asset.frames()
            {
                return Err(Error::InvalidSpec);
            }
            if used.insert(&c.asset) {
                asset.validate()?;
                samples += asset.samples.len();
                if samples > 64 * 1024 * 1024 {
                    return Err(Error::InvalidBuffer);
                }
            }
            audio.push((c.clone(), asset.clone()));
        }
        let mut spans = Vec::new();
        let mut events = Vec::new();
        let mut expansion_work = 0usize;
        for c in self.midi.iter().filter(|c| c.enabled) {
            if c.notes.is_empty() {
                continue;
            }
            let repeats = if c.looping {
                ((c.length_beats + c.source_offset) / c.source_length).ceil() as usize
            } else {
                1
            };
            expansion_work = expansion_work
                .checked_add(
                    repeats
                        .checked_mul(c.notes.len())
                        .ok_or(Error::InvalidEvents)?,
                )
                .ok_or(Error::InvalidEvents)?;
            if repeats > 1_000_000 || expansion_work > 1_000_000 {
                return Err(Error::InvalidEvents);
            }
            for repeat in 0..repeats {
                let base = repeat as f64 * c.source_length - c.source_offset;
                for n in &c.notes {
                    let a = (base + n.beat).max(0.);
                    let b = (base + n.beat + n.length).min(c.length_beats);
                    if a >= b || a >= c.length_beats || n.velocity == 0. {
                        continue;
                    }
                    let start = map.sample_at(c.start_beat + a, spec.sample_rate)?;
                    let end = map.sample_at(c.start_beat + b, spec.sample_rate)?;
                    if start >= end || spans.len() >= 500_000 {
                        return Err(Error::InvalidEvents);
                    }
                    let id = spans.len() as i32;
                    let on = MidiMessage::NoteOnId {
                        channel: n.channel,
                        note: n.key,
                        velocity: n.velocity,
                        note_id: id,
                    };
                    let off = MidiMessage::NoteOffId {
                        target: NoteTarget {
                            note_id: id,
                            channel: n.channel as i16,
                            key: n.key as i16,
                        },
                    };
                    spans.push(Span {
                        start,
                        end,
                        key_end: end,
                        on,
                        off,
                    });
                    events.push(Scheduled {
                        sample: start,
                        message: on,
                        priority: 4,
                    });
                    events.push(Scheduled {
                        sample: end,
                        message: off,
                        priority: 1,
                    });
                }
            }
        }
        let (controls, control_events) =
            control::Plan::compile(&self.midi, &map, spec.sample_rate)?;
        events.extend(control_events);
        if events.len() > 1_000_000 {
            return Err(Error::InvalidEvents);
        }
        events.sort_by_key(|e| (e.sample, e.priority));
        for span in &mut spans {
            controls.extend(span);
        }
        let mut overlap = Vec::with_capacity(spans.len() * 2);
        for span in &spans {
            overlap.push((span.start, 1i32));
            overlap.push((span.end, -1i32));
        }
        overlap.sort_unstable();
        let mut active = 0;
        for (_, delta) in overlap {
            active += delta;
            if active > 128 {
                return Err(Error::InvalidEvents);
            }
        }
        let event_budget = 766usize
            .saturating_sub(controls.count())
            .saturating_sub(if controls.has_sustain { 128 } else { 0 });
        let mut left = 0;
        let mut voices = 0i32;
        for (right, e) in events.iter().enumerate() {
            while events[left].sample.saturating_add(max_block_frames as u64) <= e.sample {
                left += 1;
            }
            if right - left + 1 > event_budget {
                return Err(Error::InvalidEvents);
            }
            voices += match e.message {
                MidiMessage::NoteOnId { .. } => 1,
                MidiMessage::NoteOffId { .. } => -1,
                _ => 0,
            };
            if !(0..=128).contains(&voices) {
                return Err(Error::InvalidEvents);
            }
        }
        spans.sort_unstable_by_key(|s| s.start);
        let mut tree = Vec::with_capacity(spans.len());
        let root = build_tree(&spans, 0, spans.len(), &mut tree);
        let mut p = Player {
            controls,
            spec,
            audio,
            spans,
            events,
            tree,
            root,
            instrument,
            midi: Vec::with_capacity(1024),
            scratch: Vec::new(),
            delay: Vec::new(),
            delay_cursor: 0,
            delay_filled: 0,
            latency: 0,
            previous_end: None,
            was_playing: false,
            failures: 0,
        };
        p.prepare(spec)?;
        Ok(p)
    }
}
#[derive(Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
    on: MidiMessage,
    off: MidiMessage,
    key_end: u64,
}
#[derive(Clone, Copy)]
struct Scheduled {
    priority: u8,
    sample: u64,
    message: MidiMessage,
}
struct Interval {
    span: usize,
    left: Option<usize>,
    right: Option<usize>,
    max_end: u64,
}
fn build_tree(spans: &[Span], a: usize, b: usize, nodes: &mut Vec<Interval>) -> Option<usize> {
    if a == b {
        return None;
    }
    let middle = (a + b) / 2;
    let left = build_tree(spans, a, middle, nodes);
    let right = build_tree(spans, middle + 1, b, nodes);
    let max_end = left
        .into_iter()
        .chain(right)
        .fold(spans[middle].end, |end, i| end.max(nodes[i].max_end));
    let index = nodes.len();
    nodes.push(Interval {
        span: middle,
        left,
        right,
        max_end,
    });
    Some(index)
}
fn held(
    nodes: &[Interval],
    spans: &[Span],
    root: Option<usize>,
    sample: u64,
    out: &mut Vec<Event>,
) {
    let Some(index) = root else { return };
    let n = &nodes[index];
    if n.max_end <= sample {
        return;
    }
    held(nodes, spans, n.left, sample, out);
    let span = spans[n.span];
    if span.start < sample {
        if span.end > sample {
            out.push(Event {
                frame: 0,
                message: span.on,
            });
            if span.key_end <= sample {
                out.push(Event {
                    frame: 0,
                    message: span.off,
                });
            }
        }
        held(nodes, spans, n.right, sample, out);
    }
}
pub struct Player {
    controls: control::Plan,
    spec: ProcessSpec,
    audio: Vec<(AudioClip, AudioAsset)>,
    spans: Vec<Span>,
    events: Vec<Scheduled>,
    tree: Vec<Interval>,
    root: Option<usize>,
    instrument: Option<Box<dyn AudioProcessor>>,
    midi: Vec<Event>,
    scratch: Vec<f32>,
    delay: Vec<f32>,
    delay_cursor: usize,
    delay_filled: usize,
    latency: usize,
    previous_end: Option<u64>,
    was_playing: bool,
    pub failures: u64,
}
impl Player {
    pub fn last_events(&self) -> &[Event] {
        &self.midi
    }
    pub fn render(
        &mut self,
        buffer: &mut AudioBuffer<'_>,
        transport: Transport,
    ) -> Result<(), Error> {
        buffer.clear();
        self.midi.clear();
        if buffer.channels() != self.spec.channels
            || buffer.frames() > self.spec.max_block_frames
            || !transport.tempo.is_finite()
            || transport.tempo <= 0.
        {
            return Err(Error::InvalidBuffer);
        }
        let start = transport.sample_position;
        let frames = buffer.frames();
        if frames == 0 {
            return Ok(());
        }
        let end = start
            .checked_add(frames as u64)
            .ok_or(Error::InvalidEvents)?;
        self.midi.clear();
        let discontinuity = self.previous_end != Some(start) || !self.was_playing;
        if transport.playing {
            if discontinuity {
                self.midi.push(Event {
                    frame: 0,
                    message: MidiMessage::AllNotesOff,
                });
                self.controls.chase(start, &mut self.midi, false);
                held(&self.tree, &self.spans, self.root, start, &mut self.midi);
                self.controls.chase(start, &mut self.midi, true);
                self.delay_filled = 0;
                self.delay_cursor = 0;
            }
            let a = self.events.partition_point(|e| e.sample < start);
            for e in self.events[a..].iter().take_while(|e| e.sample < end) {
                self.midi.push(Event {
                    frame: (e.sample - start) as usize,
                    message: e.message,
                });
            }
            for i in 1..self.midi.len() {
                let mut j = i;
                while j > 0 && self.midi[j].frame < self.midi[j - 1].frame {
                    self.midi.swap(j, j - 1);
                    j -= 1;
                }
            }
            let channels = self.spec.channels;
            for (clip, asset) in &self.audio {
                let clip_end = clip.start_frame + clip.length_frames;
                let a = start.max(clip.start_frame);
                let b = end.min(clip_end);
                if a >= b {
                    continue;
                }
                for sample in a..b {
                    let local = sample - clip.start_frame;
                    let phase = clip.source_phase() + local;
                    let position = if clip.looping {
                        phase % clip.source_length
                    } else {
                        phase
                    };
                    if position >= clip.source_length {
                        continue;
                    }
                    let gain = clip.region_gain(local);
                    let source = (clip.source_offset + position) as usize * asset.channels as usize;
                    let destination = (sample - start) as usize * channels;
                    for channel in 0..channels {
                        buffer.samples_mut()[destination + channel] += asset.samples
                            [source + if asset.channels == 1 { 0 } else { channel }]
                            * gain;
                    }
                }
            }
        } else if self.was_playing {
            self.controls.reset(&mut self.midi);
            self.midi.push(Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            });
        }
        if !self.delay.is_empty() {
            for sample in buffer.samples_mut() {
                let delayed = if self.delay_filled < self.delay.len() {
                    0.
                } else {
                    self.delay[self.delay_cursor]
                };
                self.delay[self.delay_cursor] = *sample;
                *sample = delayed;
                self.delay_cursor = (self.delay_cursor + 1) % self.delay.len();
                self.delay_filled = (self.delay_filled + 1).min(self.delay.len());
            }
        }
        if let Some(instrument) = &mut self.instrument {
            let scratch = &mut self.scratch[..frames * self.spec.channels];
            scratch.fill(0.);
            instrument.process(
                &mut AudioBuffer::new(scratch, self.spec.channels)?,
                &ProcessContext {
                    events: &self.midi,
                    transport,
                },
            );
            for (out, input) in buffer.samples_mut().iter_mut().zip(scratch) {
                *out += *input;
            }
        }
        self.was_playing = transport.playing;
        self.previous_end = if transport.playing { Some(end) } else { None };
        if buffer.samples().iter().any(|v| !v.is_finite()) {
            buffer.clear();
            return Err(Error::InvalidBuffer);
        }
        Ok(())
    }
}
impl AudioProcessor for Player {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels != self.spec.channels
            || spec.sample_rate != self.spec.sample_rate
            || spec.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        if let Some(instrument) = &mut self.instrument {
            instrument.prepare(spec)?;
            self.latency = instrument.latency_frames();
            if self.latency > 262144 {
                return Err(Error::InvalidSpec);
            }
        }
        self.scratch = vec![0.; self.spec.max_block_frames * self.spec.channels];
        self.delay = vec![0.; self.latency * self.spec.channels];
        self.delay_cursor = 0;
        self.delay_filled = 0;
        self.previous_end = None;
        self.was_playing = false;
        self.midi.clear();
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if !ctx.events.is_empty() || self.render(buffer, ctx.transport).is_err() {
            buffer.clear();
            self.midi.clear();
            self.previous_end = None;
            self.was_playing = false;
            self.delay_filled = 0;
            self.failures = self.failures.saturating_add(1);
        }
    }
    fn reset(&mut self) {
        if let Some(instrument) = &mut self.instrument {
            instrument.reset();
        }
        self.delay_filled = 0;
        self.delay_cursor = 0;
        self.previous_end = None;
        self.was_playing = false;
        self.midi.clear();
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        Document {
            schema: 1,
            sample_rate: 48000,
            channels: 2,
            timeline: Default::default(),
            audio: vec![],
            midi: vec![],
        }
    }
    fn audio() -> AudioClip {
        AudioClip {
            edits: None,
            id: "a".into(),
            asset: "wave".into(),
            track: 0,
            start_frame: 2,
            length_frames: 10,
            source_offset: 0,
            source_length: 4,
            looping: true,
            gain: 0.5,
            fade_in: 2,
            fade_out: 2,
            enabled: true,
        }
    }
    fn midi() -> MidiClip {
        MidiClip {
            controls: vec![],
            id: "m".into(),
            track: 1,
            start_beat: 0.,
            length_beats: 4.,
            source_offset: 0.,
            source_length: 2.,
            looping: true,
            notes: vec![Note {
                id: 7,
                beat: 0.5,
                length: 0.5,
                channel: 0,
                key: 60,
                velocity: 0.8,
            }],
            enabled: true,
        }
    }
    #[test]
    fn fades_loops_mono_mapping_and_callback_partitions_match_without_allocations() {
        let mut doc = document();
        doc.audio.push(audio());
        let assets = BTreeMap::from([(
            "wave".into(),
            AudioAsset {
                sample_rate: 48000,
                channels: 1,
                samples: Arc::from([1., 2., 3., 4.]),
            },
        )]);
        let mut a = doc.compile(&assets, 16, None).unwrap();
        let mut b = doc.compile(&assets, 16, None).unwrap();
        let mut expected = [0.; 32];
        a.render(
            &mut AudioBuffer::new(&mut expected, 2).unwrap(),
            Transport {
                playing: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut output = [0.; 32];
        let measure = allocation_counter::measure(|| {
            for (start, end) in [(0, 1), (1, 8), (8, 11), (11, 16)] {
                b.render(
                    &mut AudioBuffer::new(&mut output[start * 2..end * 2], 2).unwrap(),
                    Transport {
                        sample_position: start as u64,
                        playing: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
        });
        assert_eq!(measure.count_total, 0);
        assert_eq!(expected, output);
        assert_eq!(output[6], 0.5);
        assert_eq!(output[8], 1.5);
        assert_eq!(output[20], 0.25);
        assert_eq!(output[22], 0.);
        assert_eq!(output[6], output[7]);
    }
    #[test]
    fn midi_loop_offsets_seek_catchup_and_stop_are_precise() {
        let mut doc = document();
        doc.midi.push(midi());
        let mut player = doc.compile(&Assets::new(), 256, None).unwrap();
        let mut pcm = [0.; 512];
        player
            .render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 11900,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            player
                .last_events()
                .iter()
                .any(|e| e.frame == 100
                    && matches!(e.message, MidiMessage::NoteOnId { note: 60, .. }))
        );
        player
            .render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 15000,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(matches!(
            player.last_events()[0].message,
            MidiMessage::AllNotesOff
        ));
        assert!(player
            .last_events()
            .iter()
            .any(|e| e.frame == 0 && matches!(e.message, MidiMessage::NoteOnId { .. })));
        player
            .render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 15000,
                    playing: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(player.last_events().len(), 1);
        assert_eq!(player.last_events()[0].message, MidiMessage::AllNotesOff);
        player
            .render(
                &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                Transport {
                    sample_position: 59900,
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(player
            .last_events()
            .iter()
            .any(|e| e.frame == 100 && matches!(e.message, MidiMessage::NoteOnId { .. })));
    }
    #[test]
    fn edits_are_transactional_and_serialization_roundtrips() {
        let mut doc = document();
        doc.audio.push(audio());
        doc.midi.push(midi());
        let text = doc.to_json().unwrap();
        let before = doc
            .apply(Edit::MoveAudio {
                id: "a".into(),
                frame: 13,
                snap_frames: 8,
            })
            .unwrap();
        assert_eq!(doc.audio[0].start_frame, 16);
        assert_eq!(before.audio[0].start_frame, 2);
        let saved = doc.to_json().unwrap();
        assert!(doc
            .apply(Edit::DuplicateAudio {
                id: "a".into(),
                new_id: "m".into(),
                start_frame: 20
            })
            .is_err());
        assert_eq!(doc.to_json().unwrap(), saved);
        assert_eq!(Document::from_json(&text).unwrap().audio[0].start_frame, 2);
    }
    struct Latency {
        delay: [f32; 6],
        cursor: usize,
    }
    impl AudioProcessor for Latency {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for sample in b.samples_mut() {
                let value = self.delay[self.cursor];
                self.delay[self.cursor] = 1.;
                self.cursor = (self.cursor + 1) % 6;
                *sample = value;
            }
        }
        fn reset(&mut self) {
            self.delay.fill(0.);
            self.cursor = 0;
        }
        fn latency_frames(&self) -> usize {
            3
        }
    }
    #[test]
    fn audio_is_compensated_to_instrument_latency() {
        let mut doc = document();
        let mut clip = audio();
        clip.start_frame = 0;
        clip.length_frames = 8;
        clip.fade_in = 0;
        clip.fade_out = 0;
        clip.gain = 1.;
        doc.audio.push(clip);
        let assets = BTreeMap::from([(
            "wave".into(),
            AudioAsset {
                sample_rate: 48000,
                channels: 1,
                samples: Arc::from([1.; 4]),
            },
        )]);
        let mut player = doc
            .compile(
                &assets,
                8,
                Some(Box::new(Latency {
                    delay: [0.; 6],
                    cursor: 0,
                })),
            )
            .unwrap();
        let mut output = [0.; 16];
        player
            .render(
                &mut AudioBuffer::new(&mut output, 2).unwrap(),
                Transport {
                    playing: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(player.latency_frames(), 3);
        assert_eq!(&output[..6], &[0.; 6]);
        assert!(output[6..].iter().all(|v| *v == 2.));
    }
    #[test]
    fn density_and_expansion_limits_reject_unplayable_documents() {
        let mut doc = document();
        let mut clip = midi();
        clip.notes = (0..129)
            .map(|id| Note {
                id,
                beat: 0.,
                length: 1.,
                channel: 0,
                key: 60,
                velocity: 1.,
            })
            .collect();
        doc.midi.push(clip);
        assert!(doc.compile(&Assets::new(), 256, None).is_err());
        doc.midi[0].notes.truncate(1);
        doc.midi[0].source_length = 1e-8;
        doc.midi[0].notes[0].length = 1e-8;
        assert!(doc.compile(&Assets::new(), 256, None).is_err());
    }
}
