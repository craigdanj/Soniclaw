//! Prepared musical clock, note identities and bounded native event ingress for buffered hosting.
use super::*;
use soniclaw_core::{NoteExpression, NoteTarget};
use soniclaw_transport::{Clock, Frame, Timeline};
pub use vst3_host::{PluginEvent, PluginEventData};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub timeline: Timeline,
    pub recording: bool,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        soniclaw_transport::TempoMap::compile(&self.timeline)?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ParameterPoint {
    pub frame: usize,
    pub id: u32,
    pub value: f64,
}
#[derive(Clone, Debug)]
pub struct TimedOutput {
    pub sample: u64,
    pub event: PluginEvent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    Invalid,
    Full,
    Stopped,
}
// Payload is inline so the separate producer can run on a MIDI/device thread without allocating.
#[allow(clippy::large_enum_variant)]
enum Command {
    Sysex {
        offset: u16,
        length: u16,
        bytes: [u8; 1024],
    },
    Program {
        offset: u16,
        channel: u8,
        program: u8,
    },
}
pub struct NativeInput {
    tx: rtrb::Producer<Command>,
    shared: Arc<Shared>,
}
impl NativeInput {
    pub fn try_sysex(&mut self, offset: usize, data: &[u8]) -> std::result::Result<(), InputError> {
        if offset >= BLOCK
            || !(2..=1024).contains(&data.len())
            || data[0] != 0xf0
            || data[data.len() - 1] != 0xf7
            || data[1..data.len() - 1].iter().any(|x| *x > 127)
        {
            return Err(InputError::Invalid);
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(InputError::Stopped);
        }
        let mut bytes = [0; 1024];
        bytes[..data.len()].copy_from_slice(data);
        self.tx
            .push(Command::Sysex {
                offset: offset as u16,
                length: data.len() as u16,
                bytes,
            })
            .map_err(|_| InputError::Full)
    }
    pub fn try_program_change(
        &mut self,
        offset: usize,
        channel: u8,
        program: u8,
    ) -> std::result::Result<(), InputError> {
        if offset >= BLOCK || channel > 15 || program > 127 {
            return Err(InputError::Invalid);
        }
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(InputError::Stopped);
        }
        self.tx
            .push(Command::Program {
                offset: offset as u16,
                channel,
                program,
            })
            .map_err(|_| InputError::Full)
    }
}
pub(super) struct WorkerInput {
    rx: rtrb::Consumer<Command>,
    pending: Vec<Command>,
    pub output: rtrb::Producer<TimedOutput>,
}
pub(super) fn queues(
    shared: Arc<Shared>,
) -> (NativeInput, WorkerInput, rtrb::Consumer<TimedOutput>) {
    let (tx, rx) = rtrb::RingBuffer::new(128);
    let (output, out) = rtrb::RingBuffer::new(256);
    (
        NativeInput { tx, shared },
        WorkerInput {
            rx,
            pending: Vec::with_capacity(128),
            output,
        },
        out,
    )
}
pub(super) struct Music {
    pub clock: Clock,
    pub source_end: Option<u64>,
    pub recording: bool,
    pub panic_on_loop: bool,
}
impl Processor {
    pub fn with_music(mut self, config: Config) -> Result<Self> {
        config.validate()?;
        self.music = Some(Music {
            clock: Clock::new(&config.timeline, self.rate)?,
            source_end: None,
            recording: config.recording,
            panic_on_loop: config.timeline.panic_on_loop,
        });
        Ok(self)
    }
    /// Sorted normalized changes within this callback. All copying and queue operations are bounded.
    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[ParameterPoint],
    ) -> std::result::Result<(), Error> {
        if points.len() > 1024
            || points.windows(2).any(|w| w[0].frame > w[1].frame)
            || points.iter().any(|p| {
                p.frame >= b.frames()
                    || !p.value.is_finite()
                    || !(0.0..=1.0).contains(&p.value)
                    || !self.parameters.contains(&p.id)
            })
        {
            b.clear();
            return Err(Error::InvalidEvents);
        }
        self.process_inner(b, ctx, points);
        Ok(())
    }
}
impl Instance {
    pub fn start_buffered_musical(&mut self, config: Config) -> Result<Processor> {
        config.validate()?;
        self.start_buffered()?.with_music(config)
    }
    pub fn take_native_input(&mut self) -> Option<NativeInput> {
        self.native_input.take()
    }
    pub fn drain_timed_output(&mut self) -> Vec<TimedOutput> {
        let mut out = Vec::new();
        if let Some(rx) = &mut self.native_output {
            while let Ok(e) = rx.pop() {
                out.push(e);
            }
        }
        out
    }
}
#[derive(Clone, Copy)]
struct Voice {
    id: i32,
    channel: u8,
    key: u8,
}
pub(super) struct Voices {
    voices: [Option<Voice>; 128],
    next: i32,
}
impl Default for Voices {
    fn default() -> Self {
        Self {
            voices: [None; 128],
            next: 1_000_000,
        }
    }
}
fn event(data: PluginEventData, offset: i32) -> PluginEvent {
    PluginEvent {
        bus_index: 0,
        sample_offset: offset,
        ppq_position: 0.0,
        flags: 0,
        data,
    }
}
impl Voices {
    pub(super) fn send(
        &mut self,
        p: &mut vst3_host::Plugin,
        message: MidiMessage,
        offset: i32,
    ) -> Result<()> {
        let on = match message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => Some((channel, note, velocity as f32 / 127., self.next)),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity,
                note_id,
            } => Some((
                channel,
                note,
                velocity,
                if note_id < 0 { self.next } else { note_id },
            )),
            _ => None,
        };
        if let Some((channel, key, velocity, id)) = on {
            let slot = self
                .voices
                .iter_mut()
                .find(|v| v.is_none())
                .ok_or("VST3 note identity capacity exceeded")?;
            *slot = Some(Voice { id, channel, key });
            self.next = if self.next == i32::MAX {
                1_000_000
            } else {
                self.next + 1
            };
            return p
                .send_plugin_event(event(
                    PluginEventData::NoteOn {
                        channel: channel as i16,
                        pitch: key as i16,
                        tuning: 0.,
                        velocity,
                        length: 0,
                        note_id: id,
                    },
                    offset,
                ))
                .map_err(err);
        }
        let release = match message {
            MidiMessage::NoteOff {
                channel,
                note,
                velocity,
            } => Some((
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: note as i16,
                },
                velocity as f32 / 127.,
            )),
            MidiMessage::NoteOn {
                channel,
                note,
                velocity: 0,
            } => Some((
                NoteTarget {
                    note_id: -1,
                    channel: channel as i16,
                    key: note as i16,
                },
                0.,
            )),
            MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => {
                Some((target, 0.))
            }
            MidiMessage::AllNotesOff => Some((
                NoteTarget {
                    note_id: -1,
                    channel: -1,
                    key: -1,
                },
                0.,
            )),
            _ => None,
        };
        for slot in &mut self.voices {
            let Some(v) = *slot else { continue };
            if let Some((target, velocity)) = release {
                if target.matches(v.id, v.channel, v.key) {
                    p.send_plugin_event(event(
                        PluginEventData::NoteOff {
                            channel: v.channel as i16,
                            pitch: v.key as i16,
                            tuning: 0.,
                            velocity,
                            note_id: v.id,
                        },
                        offset,
                    ))
                    .map_err(err)?;
                    *slot = None;
                }
            }
            if let MidiMessage::NoteExpression {
                target,
                expression,
                value,
            } = message
            {
                if target.matches(v.id, v.channel, v.key) {
                    let data = match expression {
                        NoteExpression::Pressure => PluginEventData::PolyPressure {
                            channel: v.channel as i16,
                            pitch: v.key as i16,
                            pressure: value as f32,
                            note_id: v.id,
                        },
                        _ => {
                            let (type_id, value) = expression_value(expression, value);
                            PluginEventData::NoteExpressionValue {
                                type_id,
                                note_id: v.id,
                                value,
                            }
                        }
                    };
                    p.send_plugin_event(event(data, offset)).map_err(err)?;
                }
            }
        }
        if release.is_none() && !matches!(message, MidiMessage::NoteExpression { .. }) {
            if let Some(m) = convert_midi(message) {
                p.send_midi_event_at(m, offset).map_err(err)?;
            }
        }
        Ok(())
    }
}
fn expression_value(e: NoteExpression, v: f64) -> (u32, f64) {
    match e {
        NoteExpression::Tuning => (2, v / 240. + 0.5),
        NoteExpression::Volume => (0, v / 4.),
        NoteExpression::Pan => (1, v),
        NoteExpression::Timbre => (5, v),
        NoteExpression::Pressure => (u32::MAX, v),
    }
}
pub(super) fn context(f: Frame, recording: bool) -> vst3_host::MusicalTransport {
    vst3_host::MusicalTransport {
        position: TransportPosition {
            samples: f.sample.min(i64::MAX as u64) as i64,
            quarter_notes: f.position.beat,
        },
        tempo: f.position.bpm,
        numerator: f.position.numerator as i32,
        denominator: f.position.denominator as i32,
        bar_start: f.position.beat - f.position.beat_in_bar,
        cycle: f.loop_range,
        playing: f.playing,
        recording: recording && f.punching,
    }
}
pub(super) fn same_span(a: Frame, b: Frame) -> bool {
    !b.looped
        && a.playing == b.playing
        && a.punching == b.punching
        && a.position.bpm == b.position.bpm
        && a.position.numerator == b.position.numerator
        && a.position.denominator == b.position.denominator
        && a.position.bar == b.position.bar
}
pub(super) fn process_packet(
    p: &mut vst3_host::Plugin,
    packet: &mut Packet,
    buffers: &mut vst3_host::AudioBuffers,
    voices: &mut Voices,
    native: &mut WorkerInput,
) -> Result<()> {
    native.pending.clear();
    for _ in 0..128 {
        match native.rx.pop() {
            Ok(c) => native.pending.push(c),
            Err(_) => break,
        }
    }
    let mut start = 0;
    while start < BLOCK {
        if packet.discontinuities[start] {
            voices.send(p, MidiMessage::AllNotesOff, 0)?;
        }
        let frame = packet.music[start];
        if let Some(f) = frame {
            p.set_musical_transport(context(f, packet.recording))
                .map_err(err)?;
            if f.looped && packet.panic_on_loop {
                voices.send(p, MidiMessage::AllNotesOff, 0)?;
            }
        } else {
            p.set_tempo(packet.bpm).map_err(err)?;
            p.set_playing(packet.playing).map_err(err)?;
            p.set_transport_position(TransportPosition {
                samples: packet
                    .sample
                    .saturating_add(start as u64)
                    .min(i64::MAX as u64) as i64,
                quarter_notes: packet.beat + start as f64 * packet.bpm / 60. / buffers.sample_rate,
            })
            .map_err(err)?;
        }
        let mut end = start + 1;
        while end < BLOCK
            && !packet.discontinuities[end]
            && match (frame, packet.music[end]) {
                (Some(a), Some(b)) => {
                    same_span(a, b)
                        && b.sample == a.sample + if a.playing { (end - start) as u64 } else { 0 }
                }
                (None, None) => true,
                _ => false,
            }
        {
            end += 1;
        }
        let len = end - start;
        buffers.block_size = len;
        for c in 0..2 {
            buffers.inputs[c].resize(len, 0.);
            buffers.outputs[c].resize(len, 0.);
            for i in 0..len {
                buffers.inputs[c][i] = packet.samples[(start + i) * 2 + c];
            }
            buffers.outputs[c].fill(0.);
        }
        for e in &packet.events[..packet.count] {
            if (start..end).contains(&e.frame) {
                voices.send(p, e.message, (e.frame - start) as i32)?;
            }
        }
        for point in &packet.parameters[..packet.parameter_count] {
            if (start..end).contains(&point.frame) {
                p.queue_processor_parameter_at(point.id, point.value, (point.frame - start) as i32)
                    .map_err(err)?;
            }
        }
        for c in &native.pending {
            match c {
                Command::Sysex {
                    offset,
                    length,
                    bytes,
                } if (start..end).contains(&(*offset as usize)) => p
                    .send_sysex_at(
                        bytes[..*length as usize].to_vec(),
                        (*offset as usize - start) as i32,
                    )
                    .map_err(err)?,
                Command::Program {
                    offset,
                    channel,
                    program,
                } if (start..end).contains(&(*offset as usize)) => p
                    .send_midi_event_at(
                        vst3_host::MidiEvent::ProgramChange {
                            channel: vst3_host::MidiChannel::from_index(*channel).unwrap(),
                            program: *program,
                        },
                        (*offset as usize - start) as i32,
                    )
                    .map_err(err)?,
                _ => {}
            }
        }
        p.process_audio(buffers).map_err(err)?;
        for c in 0..2 {
            for i in 0..len {
                packet.samples[(start + i) * 2 + c] = buffers.outputs[c][i];
            }
        }
        for mut e in p.take_output_events() {
            let absolute = packet
                .sample
                .saturating_add(start as u64)
                .saturating_add(e.sample_offset.max(0) as u64);
            e.sample_offset += start as i32;
            native
                .output
                .push(TimedOutput {
                    sample: absolute,
                    event: e,
                })
                .map_err(|_| err("VST3 output event queue full"))?;
        }
        start = end;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expression_laws() {
        assert_eq!(expression_value(NoteExpression::Tuning, 12.), (2, 0.55));
        assert_eq!(expression_value(NoteExpression::Volume, 1.), (0, 0.25));
    }
    #[test]
    fn native_ingress_is_bounded_and_allocation_free() {
        let shared = Arc::new(Shared::default());
        let (mut input, _worker, _out) = queues(shared.clone());
        let m = allocation_counter::measure(|| {
            for _ in 0..128 {
                input.try_sysex(9, &[0xf0, 0x7d, 1, 0xf7]).unwrap();
            }
            assert_eq!(input.try_program_change(0, 0, 0), Err(InputError::Full));
        });
        assert_eq!(m.count_total, 0);
        shared.stop.store(true, Ordering::Release);
        assert_eq!(input.try_sysex(0, &[0xf0, 0xf7]), Err(InputError::Stopped));
        assert_eq!(
            input.try_sysex(0, &[0xf0, 128, 0xf7]),
            Err(InputError::Invalid)
        );
    }
    #[test]
    fn musical_context_wraps_in_buffer_and_preserves_meter() {
        let mut timeline = Timeline {
            loop_range: Some([0., 0.02]),
            meters: vec![soniclaw_transport::MeterPoint {
                beat: 0.,
                numerator: 3,
                denominator: 4,
            }],
            ..Default::default()
        };
        timeline.punch_range = Some([0.005, 0.015]);
        let mut c = Clock::new(&timeline, 48000.).unwrap();
        c.set_playing(true);
        c.seek(479).unwrap();
        let a = context(c.frame().unwrap(), true);
        assert_eq!(a.numerator, 3);
        assert!(!a.recording);
        c.advance(1).unwrap();
        let f = c.frame().unwrap();
        assert!(f.looped);
        assert_eq!(f.sample, 0);
        assert_eq!(context(f, true).cycle, Some([0., 0.02]));
    }
}
