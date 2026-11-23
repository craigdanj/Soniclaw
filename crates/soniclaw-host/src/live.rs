//! Main-thread rack owner and a nonblocking stereo callback bridge.
//! Only trusted CLAP modules are supported. Dropping the owner joins DSP before destruction.
use super::*;
use clap_sys::ext::gui::*;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Arc,
};
use std::thread::JoinHandle;
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RackEntry {
    pub path: PathBuf,
    pub plugin_id: String,
}
/// # Safety
/// Every configured path must select trusted, ABI-conforming native code.
/// Identical library paths share one initialization within this reconstruction.
pub unsafe fn instantiate(entries: &[RackEntry]) -> Result<Vec<ClapInstance>, HostError> {
    if entries.is_empty() || entries.len() > MAX_NODES {
        return Err("rack needs 1..8 entries".into());
    }
    let mut libraries = std::collections::BTreeMap::new();
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.path.canonicalize()?;
        if !libraries.contains_key(&path) {
            libraries.insert(path.clone(), unsafe { ClapLibrary::load(&path) }?);
        }
        out.push(libraries[&path].instantiate(&entry.plugin_id)?);
    }
    Ok(out)
}
#[derive(Clone, Copy, Debug)]
pub struct RackEvent {
    pub node: usize,
    pub event: OutputEvent,
}

