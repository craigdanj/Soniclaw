//! VST3 control owner, offline processing and bounded device bridge.
//! Native ABI implementation: pinned MIT `vst3-host`, with its device/isolation backends disabled.
pub mod bus_offline;
pub mod buses;
pub mod direct;
pub mod isolation;
pub mod musical;
pub mod musical_offline;
use serde::{Deserialize, Serialize};
use soniclaw_core::{
    validate_events, AudioBuffer, AudioBuffer64, AudioProcessor, AudioProcessor64, Error, Event,
    MidiMessage, ProcessContext, ProcessSpec,
};
use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};
pub use vst3_host::{Parameter, ParameterEdit, ParameterEditKind, PluginInfo, TransportPosition};
pub type HostError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, HostError>;
pub const BLOCK: usize = 256;
pub const LATENCY: usize = BLOCK * 4;
pub const MAX_STATE: usize = 4 * 1024 * 1024;
fn err(e: impl std::fmt::Display) -> HostError {
    e.to_string().into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema: u32,
    pub path: PathBuf,
    pub class_id: String,
    pub version: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
}
impl State {
    pub fn validate(&self) -> Result<()> {
        use sha2::{Digest, Sha256};
        if self.schema != 1
            || self.bytes.len() > MAX_STATE
            || self.class_id.len() != 32
            || !self.class_id.bytes().all(|x| x.is_ascii_hexdigit())
            || format!("{:x}", Sha256::digest(&self.bytes)) != self.sha256
        {
            return Err("invalid VST3 state schema, identity, size or digest".into());
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > MAX_STATE * 5 + 16384 {
            return Err("state JSON too large".into());
        }
        let s: Self = serde_json::from_str(text)?;
        s.validate()?;
        Ok(s)
    }
    /// Bounded control-thread file read. Never executes the saved native path.
    pub fn read(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_STATE * 5 + 16385) as u64)
            .read_to_end(&mut bytes)?;
        Self::from_json(std::str::from_utf8(&bytes)?)
    }
    /// Atomic same-directory replacement, with a uniquely created temporary file.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let text = self.to_json()?;
        let name = path
            .file_name()
            .ok_or("state path has no filename")?
            .to_string_lossy();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temp = path.with_file_name(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        Ok(serde_json::to_string_pretty(self)?)
    }
}
/// # Safety
/// This executes native code. The path must select a trusted ABI-conforming VST3.
pub unsafe fn probe(path: &Path) -> Result<vst3_host::DetailedPluginInfo> {
    vst3_host::get_detailed_plugin_info(path).map_err(err)
}
/// GUI/control-thread owner. Never sent to an audio callback. Drop joins DSP before destroying GUI/module.
pub struct Instance {
    plugin: Arc<Mutex<vst3_host::Plugin>>,
    window: Option<vst3_host::PluginWindow>,
    native_input: Option<musical::NativeInput>,
    native_output: Option<rtrb::Consumer<musical::TimedOutput>>,
    spec: ProcessSpec,
    native_latency: u32,
    layout: vst3_host::AudioBusLayout,
    direct: Option<Arc<direct::Shared>>,
    direct_commands: Option<rtrb::Producer<direct::Command>>,
    worker: Option<JoinHandle<()>>,
    offline: Option<Arc<AtomicBool>>,
    offline_errors: Arc<AtomicU64>,
    shared: Arc<Shared>,
    isolated: bool,
    _thread: PhantomData<Rc<()>>,
}
impl Instance {
    /// # Safety
    /// Load only trusted native code, on the GUI/main thread. Class ID selects a specific factory product.
    pub unsafe fn load(path: &Path, class_id: Option<&str>, spec: ProcessSpec) -> Result<Self> {
        unsafe { Self::load_internal(path, class_id, spec, true, None) }
    }
    unsafe fn load_internal(
        path: &Path,
        class_id: Option<&str>,
        spec: ProcessSpec,
        stereo: bool,
        isolation: Option<&isolation::Config>,
    ) -> Result<Self> {
        spec.validate()?;
        if (stereo && spec.channels != 2) || spec.max_block_frames > 8192 {
            return Err("VST3 owner requires stereo and at most 8192 frames".into());
        }
        if let Some(id) = class_id {
            if id.len() != 32 || !id.bytes().all(|x| x.is_ascii_hexdigit()) {
                return Err("class ID must be 32 hexadecimal digits".into());
            }
        }
        let path = path.canonicalize()?;
        if let Some(config) = isolation {
            config.validate()?;
        }
        let mut builder = vst3_host::Vst3Host::builder()
            .sample_rate(spec.sample_rate)
            .block_size(spec.max_block_frames)
            .input_channels(2)
            .output_channels(2)
            .with_process_isolation(isolation.is_some());
        if let Some(config) = isolation {
            builder = builder
                .helper_path(config.helper.clone())
                .response_timeout(std::time::Duration::from_millis(config.response_timeout_ms))
                .auto_recover_plugins(false);
        }
        let mut host = builder.build().map_err(err)?;
        let plugin = if let Some(id) = class_id {
            host.load_plugin_class(&path, id)
        } else {
            host.load_plugin(&path)
        }
        .map_err(err)?;
        if isolation.is_some() && plugin.isolation_pid().is_none() {
            return Err("isolated load did not create a helper process".into());
        }
        let layout = plugin.audio_bus_layout().map_err(err)?;
        // The device/offline convenience paths expose exactly one stereo main output and optional stereo input.
        if stereo
            && (layout.outputs.len() != 1
                || layout.outputs[0].channel_count != 2
                || layout.inputs.len() > 1
                || layout.inputs.first().is_some_and(|b| b.channel_count != 2))
        {
            return Err("use a supported stereo-main VST3 product; auxiliary buses are not mapped by this adapter".into());
        }
        Ok(Self {
            native_latency: plugin.latency_samples(),
            layout,
            plugin: Arc::new(Mutex::new(plugin)),
            window: None,
            native_input: None,
            native_output: None,
            spec,
            direct: None,
            direct_commands: None,
            worker: None,
            offline: None,
            offline_errors: Arc::new(AtomicU64::new(0)),
            shared: Arc::new(Shared::default()),
            isolated: isolation.is_some(),
            _thread: PhantomData,
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, vst3_host::Plugin>> {
        self.plugin
            .lock()
            .map_err(|_| "VST3 owner lock poisoned".into())
    }
    pub fn info(&self) -> Result<PluginInfo> {
        Ok(self.lock()?.info().clone())
    }
    pub fn parameters(&self) -> Result<Vec<Parameter>> {
        self.lock()?.get_parameters().map_err(err)
    }
    pub fn latency_frames(&self) -> Result<usize> {
        Ok(self.lock()?.latency_samples() as usize
            + if self.worker.is_some() { LATENCY } else { 0 })
    }
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        self.validate_parameter(id, value)?;
        if let Some(commands) = &mut self.direct_commands {
            commands
                .push(direct::Command { id, value })
                .map_err(|_| err("direct parameter queue full"))?;
            self.lock()?.mirror_direct_parameter(id, value);
            return Ok(());
        }

        let mut p = self.lock()?;
        p.set_parameter(id, value).map_err(err)?;
        if self.worker.is_none() {
            flush_stopped(&mut p, self.spec.sample_rate)?;
        }
        Ok(())
    }
    fn validate_parameter(&self, id: u32, value: f64) -> Result<()> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("normalized value must be finite and 0..1".into());
        }
        if !self
            .parameters()?
            .iter()
            .any(|p| p.id == id && !p.is_read_only)
        {
            return Err("unknown/read-only parameter".into());
        }
        Ok(())
    }
    pub fn save_state(&mut self) -> Result<State> {
        use sha2::{Digest, Sha256};
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("stop buffered processing before saving state".into());
        }
        let mut p = self.lock()?;
        flush_stopped(&mut p, self.spec.sample_rate)?;
        let bytes = p.save_state().map_err(err)?;
        if bytes.len() > MAX_STATE {
            return Err("plug-in state exceeds 4 MiB".into());
        }
        let info = p.info();
        Ok(State {
            schema: 1,
            path: info.path.clone(),
            class_id: info.uid.clone(),
            version: info.version.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        })
    }
    pub fn restore_state(&mut self, s: &State) -> Result<()> {
        s.validate()?;
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("stop buffered processing before restoring state".into());
        }
        let mut p = self.lock()?;
        if !p.info().uid.eq_ignore_ascii_case(&s.class_id) {
            return Err("state belongs to another VST3 class".into());
        }
        p.load_state(&s.bytes).map_err(err)
    }
    /// Blocking offline call. May allocate and lock; never call from a device callback.
    /// Timed parameters are (sample offset, stable ID, normalized value), sorted within this block.
    pub fn process_offline(
        &mut self,
        data: &mut [f32],
        changes: &[(usize, u32, f64)],
        position: TransportPosition,
        bpm: f64,
    ) -> Result<()> {
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("stop buffered processing before offline processing".into());
        }
        let frames = data.len() / 2;
        if !data.len().is_multiple_of(2)
            || frames == 0
            || frames > self.spec.max_block_frames
            || !bpm.is_finite()
            || !(1.0..=1000.0).contains(&bpm)
            || !position.quarter_notes.is_finite()
        {
            return Err("invalid offline block or transport".into());
        }
        if changes.len() > 128 || changes.windows(2).any(|w| w[0].0 > w[1].0) {
            return Err("at most 128 sorted parameter changes per block".into());
        }
        for &(offset, id, value) in changes {
            if offset >= frames {
                return Err("parameter offset outside block".into());
            }
            self.validate_parameter(id, value)?;
        }
        let mut p = self.lock()?;
        let mut buffers = vst3_host::AudioBuffers::new(2, 2, frames, self.spec.sample_rate);
        for (i, f) in data.as_chunks::<2>().0.iter().enumerate() {
            buffers.inputs[0][i] = f[0];
            buffers.inputs[1][i] = f[1];
        }
        p.start_processing().map_err(err)?;
        let result = (|| {
            p.set_tempo(bpm).map_err(err)?;
            p.set_transport_position(position).map_err(err)?;
            p.set_playing(true).map_err(err)?;
            for &(offset, id, value) in changes {
                p.set_parameter_at(id, value, offset as i32).map_err(err)?;
            }
            p.process_audio(&mut buffers).map_err(err)
        })();
        let stop = p.stop_processing().map_err(err);
        result?;
        stop?;
        for (i, f) in data.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            f[0] = buffers.outputs[0][i];
            f[1] = buffers.outputs[1][i];
        }
        Ok(())
    }
    pub fn open_editor(&mut self) -> Result<()> {
        if self.direct.is_some() {
            return Err("stop direct processing before opening an editor".into());
        }
        self.close_editor();
        let mut w = vst3_host::PluginWindow::new(self.plugin.clone());
        w.open().map_err(err)?;
        self.window = Some(w);
        Ok(())
    }
    pub fn close_editor(&mut self) {
        if let Some(mut w) = self.window.take() {
            w.close();
        }
    }
    pub fn editor_open(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.is_open())
    }
    pub fn poll(&mut self) -> Result<()> {
        if self.direct.is_some() {
            let flags = self.lock()?.direct_restart_flags();
            if !flags.is_empty() {
                self.stop()?;
                return Err("direct VST3 restart requested; reconstruct owner".into());
            }
            return Ok(());
        }
        if let Some(w) = &mut self.window {
            w.service_platform_events().map_err(err)?;
        }
        let mut p = self.lock()?;
        p.service_run_loop();
        let flags = p.service_host_requests().map_err(err)?;
        if !flags.is_empty() {
            self.shared.restarts.fetch_add(1, Ordering::Relaxed);
            let structural = flags.reload_component()
                || (flags.io_changed() && p.audio_bus_layout().map_err(err)? != self.layout)
                || (flags.latency_changed() && p.latency_samples() != self.native_latency);
            if self.worker.is_some() && structural {
                self.shared.stop.store(true, Ordering::Release);
                return Err("VST3 configuration changed; stop and recreate the owner".into());
            }
        }
        Ok(())
    }
    pub fn drain_parameter_edits(&mut self) -> Result<Vec<ParameterEdit>> {
        Ok(self.lock()?.take_parameter_edits())
    }
    pub fn drain_output_events(&self) -> Result<Vec<vst3_host::OutputEvent>> {
        Ok(self.lock()?.take_output_events())
    }
    pub fn stats(&self) -> Snapshot {
        self.shared.snapshot()
    }
    pub fn stop(&mut self) -> Result<()> {
        let direct = self.direct.take();
        if let Some(d) = &direct {
            d.revoke();
        }
        let offline = self.offline.is_some();
        if let Some(flag) = self.offline.take() {
            flag.store(true, Ordering::Release);
        }
        self.direct_commands = None;
        self.shared.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            w.join().map_err(|_| err("VST3 DSP worker panicked"))?;
        }
        let mut p = self.lock()?;
        let stop = p.stop_processing().map_err(err);
        if direct.is_some() {
            stop?;
            p.set_direct_sample_size(None).map_err(err)?;
            Ok(())
        } else if offline {
            let mode = p
                .set_process_mode(vst3_host::ProcessMode::Realtime)
                .map_err(err);
            stop.and(mode)
        } else {
            stop
        }
    }
    pub fn start_buffered(&mut self) -> Result<Processor> {
        if self.worker.is_some() || self.direct.is_some() || self.offline.is_some() {
            return Err("buffered processing already running".into());
        }
        if self.spec.max_block_frames < BLOCK {
            return Err("buffered host requires prepared maximum >=256".into());
        }
        // Each run has an independent revocation flag, so processors retained from a stopped run stay silent.
        self.shared = Arc::new(Shared::default());
        let (input, mut native, output) = musical::queues(self.shared.clone());
        self.native_input = Some(input);
        self.native_output = Some(output);
        let parameters = self
            .parameters()?
            .into_iter()
            .filter(|p| !p.is_read_only)
            .map(|p| p.id)
            .collect();
        let (native_latency, layout) = {
            let mut plugin = self.lock()?;
            plugin.service_run_loop();
            plugin.service_host_requests().map_err(err)?;
            plugin.start_processing().map_err(err)?;
            plugin.service_host_requests().map_err(err)?;
            (
                plugin.latency_samples(),
                plugin.audio_bus_layout().map_err(err)?,
            )
        };
        if layout.outputs.len() != 1
            || layout.outputs[0].channel_count != 2
            || layout.inputs.len() > 1
            || layout.inputs.first().is_some_and(|b| b.channel_count != 2)
        {
            self.lock()?.stop_processing().map_err(err)?;
            return Err("VST3 changed to unsupported audio buses".into());
        }
        self.native_latency = native_latency;
        self.layout = layout;
        let (tx, mut rx) = {
            let (a, b) = rtrb::RingBuffer::<Packet>::new(8);
            let (c, d) = rtrb::RingBuffer::<Packet>::new(8);
            ((a, d), (b, c))
        };
        let plugin = self.plugin.clone();
        let shared = self.shared.clone();
        let rate = self.spec.sample_rate;
        let isolated = self.isolated;
        self.worker = Some(
            std::thread::Builder::new()
                .name("soniclaw-vst3".into())
                .spawn(move || {
                    let mut buffers = vst3_host::AudioBuffers::new(2, 2, BLOCK, rate);
                    let mut voices = musical::Voices::default();
                    while !shared.stop.load(Ordering::Acquire) {
                        let Ok(mut packet) = rx.0.pop() else {
                            std::thread::park_timeout(std::time::Duration::from_micros(100));
                            continue;
                        };
                        let result = (|| -> Result<()> {
                            let mut p = plugin.lock().map_err(|_| err("poisoned DSP lock"))?;
                            if shared.panic.swap(false, Ordering::AcqRel) {
                                p.midi_panic().map_err(err)?;
                            }
                            musical::process_packet(
                                &mut p,
                                &mut packet,
                                &mut buffers,
                                &mut voices,
                                &mut native,
                            )?;
                            Ok(())
                        })();
                        if let Err(error) = result {
                            if shared.errors.load(Ordering::Relaxed) == 0 {
                                eprintln!("Soniclaw native worker failed: {error}");
                            }
                            packet.samples.fill(0.0);
                            shared.errors.fetch_add(1, Ordering::Relaxed);
                            if isolated {
                                shared.stop.store(true, Ordering::Release);
                            }
                        }
                        if rx.1.push(packet).is_err() {
                            shared.output_drops.fetch_add(1, Ordering::Relaxed);
                        }
                        shared.blocks.fetch_add(1, Ordering::Relaxed);
                    }
                })?,
        );
        Ok(Processor {
            tx: tx.0,
            rx: tx.1,
            input: Packet::default(),
            output: None,
            queued: None,
            position: 0,
            rate: self.spec.sample_rate,
            shared: self.shared.clone(),
            latency: LATENCY + self.native_latency as usize,
            music: None,
            parameters,
        })
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = self.stop();
        self.close_editor();
    }
}
#[derive(Default)]
struct Shared {
    stop: AtomicBool,
    blocks: AtomicU64,
    input_drops: AtomicU64,
    output_drops: AtomicU64,
    underruns: AtomicU64,
    errors: AtomicU64,
    panic: AtomicBool,
    restarts: AtomicU64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub blocks: u64,
    pub input_drops: u64,
    pub output_drops: u64,
    pub underruns: u64,
    pub process_errors: u64,
    pub restart_requests: u64,
    pub bridge_latency_frames: usize,
}
impl Shared {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            blocks: self.blocks.load(Ordering::Relaxed),
            input_drops: self.input_drops.load(Ordering::Relaxed),
            output_drops: self.output_drops.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            process_errors: self.errors.load(Ordering::Relaxed),
            restart_requests: self.restarts.load(Ordering::Relaxed),
            bridge_latency_frames: LATENCY,
        }
    }
}
#[derive(Clone, Debug)]
struct Packet {
    samples: [f32; BLOCK * 2],
    sequence: u64,
    sample: u64,
    count: usize,
    events: [Event; 128],
    beat: f64,
    bpm: f64,
    playing: bool,
    music: [Option<soniclaw_transport::Frame>; BLOCK],
    recording: bool,
    panic_on_loop: bool,
    parameters: [musical::ParameterPoint; 1024],
    parameter_count: usize,
    discontinuities: [bool; BLOCK],
}
impl Default for Packet {
    fn default() -> Self {
        Self {
            samples: [0.; BLOCK * 2],
            sequence: 0,
            sample: 0,
            count: 0,
            events: [Event {
                frame: 0,
                message: MidiMessage::AllNotesOff,
            }; 128],
            beat: 0.,
            bpm: 120.,
            playing: false,
            music: [None; BLOCK],
            recording: false,
            panic_on_loop: false,
            parameters: [musical::ParameterPoint::default(); 1024],
            parameter_count: 0,
            discontinuities: [false; BLOCK],
        }
    }
}
/// Device callback half: fixed packets, bounded SPSC operations, silence on late/missing output.
/// Native calls and all Mutex operations run on the worker/control side.
pub struct Processor {
    tx: rtrb::Producer<Packet>,
    rx: rtrb::Consumer<Packet>,
    input: Packet,
    output: Option<Packet>,
    queued: Option<Packet>,
    position: u64,
    rate: f64,
    shared: Arc<Shared>,
    latency: usize,
    music: Option<musical::Music>,
    parameters: Vec<u32>,
}
impl AudioProcessor for Processor {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        s.validate()?;
        if s.channels != 2 || s.sample_rate != self.rate || self.position != 0 {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        self.process_inner(b, ctx, &[]);
    }
}
impl Processor {
    /// Lock-free audio failure status.
    pub fn faulted(&self) -> bool {
        self.shared.errors.load(Ordering::Acquire) > 0
    }
    fn process_inner(
        &mut self,
        b: &mut AudioBuffer<'_>,
        ctx: &ProcessContext<'_>,
        points: &[musical::ParameterPoint],
    ) {
        if self.shared.stop.load(Ordering::Acquire)
            || b.channels() != 2
            || b.frames() > 8192
            || ctx.events.len() > 1024
            || !ctx.transport.tempo.is_finite()
            || !(1.0..=1000.0).contains(&ctx.transport.tempo)
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        let mut discontinuity = false;
        let mut point_index = 0;
        if let Some(m) = &mut self.music {
            discontinuity =
                m.source_end.is_some() && m.source_end != Some(ctx.transport.sample_position);
            if m.source_end != Some(ctx.transport.sample_position)
                && m.clock.seek(ctx.transport.sample_position).is_err()
            {
                b.clear();
                self.shared.stop.store(true, Ordering::Release);
                return;
            }
            m.clock.set_playing(ctx.transport.playing);
            m.source_end = Some(
                ctx.transport
                    .sample_position
                    .saturating_add(b.frames() as u64),
            );
        }
        let mut event_index = 0;
        for (i, f) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            let offset = self.position as usize % BLOCK;
            let seq = self.position / BLOCK as u64;
            if offset == 0 {
                self.input.count = 0;
                self.input.parameter_count = 0;
                self.input.sequence = seq;
                self.input.sample = ctx.transport.sample_position.saturating_add(i as u64);
                self.input.bpm = ctx.transport.tempo;
                self.input.playing = ctx.transport.playing;
                self.input.beat = self.input.sample as f64 / self.rate * self.input.bpm / 60.;
                self.output = None;
                if seq >= 4 {
                    for _ in 0..8 {
                        match self.queued.take().or_else(|| self.rx.pop().ok()) {
                            Some(p) if p.sequence + 4 > seq => {
                                self.queued = Some(p);
                                break;
                            }
                            Some(p) if p.sequence + 4 == seq => {
                                self.output = Some(p);
                                break;
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                    if self.output.is_none() {
                        self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            self.input.discontinuities[offset] = discontinuity && i == 0;
            if let Some(m) = &mut self.music {
                let f = match m.clock.frame() {
                    Ok(f) => f,
                    Err(_) => {
                        self.shared.stop.store(true, Ordering::Release);
                        b.clear();
                        return;
                    }
                };
                self.input.music[offset] = Some(f);
                self.input.recording = m.recording;
                self.input.panic_on_loop = m.panic_on_loop;
                if m.clock.advance(1).is_err() {
                    self.shared.stop.store(true, Ordering::Release);
                    b.clear();
                    return;
                }
            }
            while point_index < points.len() && points[point_index].frame == i {
                if self.input.parameter_count >= 1024 {
                    self.shared.errors.fetch_add(1, Ordering::Relaxed);
                    self.shared.stop.store(true, Ordering::Release);
                    b.clear();
                    return;
                }
                self.input.parameters[self.input.parameter_count] = musical::ParameterPoint {
                    frame: offset,
                    ..points[point_index]
                };
                self.input.parameter_count += 1;
                point_index += 1;
            }
            while event_index < ctx.events.len() && ctx.events[event_index].frame == i {
                if self.input.count < 128 {
                    self.input.events[self.input.count] = Event {
                        frame: offset,
                        message: ctx.events[event_index].message,
                    };
                    self.input.count += 1;
                } else {
                    self.shared.panic.store(true, Ordering::Release);
                    self.shared.errors.fetch_add(1, Ordering::Relaxed);
                    self.shared.stop.store(true, Ordering::Release);
                    b.clear();
                    return;
                }
                event_index += 1;
            }
            self.input.samples[offset * 2] = f[0];
            self.input.samples[offset * 2 + 1] = f[1];
            if let Some(p) = &self.output {
                f.copy_from_slice(&p.samples[offset * 2..offset * 2 + 2]);
            } else {
                f.fill(0.);
            }
            if offset + 1 == BLOCK && self.tx.push(self.input.clone()).is_err() {
                self.shared.input_drops.fetch_add(1, Ordering::Relaxed);
                self.shared.panic.store(true, Ordering::Release);
            }
            self.position = self.position.wrapping_add(1);
        }
    }
}
fn convert_midi(m: MidiMessage) -> Option<vst3_host::MidiEvent> {
    use vst3_host::{MidiChannel, MidiEvent as M};
    let channel = match m {
        MidiMessage::NoteOn { channel, .. }
        | MidiMessage::NoteOff { channel, .. }
        | MidiMessage::ControlChange { channel, .. }
        | MidiMessage::PitchBend { channel, .. }
        | MidiMessage::ChannelPressure { channel, .. }
        | MidiMessage::PolyPressure { channel, .. } => MidiChannel::from_index(channel)?,
        _ => return None,
    };
    Some(match m {
        MidiMessage::NoteOn { note, velocity, .. } => M::NoteOn {
            channel,
            note,
            velocity,
        },
        MidiMessage::NoteOff { note, velocity, .. } => M::NoteOff {
            channel,
            note,
            velocity,
        },
        MidiMessage::ControlChange {
            controller, value, ..
        } => M::ControlChange {
            channel,
            controller,
            value,
        },
        MidiMessage::PitchBend { value, .. } => M::PitchBend { channel, value },
        MidiMessage::ChannelPressure { value, .. } => M::ChannelAftertouch {
            channel,
            pressure: value,
        },
        MidiMessage::PolyPressure { note, value, .. } => M::PolyAftertouch {
            channel,
            note,
            pressure: value,
        },
        _ => return None,
    })
}

fn flush_stopped(p: &mut vst3_host::Plugin, rate: f64) -> Result<()> {
    let layout = p.audio_bus_layout().map_err(err)?;
    p.start_processing().map_err(err)?;
    let result = if layout.inputs.len() > 1
        || layout.outputs.len() != 1
        || layout.inputs.first().is_some_and(|b| b.channel_count != 2)
        || layout.outputs.first().is_some_and(|b| b.channel_count != 2)
    {
        let mut zero = vst3_host::BusAudioBuffers::new(&layout, 0, rate);
        p.process_bus_audio(&mut zero).map_err(err)
    } else {
        let mut zero = vst3_host::AudioBuffers::new(2, 2, 0, rate);
        p.process_audio(&mut zero).map_err(err)
    };
    let stop = p.stop_processing().map_err(err);
    result?;
    stop
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    #[test]
    fn callback_partitions_preserve_fixed_delay_and_allocate_nothing() {
        let (tx, _input) = rtrb::RingBuffer::<Packet>::new(8);
        let (mut output, rx) = rtrb::RingBuffer::<Packet>::new(8);
        for sequence in 0..4 {
            let packet = Packet {
                sequence,
                samples: [0.25; BLOCK * 2],
                ..Default::default()
            };
            output.push(packet).unwrap();
        }
        let mut p = Processor {
            tx,
            rx,
            input: Default::default(),
            output: None,
            queued: None,
            position: 0,
            rate: 48000.,
            shared: Arc::new(Shared::default()),
            latency: LATENCY,
            music: None,
            parameters: Vec::new(),
        };
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 512,
            channels: 2,
        })
        .unwrap();
        let mut rendered = [0.; 4096];
        let mut cursor = 0;
        let sizes = [1, 7, 64, 255, 257, 512];
        let mut turn = 0;
        while cursor < 2048 {
            let frames = sizes[turn % sizes.len()].min(2048 - cursor);
            let slice = &mut rendered[cursor * 2..(cursor + frames) * 2];
            slice.fill(1.);
            let ctx = ProcessContext {
                events: &[],
                transport: soniclaw_core::Transport {
                    sample_position: cursor as u64,
                    ..Default::default()
                },
            };
            let result = allocation_counter::measure(|| {
                p.process(&mut AudioBuffer::new(slice, 2).unwrap(), &ctx)
            });
            assert_eq!(result.count_total, 0);
            cursor += frames;
            turn += 1;
        }
        assert!(rendered[..LATENCY * 2].iter().all(|x| *x == 0.));
        assert!(rendered[LATENCY * 2..].iter().all(|x| *x == 0.25));
        assert_eq!(p.shared.snapshot().underruns, 0);
    }
    #[test]
    fn state_file_roundtrip_is_bounded_and_failed_write_preserves_destination() {
        use sha2::{Digest, Sha256};
        let dir = std::env::temp_dir().join(format!("soniclaw-vst3-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        let bytes = vec![1, 2, 3];
        let mut state = State {
            schema: 1,
            path: "trusted.vst3".into(),
            class_id: "A".repeat(32),
            version: "1".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        };
        state.write_atomic(&path).unwrap();
        assert_eq!(State::read(&path).unwrap().bytes, state.bytes);
        let original = std::fs::read(&path).unwrap();
        state.sha256 = "corrupt".into();
        assert!(state.write_atomic(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_state_rejected_before_native() {
        let s = State {
            schema: 1,
            path: "a.vst3".into(),
            class_id: "0".repeat(32),
            version: "1".into(),
            bytes: vec![1],
            sha256: "bad".into(),
        };
        assert!(s.validate().is_err());
        assert!(State::from_json("{}").is_err());
    }
    #[test]
    fn retained_processor_revoked_without_allocation() {
        let (tx, _rx) = rtrb::RingBuffer::<Packet>::new(8);
        let (_tx, rx) = rtrb::RingBuffer::<Packet>::new(8);
        let shared = Arc::new(Shared::default());
        let mut p = Processor {
            tx,
            rx,
            input: Default::default(),
            output: None,
            queued: None,
            position: 0,
            rate: 48000.,
            shared: shared.clone(),
            latency: LATENCY,
            music: None,
            parameters: Vec::new(),
        };
        shared.stop.store(true, Ordering::Release);
        let mut data = [1.; 512];
        let ctx = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        let info = allocation_counter::measure(|| {
            p.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx)
        });
        assert_eq!(info.count_total, 0);
        assert!(data.iter().all(|x| *x == 0.));
    }
}

pub mod offline;
