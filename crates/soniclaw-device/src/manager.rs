//! Control-thread output device selection, saved preferences and restart policy.
//! Poll from a main/control loop; callbacks only update atomic Telemetry counters.
use super::*;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSelector {
    pub name: String,
    pub occurrence: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuplexRoute {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cue_outputs: Vec<[u16; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_count: Option<u16>,
    pub input: Option<DeviceSelector>,
    pub use_default_input: bool,
    pub input_channels: [u16; 2],
    pub output_channels: [u16; 2],
}
impl Default for DuplexRoute {
    fn default() -> Self {
        Self {
            cue_outputs: Vec::new(),
            input_count: None,
            input: None,
            use_default_input: true,
            input_channels: [0, 0],
            output_channels: [0, 1],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevicePreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<DuplexRoute>,
    pub schema_version: u32,
    /// None tracks the current platform default host/device.
    pub host: Option<String>,
    pub output: Option<DeviceSelector>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    /// None lets the backend choose. A fixed size is a request, not measured latency.
    pub buffer_frames: Option<u32>,
    pub max_internal_frames: usize,
    pub reconnect: bool,
    pub fallback_to_default: bool,
    pub retry_ms: u32,
}
impl Default for DevicePreferences {
    fn default() -> Self {
        Self {
            duplex: None,
            schema_version: 1,
            host: None,
            output: None,
            sample_rate: None,
            channels: None,
            buffer_frames: None,
            max_internal_frames: 256,
            reconnect: true,
            fallback_to_default: false,
            retry_ms: 1000,
        }
    }
}
impl DevicePreferences {
    pub fn validate(&self) -> Result<(), DeviceError> {
        if self.duplex.as_ref().is_some_and(|d| {
            OutputMap::new(64, d.output_channels, &d.cue_outputs).is_err()
                || d.input_count.is_some_and(|n| !(1..=64).contains(&n))
                || d.input_channels
                    .iter()
                    .chain(&d.output_channels)
                    .any(|n| *n >= 64)
                || d.output_channels[0] == d.output_channels[1]
                || d.input
                    .as_ref()
                    .is_some_and(|s| s.name.is_empty() || s.name.len() > 1024 || s.occurrence > 255)
        }) || self.schema_version != 1
            || self
                .host
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 256)
            || self
                .output
                .as_ref()
                .is_some_and(|d| d.name.is_empty() || d.name.len() > 1024 || d.occurrence > 255)
            || self
                .sample_rate
                .is_some_and(|r| !(8000..=384000).contains(&r))
            || self.channels.is_some_and(|n| !(1..=64).contains(&n))
            || self
                .buffer_frames
                .is_some_and(|n| !(16..=65536).contains(&n))
            || !(1..=8192).contains(&self.max_internal_frames)
            || !(250..=30000).contains(&self.retry_ms)
        {
            return Err("invalid audio device preferences".into());
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DeviceError> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(16385)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16384 {
            return Err("device settings exceed 16 KiB".into());
        }
        let p: Self = serde_json::from_slice(&bytes)?;
        p.validate()?;
        Ok(p)
    }
    /// Explicit settings save; no audio-device or project state is changed.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), DeviceError> {
        use std::io::Write;
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut f = std::fs::File::create(path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    F32,
    I16,
    U16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRange {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub buffer_range: Option<(u32, u32)>,
    pub format: OutputFormat,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputDevice {
    pub selector: DeviceSelector,
    pub configs: Vec<OutputRange>,
    pub default_rate: u32,
    pub default_channels: u16,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInventory {
    #[serde(default)]
    pub inputs: Vec<OutputDevice>,
    #[serde(default)]
    pub default_input: Option<DeviceSelector>,
    pub host: String,
    pub hosts: Vec<String>,
    pub outputs: Vec<OutputDevice>,
    pub default_output: Option<DeviceSelector>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputPlan {
    pub duplex: Option<DuplexRoute>,
    pub hardware_channels: u16,
    pub host: String,
    pub output: DeviceSelector,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_frames: Option<u32>,
    pub max_internal_frames: usize,
    pub format: OutputFormat,
    pub used_fallback: bool,
}
impl OutputPlan {
    pub fn spec(&self) -> ProcessSpec {
        ProcessSpec {
            sample_rate: self.sample_rate as f64,
            channels: self.channels as usize,
            max_block_frames: self.max_internal_frames,
        }
    }
}
pub fn negotiate_output(
    p: &DevicePreferences,
    c: &OutputInventory,
) -> Result<OutputPlan, DeviceError> {
    p.validate()?;
    if p.host.as_ref().is_some_and(|h| h != &c.host) {
        return Err("requested audio backend unavailable".into());
    }
    let requested = p.output.as_ref().or(c.default_output.as_ref());
    let mut fallback = false;
    let device = c
        .outputs
        .iter()
        .find(|d| Some(&d.selector) == requested)
        .or_else(|| {
            if p.output.is_some() && p.fallback_to_default {
                fallback = true;
                c.outputs
                    .iter()
                    .find(|d| Some(&d.selector) == c.default_output.as_ref())
            } else {
                None
            }
        })
        .ok_or("requested output device unavailable")?;
    let rate = p.sample_rate.unwrap_or(device.default_rate);
    let route = p.duplex.clone();
    let needed = route.as_ref().map(|d| {
        d.output_channels
            .iter()
            .chain(d.cue_outputs.iter().flatten())
            .copied()
            .max()
            .unwrap_or(1)
            + 1
    });
    let channels = p.channels.unwrap_or_else(|| {
        if let Some(needed) = needed {
            device
                .configs
                .iter()
                .filter(|r| {
                    r.channels >= needed
                        && (r.min_rate..=r.max_rate).contains(&rate)
                        && p.buffer_frames.is_none_or(|n| {
                            r.buffer_range.is_none_or(|(a, b)| (a..=b).contains(&n))
                        })
                })
                .map(|r| r.channels)
                .min()
                .unwrap_or(device.default_channels)
        } else {
            device.default_channels
        }
    });
    if needed.is_some_and(|n| channels < n) {
        return Err("output channel map exceeds selected device".into());
    }
    let mut route = route;
    if let Some(d) = &mut route {
        let requested = d.input.as_ref().or({
            if d.use_default_input {
                c.default_input.as_ref()
            } else {
                None
            }
        });
        if d.input.is_some() || d.use_default_input {
            let input = c
                .inputs
                .iter()
                .find(|i| Some(&i.selector) == requested)
                .or_else(|| {
                    if p.fallback_to_default {
                        fallback = true;
                        c.inputs
                            .iter()
                            .find(|i| Some(&i.selector) == c.default_input.as_ref())
                    } else {
                        None
                    }
                })
                .ok_or("requested input device unavailable")?;
            let needed =
                (d.input_channels[0].max(d.input_channels[1]) + 1).max(d.input_count.unwrap_or(0));
            if !input.configs.iter().any(|r| {
                r.channels >= needed
                    && (r.min_rate..=r.max_rate).contains(&rate)
                    && p.buffer_frames
                        .is_none_or(|n| r.buffer_range.is_none_or(|(a, b)| (a..=b).contains(&n)))
            }) {
                return Err("input device does not support the shared rate/map/buffer".into());
            }
            d.input = Some(input.selector.clone());
        } else {
            d.input = None;
        }
        d.use_default_input = false;
    }
    let config = device
        .configs
        .iter()
        .filter(|r| {
            r.channels == channels
                && (r.min_rate..=r.max_rate).contains(&rate)
                && p.buffer_frames.is_none_or(|n| {
                    r.buffer_range
                        .is_none_or(|(min, max)| (min..=max).contains(&n))
                })
        })
        .min_by_key(|r| match r.format {
            OutputFormat::F32 => 0,
            OutputFormat::I16 => 1,
            OutputFormat::U16 => 2,
        })
        .ok_or("requested rate, channels or buffer size unsupported")?;
    Ok(OutputPlan {
        hardware_channels: channels,
        duplex: route,
        host: c.host.clone(),
        output: device.selector.clone(),
        sample_rate: rate,
        channels: if p.duplex.is_some() { 2 } else { channels },
        buffer_frames: p.buffer_frames,
        max_internal_frames: p.max_internal_frames,
        format: config.format,
        used_fallback: fallback,
    })
}
pub struct OutputEngine {
    pub output_bus: Option<Arc<ChannelBus>>,
    pub input_bus: Option<Arc<ChannelBus>>,
    pub engine: Engine,
    pub receivers: Vec<EventReceiver>,
}
/// Backends let policy/reconnect behavior be tested without opening sound hardware.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingReport {
    pub captured_frames: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub write_errors: u64,
    pub error: Option<String>,
}
pub trait OutputBackend {
    type Stream;
    fn enumerate(
        &mut self,
        preferences: &DevicePreferences,
    ) -> Result<OutputInventory, DeviceError>;
    fn open(
        &mut self,
        plan: &OutputPlan,
        engine: OutputEngine,
        telemetry: Arc<Telemetry>,
    ) -> Result<Self::Stream, DeviceError>;
    fn close(&mut self, stream: Self::Stream) -> Option<RecordingReport> {
        drop(stream);
        None
    }
}
#[derive(Default)]
pub struct NativeOutputBackend;
pub(super) fn native_host(name: Option<&str>) -> Result<cpal::Host, DeviceError> {
    match name {
        None => Ok(cpal::default_host()),
        Some(name) => {
            let id = cpal::available_hosts()
                .into_iter()
                .find(|id| id.name() == name)
                .ok_or("audio backend unavailable")?;
            Ok(cpal::host_from_id(id)?)
        }
    }
}
fn format(f: cpal::SampleFormat) -> Option<OutputFormat> {
    match f {
        cpal::SampleFormat::F32 => Some(OutputFormat::F32),
        cpal::SampleFormat::I16 => Some(OutputFormat::I16),
        cpal::SampleFormat::U16 => Some(OutputFormat::U16),
        _ => None,
    }
}
fn range(r: cpal::SupportedStreamConfigRange) -> Option<OutputRange> {
    Some(OutputRange {
        channels: r.channels(),
        min_rate: r.min_sample_rate().0,
        max_rate: r.max_sample_rate().0,
        buffer_range: match r.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            _ => None,
        },
        format: format(r.sample_format())?,
    })
}
impl OutputBackend for NativeOutputBackend {
    type Stream = ManagedStream;
    fn enumerate(&mut self, p: &DevicePreferences) -> Result<OutputInventory, DeviceError> {
        p.validate()?;
        let h = native_host(p.host.as_deref())?;
        let default = h.default_output_device().and_then(|d| d.name().ok());
        let mut c = OutputInventory {
            host: h.id().name().into(),
            hosts: cpal::available_hosts()
                .into_iter()
                .map(|h| h.name().into())
                .collect(),
            ..Default::default()
        };
        let mut counts = std::collections::BTreeMap::<String, u16>::new();
        for d in h.output_devices()?.take(256) {
            let attempt = (|| -> Result<OutputDevice, DeviceError> {
                let name = d.name()?;
                let occurrence = counts.entry(name.clone()).or_default();
                let selector = DeviceSelector {
                    name,
                    occurrence: *occurrence,
                };
                *occurrence += 1;
                let cfg = d.default_output_config()?;
                let configs = d
                    .supported_output_configs()?
                    .take(512)
                    .filter_map(range)
                    .collect();
                Ok(OutputDevice {
                    selector,
                    configs,
                    default_rate: cfg.sample_rate().0,
                    default_channels: cfg.channels(),
                })
            })();
            match attempt {
                Ok(d) => {
                    if c.default_output.is_none() && Some(&d.selector.name) == default.as_ref() {
                        c.default_output = Some(d.selector.clone());
                    }
                    c.outputs.push(d);
                }
                Err(e) => {
                    if c.warnings.len() < 16 {
                        c.warnings.push(e.to_string());
                    }
                }
            }
        }
        let default = h.default_input_device().and_then(|d| d.name().ok());
        let mut counts = std::collections::BTreeMap::<String, u16>::new();
        if let Ok(devices) = h.input_devices() {
            for d in devices.take(256) {
                let attempt = (|| -> Result<OutputDevice, DeviceError> {
                    let name = d.name()?;
                    let occurrence = counts.entry(name.clone()).or_default();
                    let selector = DeviceSelector {
                        name,
                        occurrence: *occurrence,
                    };
                    *occurrence += 1;
                    let cfg = d.default_input_config()?;
                    Ok(OutputDevice {
                        selector,
                        configs: d
                            .supported_input_configs()?
                            .take(512)
                            .filter_map(range)
                            .collect(),
                        default_rate: cfg.sample_rate().0,
                        default_channels: cfg.channels(),
                    })
                })();
                match attempt {
                    Ok(d) => {
                        if c.default_input.is_none() && Some(&d.selector.name) == default.as_ref() {
                            c.default_input = Some(d.selector.clone());
                        }
                        c.inputs.push(d);
                    }
                    Err(e) => {
                        if c.warnings.len() < 16 {
                            c.warnings.push(e.to_string());
                        }
                    }
                }
            }
        }
        Ok(c)
    }
    fn open(
        &mut self,
        p: &OutputPlan,
        built: OutputEngine,
        telemetry: Arc<Telemetry>,
    ) -> Result<ManagedStream, DeviceError> {
        if let Some(route) = &p.duplex {
            let controls = Arc::new(SessionControls::default());
            let (recorder, tap) = soniclaw_record::Recorder::new(128);
            let options = DuplexOptions {
                output_bus: built.output_bus,
                cue_outputs: route.cue_outputs.clone(),
                input_bus: built.input_bus,
                host: Some(p.host.clone()),
                input_occurrence: route.input.as_ref().map(|d| d.occurrence),
                output_occurrence: Some(p.output.occurrence),
                output_hardware_channels: Some(p.hardware_channels),
                input_device: route.input.as_ref().map(|d| d.name.clone()),
                output_device: Some(p.output.name.clone()),
                input_channels: route.input_channels,
                output_channels: route.output_channels,
                sample_rate: p.sample_rate,
                buffer_frames: p.buffer_frames,
                max_internal_frames: p.max_internal_frames,
            };
            let host = DuplexHost::start_with_events(
                built.engine,
                &options,
                telemetry,
                controls,
                Some(tap),
                built.receivers,
            )?;
            return Ok(ManagedStream::Duplex(Box::new(ManagedDuplex {
                host,
                recorder,
            })));
        }
        if built.output_bus.is_some() {
            return Err("extra output bank requires an explicit duplex output map".into());
        }
        let h = native_host(Some(&p.host))?;
        let device = h
            .output_devices()?
            .filter(|d| d.name().ok().as_ref() == Some(&p.output.name))
            .nth(p.output.occurrence as usize)
            .ok_or("selected device disappeared")?;
        let supported = device
            .supported_output_configs()?
            .filter_map(range)
            .any(|r| {
                r.format == p.format
                    && r.channels == p.channels
                    && (r.min_rate..=r.max_rate).contains(&p.sample_rate)
                    && p.buffer_frames
                        .is_none_or(|n| r.buffer_range.is_none_or(|(a, b)| (a..=b).contains(&n)))
            });
        if !supported {
            return Err("device capabilities changed before opening".into());
        }
        let config = cpal::StreamConfig {
            channels: p.channels,
            sample_rate: cpal::SampleRate(p.sample_rate),
            buffer_size: p
                .buffer_frames
                .map(cpal::BufferSize::Fixed)
                .unwrap_or(cpal::BufferSize::Default),
        };
        let mut engine = built.engine;
        let receivers = built.receivers;
        if receivers.len() > 8
            || receivers
                .iter()
                .any(|r| !Arc::ptr_eq(&r.telemetry, &telemetry))
        {
            return Err("invalid output event channels".into());
        }
        let spec = p.spec();
        engine.prepare(spec)?;
        let stream = match p.format {
            OutputFormat::F32 => {
                build::<f32>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
            OutputFormat::I16 => {
                build::<i16>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
            OutputFormat::U16 => {
                build::<u16>(&device, &config, engine, receivers, telemetry.clone(), spec)?
            }
        };
        stream.play()?;
        Ok(ManagedStream::Output(Box::new(OutputHost {
            _stream: stream,
            spec,
            device_name: p.output.name.clone(),
            telemetry,
        })))
    }
    fn close(&mut self, mut stream: ManagedStream) -> Option<RecordingReport> {
        let report = if let ManagedStream::Duplex(d) = &mut stream {
            let error = d.recorder.stop().err().map(|e| e.to_string());
            let summary = d.recorder.summary();
            if summary.captured_frames > 0 || error.is_some() {
                Some(recording_report(summary, error))
            } else {
                None
            }
        } else {
            None
        };
        drop(stream);
        report
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagerState {
    Stopped,
    Running,
    Waiting,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagerStatus {
    pub last_recording: Option<RecordingReport>,
    pub state: ManagerState,
    pub attempts: u64,
    pub reconnects: u64,
    pub active: Option<OutputPlan>,
    pub last_error: Option<String>,
}
pub struct DeviceManager<B: OutputBackend = NativeOutputBackend> {
    backend: B,
    preferences: DevicePreferences,
    inventory: OutputInventory,
    stream: Option<B::Stream>,
    telemetry: Arc<Telemetry>,
    status: ManagerStatus,
    wanted: bool,
    last_poll: Option<Duration>,
    stream_errors: u64,
    ever_started: bool,
}
impl Default for DeviceManager {
    fn default() -> Self {
        Self::new(NativeOutputBackend, DevicePreferences::default()).expect("valid defaults")
    }
}
impl<B: OutputBackend> DeviceManager<B> {
    pub fn new(backend: B, preferences: DevicePreferences) -> Result<Self, DeviceError> {
        preferences.validate()?;
        Ok(Self {
            backend,
            preferences,
            inventory: Default::default(),
            stream: None,
            telemetry: Arc::new(Default::default()),
            status: ManagerStatus {
                last_recording: None,
                state: ManagerState::Stopped,
                attempts: 0,
                reconnects: 0,
                active: None,
                last_error: None,
            },
            wanted: false,
            last_poll: None,
            stream_errors: 0,
            ever_started: false,
        })
    }
    pub fn preferences(&self) -> &DevicePreferences {
        &self.preferences
    }
    pub fn inventory(&self) -> &OutputInventory {
        &self.inventory
    }
    pub fn status(&self) -> &ManagerStatus {
        &self.status
    }
    pub fn telemetry(&self) -> Arc<Telemetry> {
        self.telemetry.clone()
    }
    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }
    pub fn wants_playback(&self) -> bool {
        self.wanted
    }
    pub fn refresh(&mut self) -> Result<(), DeviceError> {
        self.inventory = self.backend.enumerate(&self.preferences)?;
        Ok(())
    }
    fn close_stream(&mut self) {
        if let Some(stream) = self.stream.take() {
            if let Some(report) = self.backend.close(stream) {
                self.status.last_recording = Some(report);
            }
        }
    }
    pub fn stop(&mut self) {
        self.wanted = false;
        self.close_stream();
        self.status.state = ManagerState::Stopped;
        self.status.active = None;
        self.status.last_error = None;
    }
    pub fn apply(
        &mut self,
        p: DevicePreferences,
        now: Duration,
        build: &mut impl FnMut(&OutputPlan, Arc<Telemetry>) -> Result<OutputEngine, DeviceError>,
    ) -> Result<(), DeviceError> {
        p.validate()?;
        self.preferences = p;
        if self.wanted {
            self.start(now, build)
        } else {
            Ok(())
        }
    }
    pub fn start(
        &mut self,
        now: Duration,
        build: &mut impl FnMut(&OutputPlan, Arc<Telemetry>) -> Result<OutputEngine, DeviceError>,
    ) -> Result<(), DeviceError> {
        self.wanted = true;
        self.last_poll = Some(now);
        self.close_stream();
        self.status.active = None;
        self.status.attempts += 1;
        let result = (|| -> Result<OutputPlan, DeviceError> {
            self.refresh()?;
            let p = negotiate_output(&self.preferences, &self.inventory)?;
            self.telemetry = Arc::new(Default::default());
            self.stream_errors = 0;
            let engine = build(&p, self.telemetry.clone())?;
            self.stream = Some(self.backend.open(&p, engine, self.telemetry.clone())?);
            Ok(p)
        })();
        match result {
            Ok(p) => {
                self.status.state = ManagerState::Running;
                self.status.active = Some(p);
                self.status.last_error = None;
                if self.ever_started {
                    self.status.reconnects += 1;
                }
                self.ever_started = true;
                Ok(())
            }
            Err(e) => {
                self.status.state = if self.preferences.reconnect {
                    ManagerState::Waiting
                } else {
                    ManagerState::Failed
                };
                self.status.last_error = Some(e.to_string());
                Err(e)
            }
        }
    }
    /// Returns true when a restart was attempted. No enumeration or restart occurs
    /// after explicit stop; reconnection never runs on a callback thread.
    pub fn poll(
        &mut self,
        now: Duration,
        build: &mut impl FnMut(&OutputPlan, Arc<Telemetry>) -> Result<OutputEngine, DeviceError>,
    ) -> bool {
        if !self.wanted
            || (!self.preferences.reconnect && !self.is_running())
            || self.last_poll.is_some_and(|last| {
                now.saturating_sub(last) < Duration::from_millis(self.preferences.retry_ms as u64)
            })
        {
            return false;
        }
        self.last_poll = Some(now);
        let errors = self.telemetry.stream_errors.load(Ordering::Relaxed);
        if self.is_running() && errors == self.stream_errors {
            match self.backend.enumerate(&self.preferences) {
                Ok(c) => {
                    let plan = negotiate_output(&self.preferences, &c);
                    self.inventory = c;
                    if plan.as_ref().ok() == self.status.active.as_ref() {
                        return false;
                    }
                }
                Err(e) => {
                    self.status.last_error = Some(e.to_string());
                    return false;
                }
            }
        }
        self.stream_errors = errors;
        if self.is_running() && !self.preferences.reconnect {
            self.close_stream();
            self.status.active = None;
            self.status.state = ManagerState::Failed;
            self.status.last_error =
                Some("device changed or stream failed; press Play to retry".into());
            return true;
        }
        let _ = self.start(now, build);
        true
    }
}

pub enum ManagedStream {
    Output(Box<OutputHost>),
    Duplex(Box<ManagedDuplex>),
}
pub struct ManagedDuplex {
    host: DuplexHost,
    recorder: soniclaw_record::Recorder,
}
fn recording_report(s: soniclaw_record::RecordSummary, error: Option<String>) -> RecordingReport {
    RecordingReport {
        captured_frames: s.captured_frames,
        written_frames: s.written_frames,
        dropped_frames: s.dropped_frames,
        write_errors: s.write_errors,
        error,
    }
}
impl DeviceManager<NativeOutputBackend> {
    pub fn set_monitor(&self, enabled: bool) -> Result<(), DeviceError> {
        if let Some(ManagedStream::Duplex(d)) = &self.stream {
            d.host.controls.monitor.store(enabled, Ordering::Relaxed);
            Ok(())
        } else {
            Err("unified input/output mode is not running".into())
        }
    }
    pub fn monitor_enabled(&self) -> bool {
        matches!(&self.stream,Some(ManagedStream::Duplex(d)) if d.host.controls.monitor.load(Ordering::Relaxed))
    }
    pub fn set_bypass(&self, enabled: bool) -> Result<(), DeviceError> {
        if let Some(ManagedStream::Duplex(d)) = &self.stream {
            d.host.controls.bypass.store(enabled, Ordering::Relaxed);
            Ok(())
        } else {
            Err("unified mode is not running".into())
        }
    }
    pub fn record(
        &mut self,
        path: &Path,
        source: soniclaw_record::RecordSource,
    ) -> Result<(), DeviceError> {
        if let Some(ManagedStream::Duplex(d)) = &mut self.stream {
            d.recorder
                .start(path, d.host.spec.sample_rate as u32, source)
                .map_err(|e| e.to_string())?;
            Ok(())
        } else {
            Err("enable and start unified mode before recording".into())
        }
    }
    pub fn stop_recording(&mut self) -> Result<(), DeviceError> {
        if let Some(ManagedStream::Duplex(d)) = &mut self.stream {
            let result = d.recorder.stop().map_err(|e| e.to_string());
            self.status.last_recording = Some(recording_report(
                d.recorder.summary(),
                result.as_ref().err().cloned(),
            ));
            result?;
            Ok(())
        } else {
            Err("no unified recording session".into())
        }
    }
}
impl<B: OutputBackend> Drop for DeviceManager<B> {
    fn drop(&mut self) {
        self.close_stream();
    }
}