pub const BLOCK: usize = 256;
pub const BRIDGE_LATENCY: usize = BLOCK * 4;
pub const MAX_NODES: usize = 8;
const MIDI: usize = 128;
const DEPTH: usize = 8;
const ZERO: Event = Event {
    frame: 0,
    message: MidiMessage::AllNotesOff,
};
#[derive(Clone)]
struct Packet {
    samples: [f32; BLOCK * 2],
    events: [Event; MIDI],
    count: usize,
    sequence: u64,
    transport: Transport,
}
impl Default for Packet {
    fn default() -> Self {
        Self {
            samples: [0.0; BLOCK * 2],
            events: [ZERO; MIDI],
            count: 0,
            sequence: 0,
            transport: Transport::default(),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Command {
    pub(crate) node: usize,
    pub(crate) id: u32,
    pub(crate) value: f64,
}
#[derive(Default)]
pub struct Stats {
    pub input_drops: AtomicU64,
    pub output_drops: AtomicU64,
    pub underruns: AtomicU64,
    pub process_errors: AtomicU64,
    pub event_drops: AtomicU64,
    pub blocks: AtomicU64,
    pub max_callback_ns: AtomicU64,
    pub deadline_misses: AtomicU64,
    pub bridge_frames: AtomicU64,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Snapshot {
    pub input_drops: u64,
    pub output_drops: u64,
    pub underruns: u64,
    pub process_errors: u64,
    pub event_drops: u64,
    pub blocks: u64,
    pub max_callback_ns: u64,
    pub deadline_misses: u64,
    pub bridge_latency_frames: usize,
}
impl Stats {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            input_drops: self.input_drops.load(Ordering::Relaxed),
            output_drops: self.output_drops.load(Ordering::Relaxed),
            underruns: self.underruns.load(Ordering::Relaxed),
            process_errors: self.process_errors.load(Ordering::Relaxed),
            event_drops: self.event_drops.load(Ordering::Relaxed),
            blocks: self.blocks.load(Ordering::Relaxed),
            max_callback_ns: self.max_callback_ns.load(Ordering::Relaxed),
            deadline_misses: self.deadline_misses.load(Ordering::Relaxed),
            bridge_latency_frames: self.bridge_frames.load(Ordering::Relaxed) as usize,
        }
    }
}
pub(crate) struct Shared {
    pub(crate) stop: AtomicBool,
    pub(crate) panic: AtomicBool,
    pub(crate) flush: AtomicU32,
    pub(crate) bypass: AtomicU32,
    pub(crate) stats: Arc<Stats>,
}
/// Callback-owned half. Fixed 256-frame stereo packets adapt arbitrary device callback sizes.
/// Output is scheduled four packets later; late packets are discarded, never played late.
pub struct RackProcessor {
    tx: Producer<Packet>,
    rx: Consumer<Packet>,
    input: Packet,
    output: Option<Packet>,
    queued: Option<Packet>,
    position: u64,
    delay_blocks: u64,
    rate: f64,
    shared: Arc<Shared>,
    latency: usize,
}
impl AudioProcessor for RackProcessor {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 2 || (s.sample_rate - self.rate).abs() > 0.01 || self.position != 0 {
            return Err(Error::InvalidParameter);
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.shared.panic.store(true, Ordering::Release);
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if self.shared.stop.load(Ordering::Acquire)
            || b.channels() != 2
            || validate_events(ctx.events, b.frames()).is_err()
        {
            b.clear();
            return;
        }
        let mut event = 0;
        for (i, f) in b
            .samples_mut()
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            let offset = self.position as usize % BLOCK;
            let sequence = self.position / BLOCK as u64;
            if offset == 0 {
                self.input.transport = Transport {
                    sample_position: ctx.transport.sample_position.saturating_add(i as u64),
                    ..ctx.transport
                };
                self.input.count = 0;
                self.input.sequence = sequence;
                self.output = None;
                if sequence >= self.delay_blocks {
                    for _ in 0..DEPTH {
                        let next = self.queued.take().map_or_else(|| self.rx.pop().ok(), Some);
                        match next {
                            Some(p) if p.sequence + self.delay_blocks > sequence => {
                                self.queued = Some(p);
                                break;
                            }
                            Some(p) if p.sequence + self.delay_blocks == sequence => {
                                self.output = Some(p);
                                break;
                            }
                            Some(_) => {}
                            None => break,
                        }
                    }
                    if self.output.is_none() {
                        self.shared.stats.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            self.input.samples[offset * 2] = f[0];
            self.input.samples[offset * 2 + 1] = f[1];
            while event < ctx.events.len() && ctx.events[event].frame == i {
                if self.input.count < MIDI {
                    self.input.events[self.input.count] = Event {
                        frame: offset,
                        message: ctx.events[event].message,
                    };
                    self.input.count += 1;
                } else {
                    self.shared
                        .stats
                        .event_drops
                        .fetch_add(1, Ordering::Relaxed);
                    self.shared.panic.store(true, Ordering::Release);
                }
                event += 1;
            }
            if let Some(p) = &self.output {
                f.copy_from_slice(&p.samples[offset * 2..offset * 2 + 2]);
            } else {
                f.fill(0.0);
            }
            if offset + 1 == BLOCK && self.tx.push(self.input.clone()).is_err() {
                self.shared
                    .stats
                    .input_drops
                    .fetch_add(1, Ordering::Relaxed);
                self.shared.panic.store(true, Ordering::Release);
            }
            self.position = self.position.saturating_add(1);
        }
    }
}
impl Drop for RackProcessor {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
/// Raw lease used only by the owned worker; LiveRack joins it before releasing instances.
pub(crate) struct Lease {
    pub(crate) plugin: *const clap_plugin,
    pub(crate) context: *const HostContext,
    pub(crate) parameters: Vec<ParameterInfo>,
    pub(crate) resources: Resources,
    pub(crate) spec: ProcessSpec,
    pub(crate) input_channels: usize,
    pub(crate) note_dialect: u32,
    pub(crate) params: *const clap_plugin_params,
    pub(crate) latency: usize,
}
// SAFETY: resources/metadata transfer exclusively. Context has a stable Box allocation;
// immutable fields and atomic flags can be accessed concurrently. LiveRack holds every
// library/instance alive, exposes only CLAP main-thread services, and joins on Drop.
unsafe impl Send for Lease {}
pub(crate) struct Editor {
    pub(crate) plugin: *const clap_plugin,
    pub(crate) extension: *const clap_plugin_gui,
    pub(crate) scale: f64,
    pub(crate) embedded: bool,
    pub(crate) size: [u32; 2],
    pub(crate) context: *const HostContext,
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub(crate) owned: Option<crate::editors::Window>,
}
impl Editor {
    fn close(&mut self) {
        unsafe {
            (*self.context).gui_embedded.store(false, Ordering::Release);
            if let Some(hide) = (*self.extension).hide {
                hide(self.plugin);
            }
            (*self.extension).destroy.expect("validated")(self.plugin);
            (*self.context).gui_flags.store(0, Ordering::Release);
            (*self.context).gui_resize.store(0, Ordering::Release);
        }
    }
}
/// Main-thread owner, deliberately !Send via ClapInstance. Call `poll` from the GUI loop.
/// Topology and state edits require stopping and reconstructing the rack.
pub struct LiveRack {
    pub(crate) instances: Vec<ClapInstance>,
    pub(crate) editors: Vec<Option<Editor>>,
    pub(crate) commands: Producer<Command>,
    pub(crate) events: Consumer<RackEvent>,
    pub(crate) shared: Arc<Shared>,
    pub(crate) worker: Option<JoinHandle<()>>,
    pub(crate) rate: f64,
    pub(crate) editor_events: Vec<crate::editors::EditorEvent>,
    pub(crate) direct: Option<Arc<crate::direct::DirectShared>>,
}
impl LiveRack {
    /// Takes stopped instances created on this thread. Errors destroy them on this thread.
    pub fn start(
        instances: Vec<ClapInstance>,
        sample_rate: f64,
    ) -> Result<(Self, RackProcessor), HostError> {
        Self::start_buffered(instances, sample_rate, 4)
    }
    /// Select 1..7 lookahead packets (256 frames each); topology changes require reconstruction.
    pub fn start_buffered(
        mut instances: Vec<ClapInstance>,
        sample_rate: f64,
        delay_blocks: usize,
    ) -> Result<(Self, RackProcessor), HostError> {
        if !(1..DEPTH).contains(&delay_blocks) {
            return Err("buffer delay must be 1..7 packets".into());
        }
        if instances.is_empty() || instances.len() > MAX_NODES {
            return Err("live rack needs 1..8 nodes".into());
        }
        let spec = ProcessSpec {
            sample_rate,
            channels: 2,
            max_block_frames: BLOCK,
        };
        spec.validate()?;
        let mut leases = Vec::new();
        let mut latency = BLOCK * delay_blocks;
        for instance in &mut instances {
            if instance.active {
                return Err("live rack accepts deactivated instances".into());
            }
            instance.activate(spec)?;
            if instance.latency > 1_048_576 {
                return Err("latency exceeds bypass memory limit".into());
            }
            latency = latency.saturating_add(instance.latency);
            leases.push(Lease {
                plugin: instance.plugin,
                context: &*instance.context,
                parameters: instance.parameters.clone(),
                resources: instance.resources.take().expect("active resources"),
                spec,
                input_channels: instance.input_channels,
                note_dialect: instance.note_dialect,
                latency: instance.latency,
                params: unsafe {
                    (*instance.plugin).get_extension.expect("checked")(
                        instance.plugin,
                        CLAP_EXT_PARAMS.as_ptr(),
                    )
                    .cast::<clap_plugin_params>()
                },
            });
        }
        let (tx, mut requests) = RingBuffer::<Packet>::new(DEPTH);
        let (mut results, rx) = RingBuffer::<Packet>::new(DEPTH);
        let (mut event_tx, events) = RingBuffer::<RackEvent>::new(128);
        let (commands, mut controls) = RingBuffer::<Command>::new(64);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            panic: AtomicBool::new(false),
            flush: AtomicU32::new(0),
            bypass: AtomicU32::new(0),
            stats: Arc::new(Stats::default()),
        });
        shared
            .stats
            .bridge_frames
            .store((BLOCK * delay_blocks) as u64, Ordering::Relaxed);
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("soniclaw-clap-audio".into())
            .spawn(move || {
                AUDIO.set(true);
                // SAFETY: each raw lease is exclusively owned here, backed by LiveRack's instances.
                let flush_extensions: Vec<_> = leases.iter().map(|l| l.params).collect();
                let mut bypass_lines: Vec<_> = leases
                    .iter()
                    .map(|l| crate::session::BypassLine::new(l.latency).expect("validated latency"))
                    .collect();
                let mut endpoints: Vec<_> = leases
                    .iter_mut()
                    .map(|lease| HostedAudio {
                        plugin: lease.plugin,
                        _context: unsafe { &*lease.context },
                        parameters: &lease.parameters,
                        resources: std::mem::replace(&mut lease.resources, Resources::empty()),
                        spec: lease.spec,
                        input_channels: lease.input_channels,
                        note_dialect: lease.note_dialect,
                        processing: false,
                        steady: 0,
                    })
                    .collect();
                for audio in &mut endpoints {
                    if unsafe { (*audio.plugin).start_processing.expect("checked")(audio.plugin) } {
                        audio.processing = true;
                    } else {
                        worker_shared
                            .stats
                            .process_errors
                            .fetch_add(1, Ordering::Relaxed);
                        worker_shared.stop.store(true, Ordering::Release);
                        break;
                    }
                }
                let mut parameter_events: Vec<Vec<ParameterEvent>> = (0..endpoints.len())
                    .map(|_| Vec::with_capacity(64))
                    .collect();
                while !worker_shared.stop.load(Ordering::Acquire) {
                    let Ok(mut packet) = requests.pop() else {
                        std::thread::park_timeout(std::time::Duration::from_micros(100));
                        continue;
                    };
                    for values in &mut parameter_events {
                        values.clear();
                    }
                    for _ in 0..64 {
                        let Ok(c) = controls.pop() else {
                            break;
                        };
                        if let Some(events) = parameter_events.get_mut(c.node) {
                            events.push(ParameterEvent {
                                frame: 0,
                                id: c.id,
                                value: c.value,
                            });
                        }
                    }
                    if worker_shared.panic.swap(false, Ordering::AcqRel) {
                        packet.count = 1;
                        packet.events[0] = ZERO;
                    }
                    let flush_mask = worker_shared.flush.swap(0, Ordering::AcqRel);
                    for (index, (audio, parameters)) in
                        endpoints.iter_mut().zip(&parameter_events).enumerate()
                    {
                        if flush_mask & (1 << index) != 0 && !flush_extensions[index].is_null() {
                            audio.resources.out.clear();
                            let mut out = OutputContext {
                                events: &mut audio.resources.out,
                                frames: BLOCK,
                                dropped: &mut audio.resources.dropped,
                            };
                            let oe = clap_output_events {
                                ctx: (&mut out as *mut OutputContext<'_>).cast(),
                                try_push: Some(event_push),
                            };
                            let empty = Vec::<*const clap_event_header>::new();
                            let ie = clap_input_events {
                                ctx: (&empty as *const Vec<*const clap_event_header>)
                                    .cast_mut()
                                    .cast(),
                                size: Some(event_size),
                                get: Some(event_get),
                            };
                            unsafe {
                                if let Some(flush) = (*flush_extensions[index]).flush {
                                    flush(audio.plugin, &ie, &oe);
                                }
                            }
                        }

                        if flush_mask & (1 << index) != 0 {
                            for event in audio.output_events() {
                                if event_tx
                                    .push(RackEvent {
                                        node: index,
                                        event: *event,
                                    })
                                    .is_err()
                                {
                                    worker_shared
                                        .stats
                                        .event_drops
                                        .fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        let dry = packet.samples;
                        let midi = if audio.note_dialect == 0 {
                            &[][..]
                        } else {
                            &packet.events[..packet.count]
                        };
                        if audio
                            .process_block(&mut packet.samples, midi, parameters, packet.transport)
                            .is_err()
                            || packet.samples.iter().any(|x| !x.is_finite())
                        {
                            packet.samples.fill(0.0);
                            worker_shared
                                .stats
                                .process_errors
                                .fetch_add(1, Ordering::Relaxed);
                            worker_shared.panic.store(true, Ordering::Release);
                        }
                        bypass_lines[index].apply(
                            &dry,
                            &mut packet.samples,
                            worker_shared.bypass.load(Ordering::Acquire) & (1 << index) != 0,
                        );
                    }
                    worker_shared.stats.blocks.fetch_add(1, Ordering::Relaxed);
                    for (node, audio) in endpoints.iter().enumerate() {
                        for event in audio.output_events() {
                            if event_tx
                                .push(RackEvent {
                                    node,
                                    event: *event,
                                })
                                .is_err()
                            {
                                worker_shared
                                    .stats
                                    .event_drops
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    if results.push(packet).is_err() {
                        worker_shared
                            .stats
                            .output_drops
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                for audio in &mut endpoints {
                    audio.stop();
                }
                AUDIO.set(false);
            })?;
        let count = instances.len();
        let processor = RackProcessor {
            tx,
            rx,
            input: Packet::default(),
            output: None,
            queued: None,
            position: 0,
            delay_blocks: delay_blocks as u64,
            rate: sample_rate,
            shared: shared.clone(),
            latency,
        };
        Ok((
            Self {
                instances,
                editors: (0..count).map(|_| None).collect(),
                commands,
                events,
                shared,
                worker: Some(worker),
                direct: None,
                editor_events: Vec::with_capacity(128),
                rate: sample_rate,
            },
            processor,
        ))
    }
    /// Trusted direct callback mode; zero bridge frames, native latency still applies.
    pub fn start_direct(
        instances: Vec<ClapInstance>,
        spec: ProcessSpec,
    ) -> Result<(Self, crate::direct::DirectProcessor), HostError> {
        crate::direct::DirectProcessor::pair(instances, spec)
    }
    /// Drain at most 128 observed output events on the control thread.
    pub fn drain_events(&mut self) -> Vec<RackEvent> {
        let mut events = Vec::new();
        for _ in 0..128 {
            match self.events.pop() {
                Ok(e) => events.push(e),
                Err(_) => break,
            }
        }
        events
    }
    pub fn sample_rate(&self) -> f64 {
        self.rate
    }
    pub fn stats(&self) -> Arc<Stats> {
        self.shared.stats.clone()
    }
    pub fn parameters(&self, node: usize) -> Option<&[ParameterInfo]> {
        self.instances.get(node).map(|p| p.parameters())
    }
    /// Bounded next-packet parameter update, validated before queueing.
    pub fn set_parameter(&mut self, node: usize, id: u32, value: f64) -> Result<(), HostError> {
        let p = self
            .instances
            .get(node)
            .and_then(|i| i.parameters.iter().find(|p| p.id == id))
            .ok_or("unknown rack parameter")?;
        if !value.is_finite()
            || !(p.min..=p.max).contains(&value)
            || p.flags & CLAP_PARAM_IS_READONLY != 0
        {
            return Err("invalid/read-only parameter".into());
        }
        self.commands
            .push(Command { node, id, value })
            .map_err(|_| "rack parameter queue full".into())
    }
    /// Services legal main-thread requests. Restart/rescan/flush requests stop the rack;
    /// reconstruct instead of mutating active metadata or calling flush concurrently.
    pub fn poll(&mut self) -> Vec<u32> {
        let mut flags = Vec::with_capacity(self.instances.len());
        for (i, instance) in self.instances.iter_mut().enumerate() {
            let f = instance.service_main_thread();
            if f & 16 != 0 {
                self.shared.flush.fetch_or(1 << i, Ordering::Release);
            }
            if f & (1 | 8) != 0 {
                self.shared.stop.store(true, Ordering::Release);
            }
            let gui = instance.context.gui_flags.swap(0, Ordering::AcqRel);
            if gui & 8 != 0 {
                instance
                    .context
                    .gui_embedded
                    .store(false, Ordering::Release);
                self.editors[i] = None;
            } else if gui & 4 != 0 {
                if let Some(mut e) = self.editors[i].take() {
                    e.close();
                }
            }
            if gui & 2 != 0 {
                if let Some(e) = &self.editors[i] {
                    unsafe {
                        if let Some(hide) = (*e.extension).hide {
                            hide(e.plugin);
                        }
                    }
                }
            }
            if gui & 1 != 0 {
                if let Some(e) = &self.editors[i] {
                    unsafe {
                        if let Some(show) = (*e.extension).show {
                            show(e.plugin);
                        }
                    }
                }
            }
            flags.push(f);
        }
        self.poll_editors();
        flags
    }
    pub fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }
    /// Floating native editor on this owning GUI thread. Unsupported editors return an error.
    /// Host timer/fd services, embedded parenting and platform message pumps are not supplied.
    pub fn show_editor(&mut self, node: usize) -> Result<(), HostError> {
        if self
            .editors
            .get(node)
            .and_then(Option::as_ref)
            .is_some_and(|e| e.embedded)
        {
            self.close_editor(node);
        }
        if let Some(Some(e)) = self.editors.get(node) {
            return if unsafe { (*e.extension).show.ok_or("missing show")?(e.plugin) } {
                Ok(())
            } else {
                Err("editor show failed".into())
            };
        }
        let instance = self.instances.get(node).ok_or("unknown rack node")?;
        unsafe {
            let extension = (*instance.plugin).get_extension.expect("checked")(
                instance.plugin,
                CLAP_EXT_GUI.as_ptr(),
            )
            .cast::<clap_plugin_gui>();
            if extension.is_null() {
                return Err("plug-in has no GUI extension".into());
            }
            let e = &*extension;
            let api = if cfg!(target_os = "linux") {
                CLAP_WINDOW_API_X11
            } else if cfg!(target_os = "macos") {
                CLAP_WINDOW_API_COCOA
            } else {
                CLAP_WINDOW_API_WIN32
            };
            if !e.is_api_supported.ok_or("missing GUI support query")?(
                instance.plugin,
                api.as_ptr(),
                true,
            ) {
                return Err("plug-in does not support this floating editor API".into());
            }
            let destroy = e.destroy.ok_or("missing GUI destroy")?;
            let show = e.show.ok_or("missing GUI show")?;
            if !e.create.ok_or("missing GUI create")?(instance.plugin, api.as_ptr(), true) {
                return Err("editor creation failed".into());
            }
            let desired_scale = f64::from_bits(instance.context.gui_scale.load(Ordering::Acquire));
            let applied_scale = if let Some(scale) = e.set_scale {
                if scale(instance.plugin, desired_scale) {
                    desired_scale
                } else {
                    1.0
                }
            } else {
                1.0
            };
            if !show(instance.plugin) {
                destroy(instance.plugin);
                return Err("editor show failed".into());
            }
            let (mut width, mut height) = (640, 480);
            if let Some(size) = e.get_size {
                if !size(instance.plugin, &mut width, &mut height) {
                    width = 640;
                    height = 480;
                }
            }
            instance
                .context
                .gui_last_size
                .store(((width as u64) << 32) | height as u64, Ordering::Release);
            instance
                .context
                .gui_last_embedded
                .store(false, Ordering::Release);
            self.editors[node] = Some(Editor {
                plugin: instance.plugin,
                extension,
                scale: applied_scale,
                embedded: false,
                size: [width, height],
                context: &*instance.context,
                #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
                owned: None,
            });
            Ok(())
        }
    }
    pub fn close_editor(&mut self, node: usize) {
        if let Some(slot) = self.editors.get_mut(node) {
            if let Some(mut e) = slot.take() {
                e.close();
            }
        }
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(direct) = self.direct.take() {
            direct.revoke();
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                self.shared
                    .stats
                    .process_errors
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
impl Drop for LiveRack {
    fn drop(&mut self) {
        self.stop();
        for editor in &mut self.editors {
            if let Some(mut e) = editor.take() {
                e.close();
            }
        }
        for instance in &mut self.instances {
            instance.deactivate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn bridge() -> (RackProcessor, Consumer<Packet>, Producer<Packet>) {
        let (tx, rx) = RingBuffer::new(8);
        let (ot, or) = RingBuffer::new(8);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            panic: AtomicBool::new(false),
            flush: AtomicU32::new(0),
            bypass: AtomicU32::new(0),
            stats: Arc::new(Stats::default()),
        });
        (
            RackProcessor {
                tx,
                rx: or,
                input: Packet::default(),
                output: None,
                queued: None,
                position: 0,
                delay_blocks: 4,
                rate: 48000.0,
                shared,
                latency: BRIDGE_LATENCY,
            },
            rx,
            ot,
        )
    }
    fn callback(p: &mut RackProcessor, n: usize, at: u64) -> Vec<f32> {
        let mut data = vec![1.0; n * 2];
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: at,
                    tempo: 120.0,
                    playing: true,
                },
            },
        );
        data
    }
    #[test]
    fn variable_callback_sizes_and_exact_bridge_delay() {
        let (mut p, mut requests, mut results) = bridge();
        let mut out = callback(&mut p, 257, 0);
        let mut request = requests.pop().unwrap();
        request.samples.fill(0.25);
        results.push(request).ok().unwrap();
        out.extend(callback(&mut p, 767, 257));
        assert!(out.iter().all(|x| *x == 0.0));
        assert_eq!(callback(&mut p, 3, 1024), vec![0.25; 6]);
        assert_eq!(p.stats_snapshot().underruns, 0);
    }
    #[test]
    fn future_packet_survives_missing_block() {
        let (mut p, _, mut results) = bridge();
        let mut packet = Packet {
            sequence: 1,
            ..Default::default()
        };
        packet.samples.fill(0.5);
        results.push(packet).ok().unwrap();
        callback(&mut p, 1280, 0);
        assert_eq!(callback(&mut p, 1, 1280), vec![0.5; 2]);
        assert_eq!(p.stats_snapshot().underruns, 1);
    }
    #[test]
    fn callback_backpressure_is_bounded_and_heap_free() {
        let (mut p, _, _) = bridge();
        let mut data = [1.0; 512];
        let count = allocation_counter::measure(|| {
            for i in 0..20 {
                p.process(
                    &mut AudioBuffer::new(&mut data, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport {
                            sample_position: i * 256,
                            tempo: 120.0,
                            playing: true,
                        },
                    },
                );
            }
        });
        assert_eq!(count.count_total, 0);
        assert!(p.stats_snapshot().input_drops > 0);
        assert!(p.stats_snapshot().underruns > 0);
    }
    #[test]
    fn invalid_layout_and_stopped_owner_silence() {
        let (mut p, _, _) = bridge();
        assert!(p
            .prepare(ProcessSpec {
                sample_rate: 44100.0,
                max_block_frames: 256,
                channels: 2
            })
            .is_err());
        p.shared.stop.store(true, Ordering::Release);
        assert_eq!(callback(&mut p, 8, 0), vec![0.0; 16]);
    }
    impl RackProcessor {
        fn stats_snapshot(&self) -> Snapshot {
            self.shared.stats.snapshot()
        }
    }
}

#[cfg(test)]
mod buffered_config_tests {
    use super::tests::bridge;
    use super::*;
    #[test]
    fn one_packet_lookahead_has_one_packet_delay() {
        let (mut p, mut input, mut output) = bridge();
        p.delay_blocks = 1;
        let ctx = ProcessContext {
            events: &[],
            transport: Transport {
                sample_position: 0,
                tempo: 120.0,
                playing: true,
            },
        };
        let mut first = [1.0; BLOCK * 2];
        p.process(&mut AudioBuffer::new(&mut first, 2).unwrap(), &ctx);
        assert_eq!(first, [0.0; BLOCK * 2]);
        let mut packet = input.pop().unwrap();
        packet.samples.fill(0.25);
        output.push(packet).ok().unwrap();
        let mut next = [1.0; 2];
        p.process(&mut AudioBuffer::new(&mut next, 2).unwrap(), &ctx);
        assert_eq!(next, [0.25; 2]);
    }
}
