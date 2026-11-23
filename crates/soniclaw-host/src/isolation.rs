//! Explicit CLAP child ownership, multi-port RPC and fixed-latency buffered playback.
//! A crash boundary, not an OS security sandbox. All owner methods are control-only.
use super::*;
use serde::{Deserialize, Serialize};
use soniclaw_bus::flex::Layout;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
const MAX_WIRE: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Config {
    pub helper: PathBuf,
    pub module: PathBuf,
    pub plugin_id: String,
    pub sample_rate: u32,
    pub max_frames: usize,
    pub timeout_ms: u64,
    pub recovery_budget: usize,
    pub audio_configuration: Option<u32>,
}
impl Config {
    fn validate(&self) -> Result<(), HostError> {
        if !self.helper.is_absolute()
            || !self.module.is_absolute()
            || !self.helper.is_file()
            || !self.module.is_file()
            || self.plugin_id.is_empty()
            || !(8000..=192000).contains(&self.sample_rate)
            || !(1..=1024).contains(&self.max_frames)
            || !(50..=10000).contains(&self.timeout_ms)
            || !(1..=8).contains(&self.recovery_budget)
        {
            return Err("invalid explicit CLAP isolation configuration".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub schema: u32,
    pub state: Vec<u8>,
}
impl Checkpoint {
    fn validate(&self) -> Result<(), HostError> {
        if self.schema != 1 || self.state.len() > STATE_LIMIT {
            return Err("invalid CLAP checkpoint".into());
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
enum Request {
    Start,
    Stop,
    Save,
    Load(Vec<u8>),
    Block {
        frames: usize,
        inputs: Vec<Vec<f32>>,
        sample: u64,
        tempo: f64,
        playing: bool,
        events: Vec<WireEvent>,
        parameters: Vec<(usize, u32, f64)>,
    },
    Quit,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    error: Option<String>,
    layout: Option<Layout>,
    latency: usize,
    state: Vec<u8>,
    outputs: Vec<Vec<f32>>,
    parameters: Vec<ParameterInfo>,
    events: Vec<WireOutput>,
    requests: u32,
    timer_dispatches: u64,
    fd_dispatches: u64,
}
impl Reply {
    fn ok() -> Self {
        Self {
            error: None,
            layout: None,
            latency: 0,
            state: vec![],
            outputs: vec![],
            parameters: vec![],
            events: vec![],
            requests: 0,
            timer_dispatches: 0,
            fd_dispatches: 0,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct WireEvent {
    frame: usize,
    bytes: [u8; 3],
}
fn encode(e: &Event) -> Result<WireEvent, HostError> {
    let bytes = match e.message {
        MidiMessage::NoteOn {
            channel,
            note,
            velocity,
        } => [0x90 | channel, note, velocity],
        MidiMessage::NoteOff {
            channel,
            note,
            velocity,
        } => [0x80 | channel, note, velocity],
        MidiMessage::ControlChange {
            channel,
            controller,
            value,
        } => [0xb0 | channel, controller, value],
        MidiMessage::PitchBend { channel, value } => {
            [0xe0 | channel, (value & 127) as u8, (value >> 7) as u8]
        }
        _ => return Err("isolated CLAP bridge accepts MIDI1 note/CC/pitch events only".into()),
    };
    Ok(WireEvent {
        frame: e.frame,
        bytes,
    })
}
fn decode(e: &WireEvent) -> Result<Event, HostError> {
    let c = e.bytes[0] & 15;
    let a = e.bytes[1];
    let b = e.bytes[2];
    let message = match e.bytes[0] & 0xf0 {
        0x80 => MidiMessage::NoteOff {
            channel: c,
            note: a,
            velocity: b,
        },
        0x90 => MidiMessage::NoteOn {
            channel: c,
            note: a,
            velocity: b,
        },
        0xb0 => MidiMessage::ControlChange {
            channel: c,
            controller: a,
            value: b,
        },
        0xe0 => MidiMessage::PitchBend {
            channel: c,
            value: a as u16 + ((b as u16) << 7),
        },
        _ => return Err("unsupported MIDI1 event".into()),
    };
    Ok(Event {
        frame: e.frame,
        message,
    })
}
fn read_line<T: for<'de> Deserialize<'de>>(reader: &mut impl BufRead) -> Result<T, HostError> {
    let mut line = Vec::new();
    std::io::Read::take(reader, (MAX_WIRE + 1) as u64).read_until(b'\n', &mut line)?;
    if line.len() > MAX_WIRE || line.last() != Some(&b'\n') {
        return Err("invalid/oversized isolation message".into());
    }
    Ok(serde_json::from_slice(&line)?)
}
fn write_line(value: &impl Serialize, writer: &mut impl Write) -> Result<(), HostError> {
    let line = serde_json::to_vec(value)?;
    if line.len() > MAX_WIRE {
        return Err("isolation message too large".into());
    }
    writer.write_all(&line)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
struct Client {
    parameters: Arc<Vec<ParameterInfo>>,
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Result<Reply, String>>,
    reader: Option<std::thread::JoinHandle<()>>,
    timeout: Duration,
    failed: bool,
}
impl Client {
    fn receive(&mut self) -> Result<Reply, HostError> {
        let result = self
            .replies
            .recv_timeout(self.timeout)
            .map_err(|e| e.to_string())
            .and_then(|r| r)
            .and_then(|r| {
                if let Some(e) = &r.error {
                    Err(e.clone())
                } else {
                    Ok(r)
                }
            });
        match result {
            Ok(r) => Ok(r),
            Err(e) => {
                self.failed = true;
                let _ = self.child.kill();
                Err(e.into())
            }
        }
    }
    fn rpc(&mut self, r: &Request) -> Result<Reply, HostError> {
        if self.failed {
            return Err("isolated CLAP owner faulted; recover explicitly".into());
        }
        if let Err(e) = write_line(r, &mut self.input) {
            self.failed = true;
            let _ = self.child.kill();
            return Err(e);
        }
        self.receive()
    }
    fn spawn(config: &Config) -> Result<(Self, Layout, usize), HostError> {
        config.validate()?;
        let mut cmd = Command::new(&config.helper);
        cmd.arg(&config.module)
            .arg(&config.plugin_id)
            .arg(config.sample_rate.to_string())
            .arg(config.max_frames.to_string())
            .arg(
                config
                    .audio_configuration
                    .map_or("none".into(), |id| id.to_string()),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd.spawn()?;
        let input = child.stdin.take().ok_or("missing child stdin")?;
        let stdout = child.stdout.take().ok_or("missing child stdout")?;
        let (tx, rx) = mpsc::sync_channel(2);
        let reader = std::thread::spawn(move || {
            let mut read = BufReader::new(stdout);
            loop {
                let result = read_line::<Reply>(&mut read).map_err(|e| e.to_string());
                let failed = result.is_err();
                if tx.try_send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut client = Self {
            parameters: Arc::new(vec![]),
            child,
            input,
            replies: rx,
            reader: Some(reader),
            timeout: Duration::from_millis(config.timeout_ms),
            failed: false,
        };
        let hello = client.receive()?;
        let layout = hello.layout.ok_or("missing CLAP layout handshake")?;
        layout.validate()?;
        if layout
            .inputs
            .iter()
            .map(|b| b.channels)
            .sum::<usize>()
            .max(layout.outputs.iter().map(|b| b.channels).sum())
            > 8
        {
            return Err(
                "isolated CLAP flattened bridge supports eight channels per direction".into(),
            );
        }
        if hello.parameters.len() > 4096
            || hello.parameters.iter().any(|p| {
                !p.min.is_finite()
                    || !p.max.is_finite()
                    || p.min > p.max
                    || !p.value.is_finite()
                    || p.name.len() > 1024
            })
        {
            return Err("invalid parameter handshake".into());
        }
        client.parameters = Arc::new(hello.parameters);
        Ok((client, layout, hello.latency))
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(j) = self.reader.take() {
            let _ = j.join();
        }
    }
}
pub struct Isolated {
    revision: u64,
    config: Config,
    client: Arc<Mutex<Client>>,
    layout: Layout,
    latency: usize,
    checkpoint: Option<Checkpoint>,
    recoveries: usize,
    running: Arc<std::sync::atomic::AtomicBool>,
}
pub struct BufferedOwner {
    parameters: Arc<Vec<ParameterInfo>>,
    commands: rtrb::Producer<ParameterEvent>,
    events: rtrb::Consumer<TimedOutput>,
    pub control: Arc<ControlStats>,
    owner: Option<soniclaw_graph::parallel::Owner>,
    client: Arc<Mutex<Client>>,
    running: Arc<std::sync::atomic::AtomicBool>,
}
impl BufferedOwner {
    pub fn parameters(&self) -> &[ParameterInfo] {
        &self.parameters
    }
    /// UI/control thread only; delivered at frame zero of the next worker block.
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), Error> {
        let e = ParameterEvent {
            frame: 0,
            id,
            value,
        };
        validate_parameter(&e, &self.parameters, 1)?;
        self.commands.push(e).map_err(|_| Error::NotPrepared)
    }
    pub fn pop_event(&mut self) -> Option<TimedOutput> {
        self.events.pop().ok()
    }
    pub fn telemetry(&self) -> &soniclaw_graph::parallel::Telemetry {
        &self.owner.as_ref().expect("live owner").telemetry
    }
}
impl Drop for BufferedOwner {
    fn drop(&mut self) {
        drop(self.owner.take());
        if let Ok(mut c) = self.client.lock() {
            let _ = c.rpc(&Request::Stop);
        }
        self.running.store(false, Ordering::Release);
    }
}
impl Isolated {
    /// # Safety
    /// The explicitly selected module/helper must be trusted native code. No security sandbox is imposed.
    pub unsafe fn load(config: Config) -> Result<Self, HostError> {
        let (client, layout, latency) = Client::spawn(&config)?;
        Ok(Self {
            revision: 1,
            config,
            client: Arc::new(Mutex::new(client)),
            layout,
            latency,
            checkpoint: None,
            recoveries: 0,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    pub fn helper_pid(&self) -> Result<u32, HostError> {
        Ok(self
            .client
            .lock()
            .map_err(|_| "child owner poisoned")?
            .child
            .id())
    }
    pub fn faulted(&self) -> bool {
        self.client.lock().map_or(true, |c| c.failed)
    }
    fn stopped(&self) -> Result<(), HostError> {
        if self.running.load(Ordering::Acquire) {
            Err("stop/drop buffered owner before checkpoint or recovery".into())
        } else {
            Ok(())
        }
    }
    pub fn checkpoint(&mut self) -> Result<Checkpoint, HostError> {
        self.stopped()?;
        let cp = Checkpoint {
            schema: 1,
            state: self
                .client
                .lock()
                .map_err(|_| "child owner poisoned")?
                .rpc(&Request::Save)?
                .state,
        };
        cp.validate()?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("isolation revision exhausted")?;
        self.checkpoint = Some(cp.clone());
        Ok(cp)
    }
    pub fn restore_checkpoint(&mut self, cp: Checkpoint) -> Result<(), HostError> {
        self.stopped()?;
        cp.validate()?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("isolation revision exhausted")?;
        let mut client = self.client.lock().map_err(|_| "child owner poisoned")?;
        let r = client.rpc(&Request::Load(cp.state.clone()))?;
        if r.layout.as_ref() != Some(&self.layout) || r.latency != self.latency {
            return Err("restored state changes topology; stage an approved reconfiguration before playback".into());
        }
        client.parameters = Arc::new(r.parameters);
        self.checkpoint = Some(cp);
        Ok(())
    }
    pub fn recover(&mut self) -> Result<(), HostError> {
        self.stopped()?;
        if self.recoveries >= self.config.recovery_budget {
            return Err("CLAP recovery budget exhausted; reconstruct explicitly".into());
        }
        self.recoveries += 1;
        let (mut client, layout, latency) = Client::spawn(&self.config)?;
        if layout != self.layout || latency != self.latency {
            return Err("recovered CLAP layout or latency changed".into());
        }
        if let Some(cp) = &self.checkpoint {
            let r = client.rpc(&Request::Load(cp.state.clone()))?;
            if r.layout.as_ref() != Some(&self.layout) || r.latency != self.latency {
                return Err(
                    "recovered state changes topology; explicit reconfiguration required".into(),
                );
            }
            client.parameters = Arc::new(r.parameters);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("isolation revision exhausted")?;
        self.client = Arc::new(Mutex::new(client));
        Ok(())
    }
    /// Main/aux buses are concatenated in layout order in each interleaved frame.
    /// The common channel width is max(total input,total output); unused output channels are zero.
    pub fn start_buffered(
        &mut self,
        pipeline: usize,
    ) -> Result<(BufferedOwner, Processor), HostError> {
        self.stopped()?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("isolation revision exhausted")?;
        let started = self
            .client
            .lock()
            .map_err(|_| "child owner poisoned")?
            .rpc(&Request::Start)?;
        if started.layout.as_ref() != Some(&self.layout) || started.latency != self.latency {
            let _ = self
                .client
                .lock()
                .map_err(|_| "child owner poisoned")?
                .rpc(&Request::Stop);
            return Err(
                "native topology changed on activation; stage a fresh approved configuration"
                    .into(),
            );
        }
        let channels = self
            .layout
            .inputs
            .iter()
            .map(|b| b.channels)
            .sum::<usize>()
            .max(self.layout.outputs.iter().map(|b| b.channels).sum());
        let spec = ProcessSpec {
            sample_rate: self.config.sample_rate as f64,
            max_block_frames: self.config.max_frames,
            channels,
        };
        let parameters = self
            .client
            .lock()
            .map_err(|_| "child owner poisoned")?
            .parameters
            .clone();
        let (metadata_tx, metadata_rx) = rtrb::RingBuffer::new(64);
        let (commands_tx, commands_rx) = rtrb::RingBuffer::new(64);
        let (events_tx, events_rx) = rtrb::RingBuffer::new(4096);
        let control = Arc::new(ControlStats::default());
        let mut graph = soniclaw_graph::Graph::default();
        let n = graph.add(Flattened {
            metadata: metadata_rx,
            pending: None,
            commands: commands_rx,
            events: events_tx,
            stats: control.clone(),
            pipeline,
            client: self.client.clone(),
            layout: self.layout.clone(),
            channels,
            latency: self.latency,
        });
        graph.set_input(n, 1.)?;
        graph.set_output(n, 1.)?;
        let result = graph.parallel(spec, 1, pipeline);
        match result {
            Ok((owner, bridge)) => {
                self.running.store(true, Ordering::Release);
                Ok((
                    BufferedOwner {
                        parameters: parameters.clone(),
                        commands: commands_tx,
                        events: events_rx,
                        control: control.clone(),
                        owner: Some(owner),
                        client: self.client.clone(),
                        running: self.running.clone(),
                    },
                    Processor {
                        bridge,
                        metadata: metadata_tx,
                        parameters,
                        stats: control,
                        clock: 0,
                        spec,
                    },
                ))
            }
            Err(e) => {
                let _ = self
                    .client
                    .lock()
                    .map_err(|_| "child owner poisoned")?
                    .rpc(&Request::Stop);
                Err(e.into())
            }
        }
    }
}
struct Flattened {
    metadata: rtrb::Consumer<ControlPacket>,
    pending: Option<ControlPacket>,
    commands: rtrb::Consumer<ParameterEvent>,
    events: rtrb::Producer<TimedOutput>,
    stats: Arc<ControlStats>,
    pipeline: usize,
    client: Arc<Mutex<Client>>,
    layout: Layout,
    channels: usize,
    latency: usize,
}
impl AudioProcessor for Flattened {
    fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
        Ok(())
    }
    fn reset(&mut self) {}
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let tag = ctx.transport.sample_position;
        let result = (|| -> Result<(Reply, Transport), HostError> {
            let meta = loop {
                let p = self
                    .pending
                    .take()
                    .or_else(|| self.metadata.pop().ok())
                    .ok_or("missing control packet")?;
                if p.tag == tag {
                    break p;
                }
                if p.tag > tag {
                    self.pending = Some(p);
                    return Err("control packet mismatch".into());
                }
            };
            let mut parameters = Vec::with_capacity(128);
            for _ in 0..64 {
                if let Ok(e) = self.commands.pop() {
                    parameters.push((0, e.id, e.value));
                } else {
                    break;
                }
            }
            parameters.extend(
                meta.parameters[..meta.count]
                    .iter()
                    .map(|e| (e.frame, e.id, e.value)),
            );
            parameters.sort_by_key(|e| e.0);
            let mut offset = 0;
            let inputs = self
                .layout
                .inputs
                .iter()
                .map(|bus| {
                    let mut v = vec![0.; b.frames() * bus.channels];
                    for (f, d) in v.chunks_mut(bus.channels).enumerate() {
                        d.copy_from_slice(
                            &b.samples()[f * self.channels + offset
                                ..f * self.channels + offset + bus.channels],
                        );
                    }
                    offset += bus.channels;
                    v
                })
                .collect();
            let events = ctx
                .events
                .iter()
                .map(encode)
                .collect::<Result<Vec<_>, _>>()?;
            let reply = self
                .client
                .lock()
                .map_err(|_| "child owner poisoned")?
                .rpc(&Request::Block {
                    frames: b.frames(),
                    inputs,
                    sample: meta.transport.sample_position,
                    tempo: meta.transport.tempo,
                    playing: meta.transport.playing,
                    events,
                    parameters,
                })?;
            if reply.events.len() > 1024 {
                return Err("too many isolated output events".into());
            }
            Ok((reply, meta.transport))
        })();
        if result.is_err() {
            self.stats
                .mismatched_packets
                .fetch_add(1, Ordering::Relaxed);
        }
        b.clear();
        if let Ok((reply, transport)) = result {
            self.stats
                .requests
                .fetch_or(reply.requests, Ordering::Release);
            self.stats
                .timer_dispatches
                .fetch_max(reply.timer_dispatches, Ordering::Release);
            self.stats
                .fd_dispatches
                .fetch_max(reply.fd_dispatches, Ordering::Release);
            for wire in reply.events {
                if let Ok(event) = wire.decode(b.frames()) {
                    let frame = match event {
                        OutputEvent::Midi(e) => e.frame,
                        OutputEvent::Parameter(e) => e.frame,
                        OutputEvent::Gesture { frame, .. } => frame,
                    };
                    if self
                        .events
                        .push(TimedOutput {
                            render_frame: tag
                                .saturating_add(self.pipeline as u64)
                                .saturating_add(frame as u64),
                            source_frame: transport.sample_position.saturating_add(frame as u64),
                            event,
                        })
                        .is_err()
                    {
                        self.stats.event_drops.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    self.stats
                        .mismatched_packets
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            if reply.outputs.len() != self.layout.outputs.len() {
                return;
            }
            let mut offset = 0;
            for (bus, source) in self.layout.outputs.iter().zip(reply.outputs) {
                if source.len() != b.frames() * bus.channels {
                    return;
                }
                for (f, s) in source.chunks(bus.channels).enumerate() {
                    b.samples_mut()
                        [f * self.channels + offset..f * self.channels + offset + bus.channels]
                        .copy_from_slice(s);
                }
                offset += bus.channels;
            }
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
enum WireOutput {
    Midi { frame: usize, bytes: [u8; 3] },
    Parameter { frame: usize, id: u32, value: f64 },
    Gesture { frame: usize, id: u32, begin: bool },
}
impl WireOutput {
    fn encode(e: &OutputEvent) -> Result<Self, HostError> {
        Ok(match e {
            OutputEvent::Midi(e) => {
                let w = encode(e)?;
                Self::Midi {
                    frame: w.frame,
                    bytes: w.bytes,
                }
            }
            OutputEvent::Parameter(e) => Self::Parameter {
                frame: e.frame,
                id: e.id,
                value: e.value,
            },
            OutputEvent::Gesture { frame, id, begin } => Self::Gesture {
                frame: *frame,
                id: *id,
                begin: *begin,
            },
        })
    }
    fn decode(self, frames: usize) -> Result<OutputEvent, HostError> {
        let (frame, event) = match self {
            Self::Midi { frame, bytes } => (
                frame,
                OutputEvent::Midi(decode(&WireEvent { frame, bytes })?),
            ),
            Self::Parameter { frame, id, value } => {
                if !value.is_finite() {
                    return Err("invalid output parameter".into());
                }
                (
                    frame,
                    OutputEvent::Parameter(ParameterEvent { frame, id, value }),
                )
            }
            Self::Gesture { frame, id, begin } => {
                (frame, OutputEvent::Gesture { frame, id, begin })
            }
        };
        if frame >= frames {
            return Err("output event exceeds block".into());
        }
        Ok(event)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TimedOutput {
    /// Monotonic buffered render clock, including pipeline delay and event offset.
    pub render_frame: u64,
    /// Original musical sample, including event offset (may wrap/seek).
    pub source_frame: u64,
    pub event: OutputEvent,
}
#[derive(Default)]
pub struct ControlStats {
    pub metadata_drops: std::sync::atomic::AtomicU64,
    pub event_drops: std::sync::atomic::AtomicU64,
    pub mismatched_packets: std::sync::atomic::AtomicU64,
    pub requests: AtomicU32,
    pub timer_dispatches: std::sync::atomic::AtomicU64,
    pub fd_dispatches: std::sync::atomic::AtomicU64,
}
#[derive(Clone, Copy)]
struct ControlPacket {
    tag: u64,
    transport: Transport,
    count: usize,
    parameters: [ParameterEvent; 64],
}
fn validate_parameter(
    e: &ParameterEvent,
    metadata: &[ParameterInfo],
    frames: usize,
) -> Result<(), Error> {
    let p = metadata
        .iter()
        .find(|p| p.id == e.id)
        .ok_or(Error::InvalidParameter)?;
    if e.frame >= frames
        || !e.value.is_finite()
        || !(p.min..=p.max).contains(&e.value)
        || p.flags & CLAP_PARAM_IS_READONLY != 0
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
/// Audio-only fixed-packet adapter. The pipe and JSON protocol stay on the graph worker.
pub struct Processor {
    bridge: soniclaw_graph::parallel::Bridge,
    metadata: rtrb::Producer<ControlPacket>,
    parameters: Arc<Vec<ParameterInfo>>,
    stats: Arc<ControlStats>,
    clock: u64,
    spec: ProcessSpec,
}
impl Processor {
    pub fn parameters(&self) -> &[ParameterInfo] {
        &self.parameters
    }
    pub fn process_automated(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        parameters: &[ParameterEvent],
    ) -> Result<(), Error> {
        let result = (|| {
            if b.channels() != self.spec.channels
                || b.frames() > self.spec.max_block_frames
                || parameters.len() > 64
                || ctx.events.len() > 1024
                || !ctx.transport.tempo.is_finite()
                || !(20.0..=400.).contains(&ctx.transport.tempo)
                || self.clock.checked_add(b.frames() as u64).is_none()
            {
                return Err(Error::InvalidBuffer);
            }
            validate_events(ctx.events, b.frames())?;
            let mut previous = 0;
            for e in parameters {
                validate_parameter(e, &self.parameters, b.frames())?;
                if e.frame < previous {
                    return Err(Error::InvalidEvents);
                }
                previous = e.frame;
            }
            Ok(())
        })();
        if let Err(e) = result {
            b.clear();
            return Err(e);
        }
        if b.frames() == 0 {
            return Ok(());
        }
        let mut packet = ControlPacket {
            tag: self.clock,
            transport: ctx.transport,
            count: parameters.len(),
            parameters: [ParameterEvent {
                frame: 0,
                id: 0,
                value: 0.,
            }; 64],
        };
        packet.parameters[..parameters.len()].copy_from_slice(parameters);
        let dropped = self.metadata.push(packet).is_err();
        if dropped {
            self.stats.metadata_drops.fetch_add(1, Ordering::Relaxed);
            b.clear();
        }
        self.bridge.process(
            b,
            &ProcessContext {
                events: ctx.events,
                transport: Transport {
                    sample_position: self.clock,
                    ..ctx.transport
                },
            },
        );
        self.clock += b.frames() as u64;
        if dropped {
            Err(Error::NotPrepared)
        } else {
            Ok(())
        }
    }
}
impl AudioProcessor for Processor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.bridge.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        let _ = self.process_automated(b, ctx, &[]);
    }
    fn reset(&mut self) {
        self.bridge.reset();
    }
    fn latency_frames(&self) -> usize {
        self.bridge.latency_frames()
    }
}

/// Staged trusted replacement. Inspect topology and prepare explicit routes before committing.
pub struct Candidate {
    base: super::reconfigure::Topology,
    revision: u64,
    replacement: Isolated,
}
pub struct PreparedChange<P = super::reconfigure::RoutedProcessor> {
    base_revision: u64,
    plan: super::reconfigure::Plan,
    replacement: Isolated,
    owner: BufferedOwner,
    processor: P,
}
impl Candidate {
    pub fn topology(&self) -> super::reconfigure::Topology {
        self.replacement.topology()
    }
    pub fn prepare(
        mut self,
        routing: super::reconfigure::Routing,
        pipeline: usize,
        latency_budget: usize,
    ) -> Result<PreparedChange, HostError> {
        let plan = super::reconfigure::Plan {
            schema: 1,
            previous: self.base,
            next: self.replacement.topology(),
            routing,
            pipeline,
            latency_budget,
        };
        plan.validate()?;
        let (owner, processor) = self.replacement.start_buffered(pipeline)?;
        if processor.latency_frames() != pipeline + plan.next.latency {
            return Err("prepared latency differs from approved plan".into());
        }
        let processor = super::reconfigure::RoutedProcessor::new(processor, &plan)?;
        Ok(PreparedChange {
            base_revision: self.revision,
            plan,
            replacement: self.replacement,
            owner,
            processor,
        })
    }
}
impl PreparedChange {
    /// Build and prepare the complete external graph before replacing the current owner.
    pub fn build<P: AudioProcessor>(
        self,
        f: impl FnOnce(
            super::reconfigure::RoutedProcessor,
            &super::reconfigure::Plan,
        ) -> Result<P, Error>,
    ) -> Result<PreparedChange<P>, HostError> {
        let mut processor = f(self.processor, &self.plan)?;
        processor.prepare(ProcessSpec {
            sample_rate: self.plan.next.sample_rate as f64,
            max_block_frames: self.plan.next.max_frames,
            channels: self.plan.routing.channels,
        })?;
        if processor.latency_frames() > self.plan.latency_budget {
            return Err("complete graph exceeds latency budget".into());
        }
        Ok(PreparedChange {
            base_revision: self.base_revision,
            plan: self.plan,
            replacement: self.replacement,
            owner: self.owner,
            processor,
        })
    }
}
impl<P> PreparedChange<P> {
    pub fn plan(&self) -> &super::reconfigure::Plan {
        &self.plan
    }
    /// Current must remain stopped and unchanged since staging. Failure preserves its owner/state.
    pub fn commit(
        self,
        current: &mut Isolated,
    ) -> Result<(BufferedOwner, P, super::reconfigure::Plan), HostError> {
        current.stopped()?;
        if current.revision != self.base_revision || current.topology() != self.plan.previous {
            return Err("stale reconfiguration candidate".into());
        }
        let mut replacement = self.replacement;
        replacement.revision = current
            .revision
            .checked_add(1)
            .ok_or("reconfiguration revision exhausted")?;
        *current = replacement;
        Ok((self.owner, self.processor, self.plan))
    }
}
impl Isolated {
    pub fn topology(&self) -> super::reconfigure::Topology {
        super::reconfigure::Topology {
            layout: self.layout.clone(),
            latency: self.latency,
            sample_rate: self.config.sample_rate,
            max_frames: self.config.max_frames,
        }
    }
    /// Stage only the same explicitly trusted executable/module/product identity; topology settings may change.
    /// Drop the old BufferedOwner first. Healthy current state is captured automatically; fault recovery uses the last checkpoint.
    pub fn stage_reconfiguration(&self, next: Config) -> Result<Candidate, HostError> {
        self.stopped()?;
        next.validate()?;
        if next.helper != self.config.helper
            || next.module != self.config.module
            || next.plugin_id != self.config.plugin_id
        {
            return Err("reconfiguration cannot change trusted native identity".into());
        }
        let (mut client, _, _) = Client::spawn(&next)?;
        let checkpoint = if self.faulted() {
            self.checkpoint
                .clone()
                .ok_or("faulted owner needs an existing checkpoint")?
        } else {
            Checkpoint {
                schema: 1,
                state: self
                    .client
                    .lock()
                    .map_err(|_| "child owner poisoned")?
                    .rpc(&Request::Save)?
                    .state,
            }
        };
        checkpoint.validate()?;
        let r = client.rpc(&Request::Load(checkpoint.state.clone()))?;
        let layout = r.layout.ok_or("missing restored topology")?;
        let latency = r.latency;
        client.parameters = Arc::new(r.parameters);
        let replacement = Isolated {
            config: next,
            client: Arc::new(Mutex::new(client)),
            layout,
            latency,
            checkpoint: Some(checkpoint),
            recoveries: 0,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            revision: 1,
        };
        replacement.topology().validate()?;
        Ok(Candidate {
            base: self.topology(),
            revision: self.revision,
            replacement,
        })
    }
}
impl BufferedOwner {
    pub fn requires_reconfiguration(&self) -> bool {
        self.control.requests.load(Ordering::Acquire) & (1 | 8 | 64) != 0
    }
}

/// Entry point for the dedicated helper executable. Arguments are module, ID, rate, block, config.
/// # Safety
/// The helper loads the selected native module with its ABI/thread contract.
pub unsafe fn helper_main() -> Result<(), HostError> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 6 {
        return Err("usage: clap-isolation-helper MODULE ID RATE MAX CONFIG_OR_none".into());
    }
    let library = unsafe { ClapLibrary::load(Path::new(&args[1])) }?;
    let mut instance = library.instantiate(&args[2])?;
    if args[5] != "none" {
        instance.select_audio_configuration(args[5].parse()?)?;
    }
    let rate: f64 = args[3].parse()?;
    let maximum: usize = args[4].parse()?;
    instance.with_bus_audio(rate, maximum, |_| ())?;
    let layout = instance.bus_layout();
    layout.validate()?;
    let mut reader = BufReader::new(std::io::stdin());
    let mut writer = std::io::stdout();
    let mut hello = Reply::ok();
    hello.layout = Some(layout.clone());
    hello.latency = instance.latency_frames();
    hello.parameters = instance.parameters.clone();
    write_line(&hello, &mut writer)?;
    loop {
        let request: Request = read_line(&mut reader)?;
        match request {
            Request::Quit => return Ok(()),
            Request::Save => {
                let mut r = Reply::ok();
                r.state = instance.save_state()?;
                write_line(&r, &mut writer)?;
            }
            Request::Load(state) => {
                if state.len() > STATE_LIMIT {
                    return Err("state too large".into());
                }
                instance.load_state(&state)?;
                if args[5] != "none" {
                    instance.select_audio_configuration(args[5].parse()?)?;
                }
                instance.with_bus_audio(rate, maximum, |_| ())?;
                let mut r = Reply::ok();
                r.layout = Some(instance.bus_layout());
                r.latency = instance.latency_frames();
                r.parameters = instance.parameters.clone();
                write_line(&r, &mut writer)?;
            }
            Request::Start => {
                let layout = instance.bus_layout();
                let requests = Arc::new(AtomicU32::new(0));
                let observed = requests.clone();
                instance.with_bus_audio_serviced(
                    rate,
                    maximum,
                    requests,
                    |audio| -> Result<(), HostError> {
                        let mut started = Reply::ok();
                        started.layout = Some(layout.clone());
                        started.latency = audio.latency;
                        write_line(&started, &mut writer)?;
                        loop {
                            match read_line::<Request>(&mut reader)? {
                                Request::Stop => break,
                                Request::Block {
                                    frames,
                                    inputs,
                                    sample,
                                    tempo,
                                    playing,
                                    events,
                                    parameters,
                                } => {
                                    if frames == 0 || frames > maximum {
                                        return Err("isolated bus frame count invalid".into());
                                    }
                                    let mut outputs: Vec<Vec<f32>> = layout
                                        .outputs
                                        .iter()
                                        .map(|b| vec![0.; frames * b.channels])
                                        .collect();
                                    let views: Vec<_> = inputs.iter().map(Vec::as_slice).collect();
                                    let mut output: Vec<_> =
                                        outputs.iter_mut().map(Vec::as_mut_slice).collect();
                                    let midi =
                                        events.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
                                    let params: Vec<_> = parameters
                                        .iter()
                                        .map(|&(frame, id, value)| ParameterEvent {
                                            frame,
                                            id,
                                            value,
                                        })
                                        .collect();
                                    audio.process(
                                        &views,
                                        &mut output,
                                        &midi,
                                        &params,
                                        Transport {
                                            sample_position: sample,
                                            tempo,
                                            playing,
                                        },
                                    )?;
                                    let mut r = Reply::ok();
                                    r.outputs = outputs;
                                    r.events = audio
                                        .output_events()
                                        .iter()
                                        .map(WireOutput::encode)
                                        .collect::<Result<Vec<_>, _>>()?;
                                    r.requests = observed.load(Ordering::Acquire);
                                    if let Ok(s) = audio._context.services.lock() {
                                        r.timer_dispatches = s.timer_dispatches;
                                        r.fd_dispatches = s.fd_dispatches;
                                    }
                                    write_line(&r, &mut writer)?;
                                }
                                _ => return Err("stop audio before state/control requests".into()),
                            }
                        }
                        Ok(())
                    },
                )??;
                instance.service_main_thread();
                write_line(&Reply::ok(), &mut writer)?;
            }
            _ => return Err("start audio before rendering".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_gestures_and_parameter_validation_are_bounded() {
        for event in [
            OutputEvent::Gesture {
                frame: 7,
                id: 9,
                begin: true,
            },
            OutputEvent::Gesture {
                frame: 8,
                id: 9,
                begin: false,
            },
            OutputEvent::Parameter(ParameterEvent {
                frame: 12,
                id: 9,
                value: 0.75,
            }),
        ] {
            let wire = WireOutput::encode(&event).unwrap();
            let bytes = serde_json::to_vec(&wire).unwrap();
            let parsed: WireOutput = serde_json::from_slice(&bytes).unwrap();
            assert!(parsed.decode(64).is_ok());
            assert!(parsed.decode(7).is_err());
        }
        let metadata = vec![ParameterInfo {
            id: 9,
            name: "Gain".into(),
            min: 0.,
            max: 1.,
            default: 0.5,
            value: 0.5,
            flags: 0,
        }];
        assert!(validate_parameter(
            &ParameterEvent {
                frame: 32,
                id: 9,
                value: 0.75
            },
            &metadata,
            64
        )
        .is_ok());
        for e in [
            ParameterEvent {
                frame: 64,
                id: 9,
                value: 0.5,
            },
            ParameterEvent {
                frame: 0,
                id: 0,
                value: 0.5,
            },
            ParameterEvent {
                frame: 0,
                id: 9,
                value: f64::NAN,
            },
            ParameterEvent {
                frame: 0,
                id: 9,
                value: 2.,
            },
        ] {
            assert!(validate_parameter(&e, &metadata, 64).is_err());
        }
    }
    #[test]
    fn bounded_protocol_and_midi_conversion() {
        let event = Event {
            frame: 7,
            message: MidiMessage::PitchBend {
                channel: 3,
                value: 12345,
            },
        };
        assert_eq!(decode(&encode(&event).unwrap()).unwrap(), event);
        assert!(encode(&Event {
            frame: 0,
            message: MidiMessage::AllNotesOff
        })
        .is_err());
        let mut writer = Vec::new();
        write_line(
            &Checkpoint {
                schema: 1,
                state: vec![1, 2, 3],
            },
            &mut writer,
        )
        .unwrap();
        let parsed: Checkpoint = read_line(&mut std::io::Cursor::new(writer)).unwrap();
        assert_eq!(parsed.state, vec![1, 2, 3]);
        assert!(read_line::<Reply>(&mut std::io::Cursor::new(b"{} without newline")).is_err());
        assert!(Checkpoint {
            schema: 2,
            state: vec![]
        }
        .validate()
        .is_err());
    }
}
