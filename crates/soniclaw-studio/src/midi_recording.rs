//! Callback-clock MIDI 1 takes, bounded handoff and transactional note-clip publication.
fn is_false(v: &bool) -> bool {
    !*v
}
use crate::{
    project::{Project, ProjectError},
    *,
};
use soniclaw_clips::Note;
use soniclaw_midi::file::{ClipEvent, MidiClip as Smf, TempoChange};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "is_false")]
    pub looping: bool,
    #[serde(default)]
    pub input_offset_frames: i32,
    pub schema: u32,
    pub track: String,
    pub clip: String,
    pub start_beat: f64,
    pub channel: Option<u8>,
    pub grid_beats: f64,
    pub quantize_ends: bool,
    pub max_seconds: u32,
    pub max_events: usize,
    pub queue_events: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: 1,
            input_offset_frames: 0,
            looping: false,
            track: String::new(),
            clip: "midi-take".into(),
            start_beat: 0.,
            channel: None,
            grid_beats: 0.,
            quantize_ends: false,
            max_seconds: 600,
            max_events: 65536,
            queue_events: 4096,
        }
    }
}
impl Settings {
    pub fn validate(&self, p: &Project) -> Result<(), ProjectError> {
        p.validate()?;
        if self.input_offset_frames.unsigned_abs() > p.document.sample_rate * 2
            || self.schema != 1
            || !valid_id(&self.track)
            || !valid_id(&self.clip)
            || !p
                .document
                .tracks
                .iter()
                .any(|t| t.id == self.track && t.instrument.is_some())
            || !self.start_beat.is_finite()
            || !(0.0..=1_000_000.).contains(&self.start_beat)
            || self.channel.is_some_and(|c| c > 15)
            || !self.grid_beats.is_finite()
            || !(0.0..=16.).contains(&self.grid_beats)
            || !(1..=14400).contains(&self.max_seconds)
            || !(128..=65536).contains(&self.max_events)
            || !(128..=65536).contains(&self.queue_events)
            || self.looping != p.document.timeline.loop_range.is_some()
            || self.looping
                && p.document
                    .timeline
                    .loop_range
                    .as_ref()
                    .is_none_or(|r| (r[0] - self.start_beat).abs() > 1e-9)
        {
            return Err("invalid MIDI take settings; choose an instrument track and match loop mode/start to the arrangement loop".into());
        }
        Ok(())
    }
    pub fn to_json(&self, p: &Project) -> Result<String, ProjectError> {
        self.validate(p)?;
        Ok(serde_json::to_string_pretty(self)?)
    }
    pub fn from_json(s: &str, p: &Project) -> Result<Self, ProjectError> {
        if s.len() > 16384 {
            return Err("MIDI settings exceed 16 KiB".into());
        }
        let v: Self = serde_json::from_str(s)?;
        v.validate(p)?;
        Ok(v)
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Message {
    On {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    Off {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    Control {
        channel: u8,
        controller: u8,
        value: u8,
    },
    Bend {
        channel: u8,
        value: u16,
    },
    Pressure {
        channel: u8,
        value: u8,
    },
    PolyPressure {
        channel: u8,
        key: u8,
        value: u8,
    },
    Panic,
}
impl Message {
    pub(crate) fn from_core(m: MidiMessage) -> Option<Self> {
        Some(match m {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } => Self::On {
                channel,
                key: note,
                velocity,
            },
            MidiMessage::NoteOff {
                channel,
                note,
                velocity,
            } => Self::Off {
                channel,
                key: note,
                velocity,
            },
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => Self::Control {
                channel,
                controller,
                value,
            },
            MidiMessage::PitchBend { channel, value } => Self::Bend { channel, value },
            MidiMessage::ChannelPressure { channel, value } => Self::Pressure { channel, value },
            MidiMessage::PolyPressure {
                channel,
                note,
                value,
            } => Self::PolyPressure {
                channel,
                key: note,
                value,
            },
            MidiMessage::AllNotesOff => Self::Panic,
            _ => return None,
        })
    }
    pub fn to_core(self) -> MidiMessage {
        match self {
            Self::On {
                channel,
                key,
                velocity,
            } => MidiMessage::NoteOn {
                channel,
                note: key,
                velocity,
            },
            Self::Off {
                channel,
                key,
                velocity,
            } => MidiMessage::NoteOff {
                channel,
                note: key,
                velocity,
            },
            Self::Control {
                channel,
                controller,
                value,
            } => MidiMessage::ControlChange {
                channel,
                controller,
                value,
            },
            Self::Bend { channel, value } => MidiMessage::PitchBend { channel, value },
            Self::Pressure { channel, value } => MidiMessage::ChannelPressure { channel, value },
            Self::PolyPressure {
                channel,
                key,
                value,
            } => MidiMessage::PolyPressure {
                channel,
                note: key,
                value,
            },
            Self::Panic => MidiMessage::AllNotesOff,
        }
    }
    fn channel(self) -> Option<u8> {
        match self {
            Self::On { channel, .. }
            | Self::Off { channel, .. }
            | Self::Control { channel, .. }
            | Self::Bend { channel, .. }
            | Self::Pressure { channel, .. }
            | Self::PolyPressure { channel, .. } => Some(channel),
            Self::Panic => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Captured {
    #[serde(default, skip_serializing_if = "is_false")]
    pub chased: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sample: Option<u64>,
    pub sample: u64,
    pub message: Message,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cycle {
    pub sample: u64,
    pub source: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Pass {
    pub start_sample: u64,
    pub end_sample: u64,
    pub source_sample: u64,
}
#[derive(Default)]
struct Shared {
    running: AtomicBool,
    end: AtomicU64,
    lost: AtomicU64,
    unsupported: AtomicU64,
    discontinuities: AtomicU64,
    accepted: AtomicUsize,
    limit: AtomicBool,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Status {
    pub accepted_events: usize,
    pub drained_events: usize,
    pub lost_events: u64,
    pub unsupported_messages: u64,
    pub discontinuities: u64,
    pub end_sample: u64,
    pub limit_reached: bool,
    pub running: bool,
}
pub struct Recorder {
    settings: Settings,
    project_id: String,
    timeline: Timeline,
    rate: u32,
    start: u64,
    shared: Arc<Shared>,
    consumer: rtrb::Consumer<Captured>,
    cycles_consumer: rtrb::Consumer<Cycle>,
    cycles: Vec<Cycle>,
    events: Vec<Captured>,
}
pub struct Processor {
    inner: SessionProcessor,
    producer: rtrb::Producer<Captured>,
    cycles_producer: rtrb::Producer<Cycle>,
    looping: bool,
    elapsed: u64,
    previous_source: Option<u64>,
    shared: Arc<Shared>,
    busy: Arc<AtomicBool>,
    start: u64,
    end: u64,
    expected: u64,
    channel: Option<u8>,
    max_events: usize,
}
pub struct Armed {
    pub controller: Controller,
    pub processor: Processor,
    pub recorder: Recorder,
}
/// Prepare a new, linear MIDI take while stopped. Existing MIDI input is routed only to the selected track.
pub fn prepare(
    p: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    latency_budget: usize,
) -> Result<Armed, ProjectError> {
    prepare_internal(p, factory, settings, Some(latency_budget), None)
}
/// Use precisely the compiled processor latency, without a reserved future-insert delay.
pub fn prepare_low_latency(
    p: &Project,
    factory: &mut impl Factory,
    settings: Settings,
) -> Result<Armed, ProjectError> {
    prepare_internal(p, factory, settings, None, None)
}
/// Monitoring variant; the device must publish the same physical input bus before processing.
pub fn prepare_low_latency_with_inputs(
    p: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    inputs: Option<&crate::inputs::Setup>,
) -> Result<Armed, ProjectError> {
    prepare_internal(p, factory, settings, None, inputs)
}
fn prepare_internal(
    p: &Project,
    factory: &mut impl Factory,
    settings: Settings,
    latency_budget: Option<usize>,
    inputs: Option<&crate::inputs::Setup>,
) -> Result<Armed, ProjectError> {
    settings.validate(p)?;
    let map = TempoMap::compile(&p.document.timeline)?;
    let start = map.sample_at(settings.start_beat, p.document.sample_rate as f64)?;
    let end = start
        .checked_add(settings.max_seconds as u64 * p.document.sample_rate as u64)
        .ok_or("MIDI range overflow")?;
    map.beat_at_sample(end, p.document.sample_rate as f64)?;
    let mut d = p.document.clone();
    for t in &mut d.tracks {
        t.midi_input = t.id == settings.track;
    }
    let prepared = if let Some(inputs) = inputs {
        d.compile_inputs_at(&p.assets, &p.disk, factory, start, inputs)?
    } else {
        d.compile_streamed_at(&p.assets, &p.disk, factory, start)?
    };
    let budget = latency_budget.unwrap_or(prepared.latency_frames());
    let (mut controller, inner) = live_pair(prepared, budget, 1)?;
    controller.seek(start)?;
    controller.set_playing(true)?;
    controller.midi_busy.store(true, Ordering::Release);
    inner.capture.clock_enabled.store(true, Ordering::Release);
    let shared = Arc::new(Shared::default());
    shared.running.store(true, Ordering::Release);
    shared.end.store(start, Ordering::Release);
    let (producer, consumer) = rtrb::RingBuffer::new(settings.queue_events);
    let (cycles_producer, cycles_consumer) = rtrb::RingBuffer::new(4096);
    let processor = Processor {
        cycles_producer,
        looping: settings.looping,
        elapsed: 0,
        previous_source: None,
        inner,
        producer,
        shared: shared.clone(),
        busy: controller.midi_busy.clone(),
        start,
        end,
        expected: start,
        channel: settings.channel,
        max_events: settings.max_events,
    };
    let recorder = Recorder {
        cycles_consumer,
        cycles: Vec::with_capacity(4096),
        events: Vec::with_capacity(settings.max_events),
        settings,
        project_id: p.project_id.clone(),
        timeline: p.document.timeline.clone(),
        rate: p.document.sample_rate,
        start,
        shared,
        consumer,
    };
    Ok(Armed {
        controller,
        processor,
        recorder,
    })
}
impl AudioProcessor for Processor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.inner.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if b.channels() != 2
            || b.frames() > self.inner.spec.max_block_frames
            || ctx.events.len() > 1024
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.inner.process(b, ctx);
        if b.frames() == 0 {
            return;
        }
        let Some((first, playing, _)) = self.inner.capture.clock_at(0) else {
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if !playing {
            return;
        }
        if !self.looping && first != self.expected {
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
        let Some((last, _, _)) = self.inner.capture.clock_at(b.frames() - 1) else {
            self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if self.looping {
            for f in 0..b.frames() {
                let Some((source, playing, looped)) = self.inner.capture.clock_at(f) else {
                    self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                if !playing {
                    self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if self.previous_source.is_some_and(|p| source != p + 1) && !looped {
                    self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                }
                let sample = self
                    .start
                    .saturating_add(self.elapsed)
                    .saturating_add(f as u64);
                if (looped || self.elapsed == 0 && f == 0)
                    && sample < self.end
                    && self.cycles_producer.push(Cycle { sample, source }).is_err()
                {
                    self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                }
                self.previous_source = Some(source);
            }
        }
        self.expected = last.saturating_add(1);
        for e in ctx.events {
            let Some((source_sample, playing, _)) = self.inner.capture.clock_at(e.frame) else {
                self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let sample = if self.looping {
                self.start
                    .saturating_add(self.elapsed)
                    .saturating_add(e.frame as u64)
            } else {
                source_sample
            };
            if !playing || sample < self.start || sample >= self.end {
                continue;
            }
            let Some(message) = Message::from_core(e.message) else {
                self.shared.unsupported.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if self.channel.is_some()
                && message.channel().is_some()
                && self.channel != message.channel()
            {
                continue;
            }
            if self.shared.accepted.load(Ordering::Relaxed) >= self.max_events {
                self.shared.lost.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if self
                .producer
                .push(Captured {
                    chased: false,
                    sample,
                    message,
                    source_sample: if self.looping {
                        Some(source_sample)
                    } else {
                        None
                    },
                })
                .is_err()
            {
                self.shared.lost.fetch_add(1, Ordering::Relaxed);
            } else {
                self.shared.accepted.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.elapsed = self.elapsed.saturating_add(b.frames() as u64);
        let acquisition = if self.looping {
            self.start.saturating_add(self.elapsed)
        } else {
            self.expected
        };
        self.shared
            .end
            .fetch_max(acquisition.min(self.end), Ordering::Release);
        self.shared
            .limit
            .store(acquisition >= self.end, Ordering::Release);
    }
    fn reset(&mut self) {
        self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
        self.inner.reset();
    }
    fn latency_frames(&self) -> usize {
        self.inner.latency_frames()
    }
}
impl Drop for Processor {
    fn drop(&mut self) {
        self.inner
            .capture
            .clock_enabled
            .store(false, Ordering::Release);
        self.busy.store(false, Ordering::Release);
        self.shared.running.store(false, Ordering::Release);
    }
}
impl Recorder {
    pub fn drain(&mut self) -> usize {
        let before = self.events.len();
        for _ in 0..self.cycles_consumer.slots() {
            let Ok(c) = self.cycles_consumer.pop() else {
                break;
            };
            if self.cycles.len() < 4096 {
                self.cycles.push(c);
            } else {
                self.shared.discontinuities.fetch_add(1, Ordering::Relaxed);
            }
        }
        while let Ok(e) = self.consumer.pop() {
            if self.events.len() < self.settings.max_events {
                self.events.push(e);
            } else {
                self.shared.lost.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.events.len() - before
    }
    pub fn status(&self) -> Status {
        Status {
            accepted_events: self.shared.accepted.load(Ordering::Relaxed),
            drained_events: self.events.len(),
            lost_events: self.shared.lost.load(Ordering::Relaxed),
            unsupported_messages: self.shared.unsupported.load(Ordering::Relaxed),
            discontinuities: self.shared.discontinuities.load(Ordering::Relaxed),
            end_sample: self.shared.end.load(Ordering::Acquire),
            limit_reached: self.shared.limit.load(Ordering::Acquire),
            running: self.shared.running.load(Ordering::Acquire),
        }
    }
    /// Control-side partial snapshot. Explicit acceptance is required before applying recovered notes.
    pub fn checkpoint(&mut self, input_dropped: u64) -> Result<Take, ProjectError> {
        self.drain();
        let status = self.status();
        let end = status
            .end_sample
            .max(
                self.cycles
                    .last()
                    .map_or(self.start, |c| c.sample.saturating_add(1)),
            )
            .max(
                self.events
                    .last()
                    .map_or(self.start, |e| e.sample.saturating_add(1)),
            );
        let take = Take {
            schema: 3,
            cycles: self.cycles.clone(),
            project_id: self.project_id.clone(),
            settings: self.settings.clone(),
            timeline: self.timeline.clone(),
            sample_rate: self.rate,
            start_sample: self.start,
            end_sample: end,
            lost_events: status.lost_events,
            unsupported_messages: status.unsupported_messages,
            discontinuities: status.discontinuities,
            input_dropped,
            interrupted: true,
            recovered_from_checkpoint: false,
            events: self.events.clone(),
        };
        take.validate()?;
        Ok(take)
    }
    /// Stop/drop the device processor first. Input bridge loss is supplied by the host's telemetry.
    pub fn finish(&mut self, input_dropped: u64) -> Result<Take, ProjectError> {
        if self.status().running {
            return Err("stop and drop the MIDI audio processor before finalizing".into());
        }
        self.drain();
        let status = self.status();
        let take = Take {
            schema: 3,
            cycles: self.cycles.clone(),
            interrupted: false,
            recovered_from_checkpoint: false,
            project_id: self.project_id.clone(),
            settings: self.settings.clone(),
            timeline: self.timeline.clone(),
            sample_rate: self.rate,
            start_sample: self.start,
            end_sample: status.end_sample,
            lost_events: status.lost_events,
            unsupported_messages: status.unsupported_messages,
            discontinuities: status.discontinuities,
            input_dropped,
            events: self.events.clone(),
        };
        take.validate()?;
        Ok(take)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycles: Vec<Cycle>,
    pub schema: u32,
    pub project_id: String,
    pub settings: Settings,
    pub timeline: Timeline,
    pub sample_rate: u32,
    pub start_sample: u64,
    pub end_sample: u64,
    pub lost_events: u64,
    pub unsupported_messages: u64,
    pub discontinuities: u64,
    pub input_dropped: u64,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub recovered_from_checkpoint: bool,
    pub events: Vec<Captured>,
}
#[derive(Clone, Copy)]
struct Held {
    chased: bool,
    sample: u64,
    velocity: u8,
}
impl Take {
    /// Accept an interrupted, otherwise lossless prefix. Held notes close at the checkpoint boundary.
    pub fn accept_checkpoint(&self) -> Result<Self, ProjectError> {
        self.validate()?;
        if !self.interrupted {
            return Err("take is not an interrupted checkpoint".into());
        }
        let mut next = self.clone();
        next.schema = 3;
        next.interrupted = false;
        next.recovered_from_checkpoint = true;
        if !next.complete() {
            return Err(
                "checkpoint contains input loss or discontinuities; inspect raw data".into(),
            );
        }
        Ok(next)
    }
    pub fn complete(&self) -> bool {
        !self.interrupted
            && self.lost_events == 0
            && self.unsupported_messages == 0
            && self.discontinuities == 0
            && self.input_dropped == 0
    }
    pub fn validate(&self) -> Result<(), ProjectError> {
        let map = TempoMap::compile(&self.timeline)?;
        if self.settings.input_offset_frames.unsigned_abs() > self.sample_rate * 2
            || !matches!(self.schema, 1..=3)
            || self.project_id.is_empty()
            || self.project_id.len() > 128
            || self.settings.schema != 1
            || !valid_id(&self.settings.track)
            || !valid_id(&self.settings.clip)
            || !(8000..=192000).contains(&self.sample_rate)
            || self.settings.looping != self.timeline.loop_range.is_some()
            || self.cycles.len() > 4096
            || self.cycles.windows(2).any(|w| w[0].sample >= w[1].sample)
            || self.cycles.iter().any(|c| {
                c.sample < self.start_sample || c.sample >= self.end_sample || c.source > 1 << 48
            })
            || !(128..=65536).contains(&self.settings.max_events)
            || !(128..=65536).contains(&self.settings.queue_events)
            || !(1..=14400).contains(&self.settings.max_seconds)
            || !self.settings.grid_beats.is_finite()
            || !(0.0..=16.).contains(&self.settings.grid_beats)
            || self.settings.channel.is_some_and(|c| c > 15)
            || self.events.len() > self.settings.max_events
            || self.end_sample < self.start_sample
            || self.end_sample - self.start_sample
                > self.settings.max_seconds as u64 * self.sample_rate as u64
            || map.sample_at(self.settings.start_beat, self.sample_rate as f64)?
                != self.start_sample
        {
            return Err("invalid MIDI take metadata".into());
        }
        let mut last = self.start_sample;
        for e in &self.events {
            if self.complete() && e.sample < last
                || e.sample < self.start_sample
                || e.sample >= self.end_sample
            {
                return Err("invalid/nonmonotonic MIDI take time".into());
            }
            validate_events(
                &[Event {
                    frame: 0,
                    message: e.message.to_core(),
                }],
                1,
            )?;
            if self.settings.channel.is_some()
                && e.message.channel().is_some()
                && self.settings.channel != e.message.channel()
            {
                return Err("take contains messages outside its channel filter".into());
            }
            last = e.sample;
        }
        map.beat_at_sample(self.end_sample, self.sample_rate as f64)?;
        Ok(())
    }
    pub fn to_json(&self) -> Result<String, ProjectError> {
        self.validate()?;
        let s = serde_json::to_string_pretty(self)?;
        if s.len() > 16 * 1024 * 1024 {
            return Err("MIDI take exceeds 16 MiB".into());
        }
        Ok(s)
    }
    pub fn from_json(s: &str) -> Result<Self, ProjectError> {
        if s.len() > 16 * 1024 * 1024 {
            return Err("MIDI take exceeds 16 MiB".into());
        }
        let t: Self = serde_json::from_str(s)?;
        t.validate()?;
        Ok(t)
    }
    /// Save exclusively to a new directory. Raw performance remains separate from editable note/controller projections.
    pub fn save_new(&self, path: &Path) -> Result<(), ProjectError> {
        let text = self.to_json()?;
        std::fs::create_dir(path)?;
        let result = std::fs::write(path.join("midi-take.json"), text);
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(path);
            return Err(e.into());
        }
        Ok(())
    }
    pub fn note_clip(&self, track: u16) -> Result<MidiClip, ProjectError> {
        self.validate()?;
        if self.settings.looping {
            return Err("choose a MIDI loop pass before converting a note clip".into());
        }
        if !self.complete() {
            return Err("MIDI take is incomplete; inspect raw data before applying".into());
        }
        if self.end_sample <= self.start_sample {
            return Err("MIDI take has no duration".into());
        }
        let map = TempoMap::compile(&self.timeline)?;
        let end_beat = map.beat_at_sample(self.end_sample, self.sample_rate as f64)?;
        let length = end_beat - self.settings.start_beat;
        let mut held: [Option<Held>; 2048] = [None; 2048];
        let mut notes = vec![];
        let mut close = |channel: u8, key: u8, h: Held, end: u64| -> Result<(), ProjectError> {
            if end <= h.sample && !h.chased {
                return Ok(());
            }
            let start = h.sample.min(self.end_sample - 1);
            let end = end.max(start + 1).min(self.end_sample);
            let snap = |sample: u64| -> Result<u64, ProjectError> {
                let beat =
                    map.beat_at_sample(sample, self.sample_rate as f64)? - self.settings.start_beat;
                let beat = if self.settings.grid_beats > 0. {
                    (beat / self.settings.grid_beats).round() * self.settings.grid_beats
                } else {
                    beat
                };
                Ok(map.sample_at(
                    self.settings.start_beat + beat.clamp(0., length),
                    self.sample_rate as f64,
                )?)
            };
            let a = snap(start)?.clamp(self.start_sample, self.end_sample - 1);
            let b = if self.settings.quantize_ends {
                snap(end)?
            } else {
                end
            };
            let b = b.clamp(a + 1, self.end_sample);
            let beat = map.beat_at_sample(a, self.sample_rate as f64)?;
            let end = map.beat_at_sample(b, self.sample_rate as f64)?;
            if notes.len() >= 32768 {
                return Err("MIDI note limit exceeded".into());
            }
            notes.push(Note {
                id: notes.len() as u32,
                beat: (beat - self.settings.start_beat).max(0.),
                length: end - beat,
                channel,
                key,
                velocity: h.velocity as f32 / 127.,
            });
            Ok(())
        };
        for captured in &self.events {
            let mut e = *captured;
            let corrected = e.sample as i128 - self.settings.input_offset_frames as i128;
            if corrected >= self.end_sample as i128 && matches!(e.message, Message::On { .. }) {
                continue;
            }
            e.sample = corrected.clamp(self.start_sample as i128, self.end_sample as i128) as u64;
            match e.message {
                Message::On {
                    channel,
                    key,
                    velocity,
                } if velocity > 0 => {
                    let i = channel as usize * 128 + key as usize;
                    if let Some(old) = held[i].replace(Held {
                        sample: e.sample,
                        velocity,
                        chased: e.chased,
                    }) {
                        close(channel, key, old, e.sample)?;
                    }
                }
                Message::On {
                    channel,
                    key,
                    velocity: 0,
                }
                | Message::Off { channel, key, .. } => {
                    if let Some(old) = held[channel as usize * 128 + key as usize].take() {
                        close(channel, key, old, e.sample)?;
                    }
                }
                Message::Panic => {
                    for (i, h) in held.iter_mut().enumerate() {
                        if let Some(old) = h.take() {
                            close((i / 128) as u8, (i % 128) as u8, old, e.sample)?;
                        }
                    }
                }
                Message::Control {
                    channel,
                    controller: 120 | 123,
                    ..
                } => {
                    for key in 0..128 {
                        if let Some(old) = held[channel as usize * 128 + key].take() {
                            close(channel, key as u8, old, e.sample)?;
                        }
                    }
                }
                _ => {}
            }
        }
        for (i, h) in held.iter_mut().enumerate() {
            if let Some(old) = h.take() {
                close(
                    (i / 128) as u8,
                    (i % 128) as u8,
                    old,
                    self.end_sample
                        .saturating_add_signed(-(self.settings.input_offset_frames as i64))
                        .clamp(self.start_sample + 1, self.end_sample),
                )?;
            }
        }
        for note in &mut notes {
            note.length = note.length.min(length - note.beat);
        }
        notes.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.id.cmp(&b.id)));
        let mut controls = vec![];
        for captured in &self.events {
            let corrected = captured.sample as i128 - self.settings.input_offset_frames as i128;
            if corrected >= self.end_sample as i128 {
                continue;
            }
            let sample = corrected.max(self.start_sample as i128) as u64;
            let beat =
                map.beat_at_sample(sample, self.sample_rate as f64)? - self.settings.start_beat;
            let mut push = |channel, control| -> Result<(), ProjectError> {
                if controls.len() >= 65536 {
                    return Err("MIDI controller conversion bound".into());
                }
                controls.push(soniclaw_clips::ControlEvent {
                    id: controls.len() as u32,
                    beat: beat.max(0.).min(length - f64::EPSILON),
                    channel,
                    control,
                });
                Ok(())
            };
            use soniclaw_clips::Control;
            match captured.message {
                Message::Control {
                    channel,
                    controller,
                    value,
                } => push(channel, Control::Cc { controller, value })?,
                Message::Bend { channel, value } => push(channel, Control::Bend { value })?,
                Message::Pressure { channel, value } => push(channel, Control::Pressure { value })?,
                Message::PolyPressure {
                    channel,
                    key,
                    value,
                } => push(channel, Control::PolyPressure { key, value })?,
                Message::Panic => {
                    for channel in 0..16 {
                        push(
                            channel,
                            Control::Cc {
                                controller: 64,
                                value: 0,
                            },
                        )?;
                        push(
                            channel,
                            Control::Cc {
                                controller: 120,
                                value: 0,
                            },
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(MidiClip {
            controls,
            id: self.settings.clip.clone(),
            track,
            start_beat: self.settings.start_beat,
            length_beats: length,
            source_offset: 0.,
            source_length: length,
            looping: false,
            notes,
            enabled: true,
        })
    }
    /// Append without overwriting any prior region. Caller can wrap this in global project History::edit.
    pub fn apply(&self, p: &mut Project) -> Result<(), ProjectError> {
        if self.settings.looping {
            return self.apply_pass(
                p,
                self.passes()?
                    .len()
                    .checked_sub(1)
                    .ok_or("no complete MIDI loop passes")?,
            );
        }
        self.settings.validate(p)?;
        if p.project_id != self.project_id
            || p.document.timeline != self.timeline
            || p.document.sample_rate != self.sample_rate
        {
            return Err("MIDI take belongs to a different project/timeline".into());
        }
        if p.document.tracks.iter().any(|t| {
            t.audio.iter().any(|c| c.id == self.settings.clip)
                || t.midi.iter().any(|c| c.id == self.settings.clip)
                || t.streamed.iter().any(|c| c.id == self.settings.clip)
        }) {
            return Err("MIDI clip identity already exists".into());
        }
        let mut next = p.clone();
        let i = next
            .document
            .tracks
            .iter()
            .position(|t| t.id == self.settings.track)
            .ok_or("missing MIDI track")?;
        let clip = self.note_clip(i as u16)?;
        if clip.notes.is_empty() {
            return Err("MIDI take contains no notes".into());
        }
        next.document.tracks[i].midi.push(clip);
        next.validate()?;
        *p = next;
        Ok(())
    }
    /// Compile the complete candidate before publishing the new region; all work remains off callback.
    pub fn apply_prepared(
        &self,
        p: &mut Project,
        factory: &mut impl Factory,
    ) -> Result<PreparedSession, ProjectError> {
        let mut next = p.clone();
        self.apply(&mut next)?;
        let prepared = next
            .document
            .compile_streamed(&next.assets, &next.disk, factory)?;
        *p = next;
        Ok(prepared)
    }
    /// MIDI 1 controller/pressure/bend data is retained here. Continuous tempo ramps are explicitly rejected.
    pub fn smf(&self, ppqn: u16) -> Result<Vec<u8>, ProjectError> {
        self.validate()?;
        if self.settings.looping {
            return Err("choose a MIDI loop pass before SMF export".into());
        }
        if !self.complete() {
            return Err("incomplete MIDI take cannot be exported as complete SMF".into());
        }
        if ppqn == 0
            || ppqn > 32767
            || self
                .timeline
                .tempo
                .iter()
                .any(|p| p.transition == soniclaw_transport::Transition::Linear)
        {
            return Err(
                "SMF export requires PPQN 1..32767 and stepped tempo; keep JSON for tempo ramps"
                    .into(),
            );
        }
        let map = TempoMap::compile(&self.timeline)?;
        let beat = |sample| -> Result<f64, ProjectError> {
            Ok(
                (map.beat_at_sample(sample, self.sample_rate as f64)? - self.settings.start_beat)
                    .max(0.),
            )
        };
        let tick = |sample: u64| -> Result<u64, ProjectError> {
            Ok((beat(
                sample
                    .saturating_add_signed(-(self.settings.input_offset_frames as i64))
                    .clamp(self.start_sample, self.end_sample),
            )? * ppqn as f64)
                .round() as u64)
        };
        let mut events = vec![];
        for e in &self.events {
            if events.len() + 16 + 32 + self.timeline.tempo.len() > 131072 {
                return Err("MIDI panic expansion exceeds SMF event bound".into());
            }
            if matches!(e.message, Message::Panic) {
                for channel in 0..16 {
                    events.push(ClipEvent {
                        tick: tick(e.sample)?,
                        message: MidiMessage::ControlChange {
                            channel,
                            controller: 123,
                            value: 0,
                        },
                    });
                }
            } else {
                events.push(ClipEvent {
                    tick: tick(e.sample)?,
                    message: e.message.to_core(),
                });
            }
        }
        for channel in 0..16 {
            for controller in [64, 123] {
                events.push(ClipEvent {
                    tick: tick(self.end_sample)?,
                    message: MidiMessage::ControlChange {
                        channel,
                        controller,
                        value: 0,
                    },
                });
            }
        }
        let initial = map.position(self.settings.start_beat)?.bpm;
        let mut tempos = vec![TempoChange {
            tick: 0,
            micros_per_quarter: (60_000_000. / initial).round() as u32,
        }];
        let end = map.beat_at_sample(self.end_sample, self.sample_rate as f64)?;
        for p in &self.timeline.tempo {
            if p.beat > self.settings.start_beat && p.beat < end {
                tempos.push(TempoChange {
                    tick: ((p.beat - self.settings.start_beat) * ppqn as f64).round() as u64,
                    micros_per_quarter: (60_000_000. / p.bpm).round() as u32,
                });
            }
        }
        Ok(Smf {
            ppqn,
            tempos,
            events,
        }
        .write()?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        let (mut d, a) = crate::tests::fixture();
        d.tracks[0].audio.clear();
        d.tracks[0].instrument = Some(Descriptor {
            id: "instrument".into(),
            kind: "sine".into(),
            state: serde_json::json!({}),
        });
        Project::new("MIDI".into(), d, a, Default::default()).unwrap()
    }
    fn render(block: usize, channel: Option<u8>) -> (Project, Take) {
        let p = project();
        let settings = Settings {
            track: "audio".into(),
            clip: "recorded".into(),
            channel,
            ..Default::default()
        };
        let mut armed = prepare(&p, &mut Builtins, settings, 64).unwrap();
        assert!(armed.recorder.finish(0).is_err());
        let replacement = p.document.compile(&p.assets, &mut Builtins).unwrap();
        assert!(armed.controller.replace(replacement).is_err());
        let input = [
            (
                10,
                MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            ),
            (
                20,
                MidiMessage::ControlChange {
                    channel: 0,
                    controller: 64,
                    value: 127,
                },
            ),
            (
                33,
                MidiMessage::PitchBend {
                    channel: 0,
                    value: 9000,
                },
            ),
            (
                75,
                MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 50,
                },
            ),
            (
                150,
                MidiMessage::NoteOn {
                    channel: 1,
                    note: 64,
                    velocity: 90,
                },
            ),
            (
                180,
                MidiMessage::NoteOn {
                    channel: 1,
                    note: 64,
                    velocity: 80,
                },
            ),
            (260, MidiMessage::AllNotesOff),
            (
                400,
                MidiMessage::NoteOn {
                    channel: 1,
                    note: 67,
                    velocity: 110,
                },
            ),
        ];
        let blocks: Vec<_> = (0..512)
            .step_by(block)
            .map(|offset| {
                let n = block.min(512 - offset);
                let events = input
                    .iter()
                    .filter(|(at, _)| *at >= offset && *at < offset + n)
                    .map(|(at, message)| Event {
                        frame: at - offset,
                        message: *message,
                    })
                    .collect::<Vec<_>>();
                (n, events)
            })
            .collect();
        let mut data = vec![0.; 512];
        let allocations = allocation_counter::measure(|| {
            for (n, events) in &blocks {
                data[..n * 2].fill(0.);
                armed.processor.process(
                    &mut AudioBuffer::new(&mut data[..n * 2], 2).unwrap(),
                    &ProcessContext {
                        events,
                        transport: Default::default(),
                    },
                );
            }
        });
        assert_eq!(allocations.count_total, 0);
        drop(armed.processor);
        let take = armed.recorder.finish(0).unwrap();
        assert!(take.complete());
        assert_eq!(take.end_sample, 512);
        (p, take)
    }
    #[test]
    fn sample_offsets_partition_invariance_raw_controllers_and_hanging_notes() {
        let (p, a) = render(64, None);
        let (_, b) = render(127, None);
        assert_eq!(
            a.events.iter().map(|e| e.sample).collect::<Vec<_>>(),
            b.events.iter().map(|e| e.sample).collect::<Vec<_>>()
        );
        let clip = a.note_clip(0).unwrap();
        assert_eq!(clip.notes.len(), 4);
        let map = TempoMap::compile(&a.timeline).unwrap();
        assert_eq!(map.sample_at(clip.notes[0].beat, 48000.).unwrap(), 10);
        assert_eq!(
            map.sample_at(clip.notes[0].beat + clip.notes[0].length, 48000.)
                .unwrap(),
            75
        );
        assert_eq!(
            map.sample_at(clip.notes[3].beat + clip.notes[3].length, 48000.)
                .unwrap(),
            512
        );
        let smf = Smf::read(&a.smf(960).unwrap()).unwrap();
        assert!(smf
            .events
            .iter()
            .any(|e| matches!(e.message, MidiMessage::PitchBend { value: 9000, .. })));
        let json = a.to_json().unwrap();
        let a = Take::from_json(&json).unwrap();
        let mut next = p.clone();
        let mut h = crate::history::History::default();
        h.edit(&mut next, "Record MIDI", |p| {
            drop(a.apply_prepared(p, &mut Builtins)?);
            Ok(())
        })
        .unwrap();
        assert_eq!(next.document.tracks[0].midi.len(), 1);
        h.undo(&mut next).unwrap();
        assert!(next.document.tracks[0].midi.is_empty());
        h.redo(&mut next).unwrap();
        assert_eq!(next.document.tracks[0].midi.len(), 1);
        assert!(a.apply(&mut next).is_err());
    }
    #[test]
    fn filtering_quantization_and_stale_timeline() {
        let (p, mut take) = render(128, Some(1));
        assert!(take
            .events
            .iter()
            .all(|e| e.message.channel().is_none_or(|c| c == 1)));
        take.settings.grid_beats = 0.25;
        take.settings.quantize_ends = true;
        let clip = take.note_clip(0).unwrap();
        assert!(clip
            .notes
            .iter()
            .all(|n| n.length > 0. && n.beat + n.length <= clip.source_length));
        let mut changed = p;
        changed.document.timeline.tempo[0].bpm = 90.;
        let before = changed.document.to_json().unwrap();
        assert!(take.apply(&mut changed).is_err());
        assert_eq!(changed.document.to_json().unwrap(), before);
    }
    #[test]
    fn queue_overflow_unsupported_and_input_loss_never_publish_as_complete() {
        let p = project();
        let settings = Settings {
            track: "audio".into(),
            max_events: 65536,
            queue_events: 128,
            ..Default::default()
        };
        let mut a = prepare(&p, &mut Builtins, settings, 64).unwrap();
        let events = vec![
            Event {
                frame: 0,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100
                }
            };
            129
        ];
        let mut data = vec![0.; 512];
        a.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &events,
                transport: Default::default(),
            },
        );
        drop(a.processor);
        let t = a.recorder.finish(0).unwrap();
        assert_eq!(t.lost_events, 1);
        assert!(!t.complete());
        assert!(t.note_clip(0).is_err());
        assert!(Take::from_json(&t.to_json().unwrap()).is_ok());
        let (_, t) = render(128, None);
        let mut loss = t.clone();
        loss.input_dropped = 1;
        assert!(loss.smf(960).is_err());
        let mut settings = t.settings.clone();
        settings.clip = "unsupported".into();
        let mut a = prepare(&p, &mut Builtins, settings, 64).unwrap();
        a.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[Event {
                    frame: 0,
                    message: MidiMessage::NoteOnId {
                        channel: 0,
                        note: 60,
                        velocity: 0.5,
                        note_id: 7,
                    },
                }],
                transport: Default::default(),
            },
        );
        drop(a.processor);
        let t = a.recorder.finish(0).unwrap();
        assert_eq!(t.unsupported_messages, 1);
        assert!(!t.complete());
    }
    #[test]
    fn ramps_capture_in_beats_but_do_not_silently_approximate_smf() {
        let (_, mut t) = render(128, None);
        t.timeline.tempo[0].transition = soniclaw_transport::Transition::Linear;
        t.timeline.tempo.push(soniclaw_transport::TempoPoint {
            beat: 4.,
            bpm: 180.,
            transition: soniclaw_transport::Transition::Step,
        });
        assert!(t.note_clip(0).is_ok());
        assert!(t.smf(960).is_err());
        assert!(Take::from_json(&" ".repeat(16 * 1024 * 1024 + 1)).is_err());
    }
    #[test]
    fn duration_limit_and_seek_discontinuities_are_visible() {
        let p = project();
        let settings = Settings {
            track: "audio".into(),
            max_seconds: 1,
            ..Default::default()
        };
        let mut a = prepare(&p, &mut Builtins, settings, 64).unwrap();
        let mut data = vec![0.; 512];
        while !a.recorder.status().limit_reached {
            a.processor.process(
                &mut AudioBuffer::new(&mut data, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Default::default(),
                },
            );
        }
        drop(a.processor);
        let t = a.recorder.finish(0).unwrap();
        assert_eq!(t.end_sample, 48000);
        assert!(t.complete());
        let settings = Settings {
            track: "audio".into(),
            ..Default::default()
        };
        let mut a = prepare(&p, &mut Builtins, settings, 64).unwrap();
        let on = [Event {
            frame: 10,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        }];
        a.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &on,
                transport: Default::default(),
            },
        );
        a.controller.seek(100).unwrap();
        a.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &on,
                transport: Default::default(),
            },
        );
        drop(a.processor);
        let t = a.recorder.finish(0).unwrap();
        assert!(!t.complete());
        assert!(t.discontinuities > 0);
        assert!(Take::from_json(&t.to_json().unwrap()).is_ok());
    }
    #[test]
    fn nonzero_start_and_step_tempo_boundaries_keep_absolute_clock() {
        let mut p = project();
        p.document
            .timeline
            .tempo
            .push(soniclaw_transport::TempoPoint {
                beat: 2.005,
                bpm: 60.,
                transition: soniclaw_transport::Transition::Step,
            });
        let settings = Settings {
            track: "audio".into(),
            start_beat: 2.,
            ..Default::default()
        };
        let mut a = prepare(&p, &mut Builtins, settings, 64).unwrap();
        let mut data = vec![0.; 512];
        a.processor.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[
                    Event {
                        frame: 17,
                        message: MidiMessage::NoteOn {
                            channel: 0,
                            note: 60,
                            velocity: 100,
                        },
                    },
                    Event {
                        frame: 201,
                        message: MidiMessage::NoteOff {
                            channel: 0,
                            note: 60,
                            velocity: 0,
                        },
                    },
                ],
                transport: Default::default(),
            },
        );
        drop(a.processor);
        let t = a.recorder.finish(0).unwrap();
        assert_eq!(t.start_sample, 48000);
        assert_eq!(t.events[0].sample, 48017);
        let clip = t.note_clip(0).unwrap();
        let map = TempoMap::compile(&p.document.timeline).unwrap();
        assert_eq!(
            map.sample_at(clip.start_beat + clip.notes[0].beat, 48000.)
                .unwrap(),
            48017
        );
        assert_eq!(
            map.sample_at(
                clip.start_beat + clip.notes[0].beat + clip.notes[0].length,
                48000.
            )
            .unwrap(),
            48201
        );
        let smf = Smf::read(&t.smf(960).unwrap()).unwrap();
        assert_eq!(smf.tempos.len(), 2);
    }
}
