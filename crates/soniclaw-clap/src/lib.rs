//! CLAP 1.2 ABI adapter. Unsafe code is confined to this crate and export/host
//! tools. Hosts must obey CLAP pointer, lifetime and callback threading rules.
//! DSP is exclusively borrowed during activate/process/reset/deactivate; GUI and
//! parameter queries use shared atomics. The editor feature adds a host-parented GUI.
#![deny(unsafe_op_in_unsafe_fn)]
#[doc(hidden)]
pub use clap_sys as sys;
/// An immutable descriptor whose strings can only come from static CStr values.
pub struct ClapDescriptor {
    raw: clap_plugin_descriptor,
}
use clap_sys::{
    events::*,
    ext::{audio_ports::*, latency::*, note_ports::*, params::*, render::*, state::*, tail::*},
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use soniclaw_core::*;
use soniclaw_plugin::{parameter_id, PluginInstance};
use soniclaw_state::{Preset, ProjectState};
use std::{
    cell::UnsafeCell,
    ffi::{c_char, c_void, CStr},
    panic::{catch_unwind, AssertUnwindSafe},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, OnceLock,
    },
};
mod fp;
#[cfg(feature = "editor")]
mod gui;
const LIMIT: usize = 1024;
const MAX_PARAMETERS: usize = 256;
const MAX_STATE: usize = 4 * 1024 * 1024;
struct Features([*const c_char; 3]);
// SAFETY: Every pointer is a static immutable NUL-terminated string or null.
unsafe impl Sync for Features {}
static EFFECT: Features = Features([c"audio-effect".as_ptr(), c"utility".as_ptr(), ptr::null()]);
static SYNTH: Features = Features([c"instrument".as_ptr(), c"synthesizer".as_ptr(), ptr::null()]);
pub const fn descriptor(
    id: &'static CStr,
    name: &'static CStr,
    vendor: &'static CStr,
    version: &'static CStr,
    instrument: bool,
) -> ClapDescriptor {
    ClapDescriptor {
        raw: clap_plugin_descriptor {
            clap_version: CLAP_VERSION,
            id: id.as_ptr(),
            name: name.as_ptr(),
            vendor: vendor.as_ptr(),
            url: c"".as_ptr(),
            manual_url: c"".as_ptr(),
            support_url: c"".as_ptr(),
            version: version.as_ptr(),
            description: c"Built with Soniclaw".as_ptr(),
            features: if instrument {
                SYNTH.0.as_ptr()
            } else {
                EFFECT.0.as_ptr()
            },
        },
    }
}
pub struct Definition {
    pub descriptor: &'static ClapDescriptor,
    pub create: fn() -> PluginInstance,
    pub instrument: bool,
    pub tail_seconds: f64,
}
static DEFINITIONS: OnceLock<&'static [Definition]> = OnceLock::new();
#[doc(hidden)]
pub fn initialize(definitions: &'static [Definition]) -> bool {
    let current = DEFINITIONS.get_or_init(|| definitions);
    std::ptr::eq(*current, definitions)
}
#[macro_export]
macro_rules! export_clap {
    ($definitions:expr) => {
        static SONICLAW_DEFINITIONS: &[$crate::Definition] = &$definitions;
        unsafe extern "C" fn soniclaw_entry_init(_: *const std::ffi::c_char) -> bool {
            $crate::initialize(SONICLAW_DEFINITIONS)
        }
        unsafe extern "C" fn soniclaw_entry_deinit() {}
        #[no_mangle]
        pub static clap_entry: $crate::sys::entry::clap_plugin_entry =
            $crate::sys::entry::clap_plugin_entry {
                clap_version: $crate::sys::version::CLAP_VERSION,
                init: Some(soniclaw_entry_init),
                deinit: Some(soniclaw_entry_deinit),
                get_factory: Some($crate::get_factory),
            };
    };
}
struct Runtime {
    #[cfg(feature = "editor")]
    editor_events: soniclaw_editor::EditReceiver,
    #[cfg(feature = "editor")]
    editor_pending: Option<(soniclaw_editor::Edit, u8)>,
    #[cfg(feature = "editor")]
    editor_epoch: u64,
    processor: PluginInstance,
    audio: Vec<f32>,
    sidechain: Vec<f32>,
    audio64: Vec<f64>,
    midi: Vec<Event>,
    automation: Vec<AutomationEvent>,
    max_frames: usize,
    sample_rate: f64,
}
struct Instance {
    #[cfg(feature = "editor")]
    gui: UnsafeCell<gui::GuiState>,
    #[cfg(feature = "editor")]
    editor_sender: soniclaw_editor::EditSender,
    plugin: clap_plugin,
    definition: &'static Definition,
    host: *const clap_host,
    sidechain: bool,
    supports_f64: bool,
    parameters: Box<[Arc<Parameter>]>,
    runtime: UnsafeCell<Runtime>,
    // Main-thread-only project metadata; audio reads only the prepared Runtime.
    project: UnsafeCell<Option<ProjectState>>,
    active: AtomicBool,
    processing: AtomicBool,
    latency: AtomicU32,
    tail: AtomicU32,
    pending: Box<[AtomicU32]>,
    pending_epoch: AtomicU64,
    applied_epoch: AtomicU64,
}
impl Instance {
    fn parameter(&self, id: u32) -> Option<&Arc<Parameter>> {
        self.parameters
            .iter()
            .find(|p| parameter_id(p.info().id) == id)
    }
    fn apply_pending(&self) {
        let epoch = self.pending_epoch.load(Ordering::SeqCst);
        if epoch.is_multiple_of(2) && epoch != self.applied_epoch.load(Ordering::SeqCst) {
            let mut values = [0.0; MAX_PARAMETERS];
            for (i, p) in self.pending.iter().enumerate() {
                values[i] = f32::from_bits(p.load(Ordering::SeqCst));
            }
            if self.pending_epoch.load(Ordering::SeqCst) == epoch {
                for (p, value) in self.parameters.iter().zip(values) {
                    let _ = p.set(value);
                }
                self.applied_epoch.store(epoch, Ordering::SeqCst);
            }
        }
    }
}
// SAFETY: CLAP supplies a live pointer returned by this factory until destroy.
unsafe fn instance<'a>(plugin: *const clap_plugin) -> Option<&'a Instance> {
    unsafe { plugin.as_ref()?.plugin_data.cast::<Instance>().as_ref() }
}
unsafe extern "C" fn count(_: *const clap_plugin_factory) -> u32 {
    DEFINITIONS.get().map_or(0, |d| d.len() as u32)
}
unsafe extern "C" fn get_descriptor(
    _: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    DEFINITIONS
        .get()
        .and_then(|d| d.get(index as usize))
        .map_or(ptr::null(), |d| &d.descriptor.raw)
}
unsafe extern "C" fn create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    // SAFETY: host and id are valid for this factory call by the CLAP contract.
    unsafe {
        if host.is_null() || id.is_null() || !clap_version_is_compatible((*host).clap_version) {
            return ptr::null();
        }
        let Some(definition) = DEFINITIONS.get().and_then(|d| {
            d.iter()
                .find(|d| CStr::from_ptr(d.descriptor.raw.id) == CStr::from_ptr(id))
        }) else {
            return ptr::null();
        };
        catch_unwind(AssertUnwindSafe(|| {
            let processor = (definition.create)();
            if processor.descriptor.is_instrument != definition.instrument
                || processor.parameters().len() > MAX_PARAMETERS
                || processor.descriptor.id.as_bytes() != CStr::from_ptr(id).to_bytes()
            {
                return ptr::null();
            }
            let parameters = processor.parameters().to_vec().into_boxed_slice();
            let pending = parameters
                .iter()
                .map(|p| AtomicU32::new(p.get().to_bits()))
                .collect::<Vec<_>>()
                .into_boxed_slice();
            #[cfg(feature = "editor")]
            let (editor_sender, editor_events) = soniclaw_editor::edit_channel(1024);
            let mut boxed = Box::new(Instance {
                #[cfg(feature = "editor")]
                gui: UnsafeCell::new(gui::GuiState::default()),
                #[cfg(feature = "editor")]
                editor_sender,
                plugin: clap_plugin {
                    desc: &definition.descriptor.raw,
                    plugin_data: ptr::null_mut(),
                    init: Some(init),
                    destroy: Some(destroy),
                    activate: Some(activate),
                    deactivate: Some(deactivate),
                    start_processing: Some(start),
                    stop_processing: Some(stop),
                    reset: Some(reset),
                    process: Some(process),
                    get_extension: Some(extension),
                    on_main_thread: Some(main_thread),
                },
                sidechain: processor.has_sidechain(),
                supports_f64: processor.supports_f64(),
                definition,
                host,
                parameters,
                project: UnsafeCell::new(processor.project_state().cloned()),
                runtime: UnsafeCell::new(Runtime {
                    #[cfg(feature = "editor")]
                    editor_events,
                    #[cfg(feature = "editor")]
                    editor_pending: None,
                    #[cfg(feature = "editor")]
                    editor_epoch: 0,
                    processor,
                    audio: Vec::new(),
                    sidechain: Vec::new(),
                    audio64: Vec::new(),
                    midi: Vec::with_capacity(LIMIT),
                    automation: Vec::with_capacity(LIMIT),
                    max_frames: 0,
                    sample_rate: 48000.0,
                }),
                active: AtomicBool::new(false),
                processing: AtomicBool::new(false),
                latency: AtomicU32::new(0),
                tail: AtomicU32::new(0),
                pending,
                pending_epoch: AtomicU64::new(0),
                applied_epoch: AtomicU64::new(0),
            });
            let raw: &mut Instance = &mut boxed;
            raw.plugin.plugin_data = (raw as *mut Instance).cast();
            let pointer = &raw.plugin as *const clap_plugin;
            let _ = Box::into_raw(boxed);
            pointer
        }))
        .unwrap_or(ptr::null())
    }
}
unsafe extern "C" fn init(plugin: *const clap_plugin) -> bool {
    unsafe { instance(plugin).is_some() }
}
unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    // SAFETY: destroy occurs exactly once, after stopping/deactivation. The Box
    // owns plugin_data; pointer points into that same stable allocation.
    unsafe {
        if let Some(i) = instance(plugin) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                #[cfg(feature = "editor")]
                (*i.gui.get()).editor.take();
                drop(Box::from_raw(i.plugin.plugin_data.cast::<Instance>()))
            }));
        }
    }
}
unsafe extern "C" fn activate(plugin: *const clap_plugin, rate: f64, min: u32, max: u32) -> bool {
    // SAFETY: activation is serialized by the host, outside process callbacks.
    unsafe {
        let Some(i) = instance(plugin) else {
            return false;
        };
        if i.active.load(Ordering::Acquire) || min == 0 || max < min || max > 65536 {
            return false;
        }
        catch_unwind(AssertUnwindSafe(|| {
            let r = &mut *i.runtime.get();
            if r.processor
                .activate(ProcessSpec {
                    sample_rate: rate,
                    max_block_frames: max as usize,
                    channels: 2,
                })
                .is_err()
            {
                return false;
            }
            r.audio = vec![0.0; max as usize * 2];
            if i.supports_f64 {
                r.audio64.resize(max as usize * 2, 0.);
            }
            r.sidechain = if i.sidechain {
                vec![0.0; max as usize * 2]
            } else {
                Vec::new()
            };
            r.max_frames = max as usize;
            r.sample_rate = rate;
            i.latency.store(
                r.processor.latency_frames().min(u32::MAX as usize) as u32,
                Ordering::Release,
            );
            i.tail.store(
                (i.definition.tail_seconds.max(0.0) * rate).min(u32::MAX as f64) as u32,
                Ordering::Release,
            );
            i.active.store(true, Ordering::Release);
            true
        }))
        .unwrap_or(false)
    }
}
unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
    // SAFETY: host has stopped processing before deactivation.
    unsafe {
        if let Some(i) = instance(plugin) {
            i.processing.store(false, Ordering::Release);
            i.active.store(false, Ordering::Release);
            let _ = catch_unwind(AssertUnwindSafe(|| {
                (&mut *i.runtime.get()).processor.deactivate()
            }));
        }
    }
}
unsafe extern "C" fn start(plugin: *const clap_plugin) -> bool {
    unsafe {
        instance(plugin).is_some_and(|i| {
            if !i.active.load(Ordering::Acquire) {
                return false;
            }
            i.processing.store(true, Ordering::Release);
            true
        })
    }
}
unsafe extern "C" fn stop(plugin: *const clap_plugin) {
    unsafe {
        if let Some(i) = instance(plugin) {
            i.processing.store(false, Ordering::Release);
        }
    }
}
unsafe extern "C" fn reset(plugin: *const clap_plugin) {
    // SAFETY: reset never overlaps another audio callback for this instance.
    unsafe {
        if let Some(i) = instance(plugin) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                (&mut *i.runtime.get()).processor.reset()
            }));
        }
    }
}
unsafe extern "C" fn main_thread(plugin: *const clap_plugin) {
    #[cfg(feature = "editor")]
    unsafe {
        if let Some(i) = instance(plugin) {
            if let Some(e) = (&*i.gui.get()).editor.as_ref() {
                e.on_main_thread();
            }
        }
    }
    #[cfg(not(feature = "editor"))]
    let _ = plugin;
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(count),
    get_plugin_descriptor: Some(get_descriptor),
    create_plugin: Some(create),
};
/// # Safety
/// `id` must be a valid NUL-terminated string for this host factory query.
#[doc(hidden)]
pub unsafe extern "C" fn get_factory(id: *const c_char) -> *const c_void {
    unsafe {
        if !id.is_null() && CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
            (&FACTORY as *const clap_plugin_factory).cast()
        } else {
            ptr::null()
        }
    }
}
unsafe extern "C" fn process(
    plugin: *const clap_plugin,
    context: *const clap_process,
) -> clap_process_status {
    // SAFETY: process is exclusive per instance. All host buffers remain live for
    // frames_count frames. Input is copied before any potentially in-place output.
    unsafe {
        let _floating_point = fp::Guard::new();
        let result = catch_unwind(AssertUnwindSafe(|| process_inner(plugin, context)))
            .unwrap_or(CLAP_PROCESS_ERROR);
        if result == CLAP_PROCESS_ERROR {
            clear_output(
                context,
                instance(plugin).map_or(0, |i| (&*i.runtime.get()).max_frames),
            );
        }
        result
    }
}
// SAFETY: Host buffers meet CLAP's length contract. Called only after input reads.
unsafe fn clear_output(context: *const clap_process, max_frames: usize) {
    unsafe {
        let Some(c) = context.as_ref() else {
            return;
        };
        if c.frames_count > 65536 || c.audio_outputs.is_null() || c.audio_outputs_count == 0 {
            return;
        }
        let out = &mut *c.audio_outputs;
        if out.channel_count > 64 {
            return;
        }
        let frames = (c.frames_count as usize).min(max_frames);
        clear_channels(out.data32, out.channel_count as usize, frames, 0.0);
        clear_channels(out.data64, out.channel_count as usize, frames, 0.0);
        out.constant_mask = 0;
    }
}

// SAFETY: CLAP supplies a writable channel-pointer array and complete frames.
unsafe fn clear_channels<T: Copy>(planes: *mut *mut T, channels: usize, frames: usize, zero: T) {
    if !planes.is_null() {
        for i in 0..channels {
            let plane = unsafe { *planes.add(i) };
            if !plane.is_null() {
                unsafe { std::slice::from_raw_parts_mut(plane, frames).fill(zero) };
            }
        }
    }
}

// SAFETY: the host supplies this declared stereo plane array for the entire block.
unsafe fn stereo_planes<T>(planes: *mut *mut T) -> Option<[*mut T; 2]> {
    if planes.is_null() {
        return None;
    }
    let pair = unsafe { [*planes, *planes.add(1)] };
    pair.iter().all(|p| !p.is_null()).then_some(pair)
}

// SAFETY: each source plane contains at least one sample per nonconstant frame.
unsafe fn read_stereo<T: Copy>(destination: &mut [T], planes: [*mut T; 2], constant_mask: u64) {
    for (frame, pair) in destination.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        for (channel, (sample, plane)) in pair.iter_mut().zip(planes).enumerate() {
            let index = if constant_mask & (1 << channel) != 0 {
                0
            } else {
                frame
            };
            *sample = unsafe { *plane.add(index) };
        }
    }
}

// SAFETY: each plane is writable for all source frames and reads have completed.
unsafe fn write_stereo<T: Copy>(planes: [*mut T; 2], source: &[T]) {
    for (frame, pair) in source.as_chunks::<2>().0.iter().enumerate() {
        unsafe {
            *planes[0].add(frame) = pair[0];
            *planes[1].add(frame) = pair[1];
        }
    }
}

unsafe fn process_inner(
    plugin: *const clap_plugin,
    context: *const clap_process,
) -> clap_process_status {
    unsafe {
        let Some(i) = instance(plugin) else {
            return CLAP_PROCESS_ERROR;
        };
        let Some(context) = context.as_ref() else {
            return CLAP_PROCESS_ERROR;
        };
        if !i.active.load(Ordering::Acquire) || !i.processing.load(Ordering::Acquire) {
            return CLAP_PROCESS_ERROR;
        }
        let r = &mut *i.runtime.get();
        let frames = context.frames_count as usize;
        if frames > r.max_frames
            || context.audio_outputs_count != 1
            || context.audio_outputs.is_null()
        {
            return CLAP_PROCESS_ERROR;
        }
        let output = &mut *context.audio_outputs;
        if output.channel_count != 2 {
            return CLAP_PROCESS_ERROR;
        }
        let (out32, out64) = match (output.data32.is_null(), output.data64.is_null()) {
            (false, true) => (stereo_planes(output.data32), None),
            (true, false) if i.supports_f64 => (None, stereo_planes(output.data64)),
            _ => return CLAP_PROCESS_ERROR,
        };
        if out32.is_none() && out64.is_none() {
            return CLAP_PROCESS_ERROR;
        }
        if i.definition.instrument {
            if out64.is_some() {
                r.audio64[..frames * 2].fill(0.);
            } else {
                r.audio[..frames * 2].fill(0.);
            }
        } else {
            if !(1..=if i.sidechain { 2 } else { 1 }).contains(&context.audio_inputs_count)
                || context.audio_inputs.is_null()
            {
                return CLAP_PROCESS_ERROR;
            }
            let input = &*context.audio_inputs;
            if input.channel_count != 2 {
                return CLAP_PROCESS_ERROR;
            }
            if out64.is_some() {
                if !input.data32.is_null() {
                    return CLAP_PROCESS_ERROR;
                }
                let Some(planes) = stereo_planes(input.data64) else {
                    return CLAP_PROCESS_ERROR;
                };
                read_stereo(&mut r.audio64[..frames * 2], planes, input.constant_mask);
            } else {
                if !input.data64.is_null() {
                    return CLAP_PROCESS_ERROR;
                }
                let Some(planes) = stereo_planes(input.data32) else {
                    return CLAP_PROCESS_ERROR;
                };
                read_stereo(&mut r.audio[..frames * 2], planes, input.constant_mask);
            }
        }
        // Clear only on errors, preserving valid in-place host input until copied.
        if i.sidechain {
            r.sidechain[..frames * 2].fill(0.0);
            if context.audio_inputs_count == 2 {
                let key = &*context.audio_inputs.add(1);
                if key.channel_count != 0 {
                    if key.channel_count != 2 || key.data32.is_null() {
                        return CLAP_PROCESS_ERROR;
                    }
                    let sources = [*key.data32, *key.data32.add(1)];
                    if sources.iter().any(|p| p.is_null()) {
                        return CLAP_PROCESS_ERROR;
                    }
                    for (f, pair) in r.sidechain[..frames * 2]
                        .as_chunks_mut::<2>()
                        .0
                        .iter_mut()
                        .enumerate()
                    {
                        pair[0] = *sources[0].add(if key.constant_mask & 1 != 0 { 0 } else { f });
                        pair[1] = *sources[1].add(if key.constant_mask & 2 != 0 { 0 } else { f });
                    }
                }
            }
        }
        r.midi.clear();
        r.automation.clear();
        let mut last = 0;
        if let Some(list) = context.in_events.as_ref() {
            let (Some(size), Some(get)) = (list.size, list.get) else {
                return CLAP_PROCESS_ERROR;
            };
            let count = size(list) as usize;
            if count > LIMIT {
                return CLAP_PROCESS_ERROR;
            }
            for index in 0..count {
                let header = get(list, index as u32);
                let Some(h) = header.as_ref() else {
                    return CLAP_PROCESS_ERROR;
                };
                if (h.size as usize) < std::mem::size_of::<clap_event_header>()
                    || h.time < last
                    || h.time >= context.frames_count
                {
                    return CLAP_PROCESS_ERROR;
                }
                last = h.time;
                if h.space_id != CLAP_CORE_EVENT_SPACE_ID {
                    continue;
                }
                match h.type_ {
                    CLAP_EVENT_PARAM_VALUE
                        if h.size as usize >= std::mem::size_of::<clap_event_param_value>() =>
                    {
                        let e = &*header.cast::<clap_event_param_value>();
                        if e.note_id != -1 || e.channel != -1 || e.key != -1 || e.port_index != -1 {
                            continue;
                        }
                        if i.parameter(e.param_id).is_none() {
                            continue;
                        }
                        r.automation.push(AutomationEvent {
                            frame: e.header.time as usize,
                            parameter: e.param_id,
                            value: e.value as f32,
                        });
                    }
                    CLAP_EVENT_MIDI
                        if i.definition.instrument
                            && h.size as usize >= std::mem::size_of::<clap_event_midi>() =>
                    {
                        let e = &*header.cast::<clap_event_midi>();
                        if e.port_index != 0 {
                            continue;
                        }
                        let mut parser = soniclaw_midi::Parser::default();
                        for byte in e.data {
                            if let Some(message) = parser.feed(byte) {
                                r.midi.push(Event {
                                    frame: e.header.time as usize,
                                    message,
                                });
                            }
                        }
                    }
                    CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE
                        if i.definition.instrument
                            && h.size as usize >= std::mem::size_of::<clap_event_note>() =>
                    {
                        let e = &*header.cast::<clap_event_note>();
                        if !(-1..=0).contains(&e.port_index)
                            || !(-1..=15).contains(&e.channel)
                            || !(-1..=127).contains(&e.key)
                            || e.note_id < -1
                        {
                            continue;
                        }
                        let target = soniclaw_core::NoteTarget {
                            note_id: e.note_id,
                            channel: e.channel,
                            key: e.key,
                        };
                        let message = if h.type_ == CLAP_EVENT_NOTE_ON {
                            if e.channel < 0
                                || e.key < 0
                                || !e.velocity.is_finite()
                                || !(0.0..=1.0).contains(&e.velocity)
                            {
                                continue;
                            }
                            MidiMessage::NoteOnId {
                                channel: e.channel as u8,
                                note: e.key as u8,
                                velocity: e.velocity as f32,
                                note_id: e.note_id,
                            }
                        } else if h.type_ == CLAP_EVENT_NOTE_CHOKE {
                            MidiMessage::NoteChoke { target }
                        } else {
                            MidiMessage::NoteOffId { target }
                        };
                        r.midi.push(Event {
                            frame: h.time as usize,
                            message,
                        });
                    }
                    CLAP_EVENT_NOTE_EXPRESSION
                        if i.definition.instrument
                            && h.size as usize
                                >= std::mem::size_of::<clap_event_note_expression>() =>
                    {
                        let e = &*header.cast::<clap_event_note_expression>();
                        let target = soniclaw_core::NoteTarget {
                            note_id: e.note_id,
                            channel: e.channel,
                            key: e.key,
                        };
                        if !(-1..=0).contains(&e.port_index) || !target.valid() {
                            continue;
                        }
                        let expression = match e.expression_id {
                            CLAP_NOTE_EXPRESSION_TUNING => soniclaw_core::NoteExpression::Tuning,
                            CLAP_NOTE_EXPRESSION_PRESSURE => {
                                soniclaw_core::NoteExpression::Pressure
                            }
                            CLAP_NOTE_EXPRESSION_BRIGHTNESS => {
                                soniclaw_core::NoteExpression::Timbre
                            }
                            CLAP_NOTE_EXPRESSION_PAN => soniclaw_core::NoteExpression::Pan,
                            CLAP_NOTE_EXPRESSION_VOLUME => soniclaw_core::NoteExpression::Volume,
                            _ => continue,
                        };
                        if !soniclaw_core::VoiceExpression::default().set(expression, e.value) {
                            continue;
                        }
                        r.midi.push(Event {
                            frame: h.time as usize,
                            message: MidiMessage::NoteExpression {
                                target,
                                expression,
                                value: e.value,
                            },
                        });
                    }
                    _ => {}
                }
            }
        }
        i.apply_pending();
        #[cfg(feature = "editor")]
        emit_editor(i, r, context.out_events);
        if let Some(t) = context.transport.as_ref() {
            let tempo = if t.flags & CLAP_TRANSPORT_HAS_TEMPO != 0
                && t.tempo.is_finite()
                && t.tempo > 0.0
            {
                t.tempo
            } else {
                120.0
            };
            let seconds = if t.flags & CLAP_TRANSPORT_HAS_SECONDS_TIMELINE != 0 {
                t.song_pos_seconds as f64 / (1u64 << 31) as f64
            } else {
                context.steady_time.max(0) as f64 / r.sample_rate
            };
            let _ = r.processor.set_transport(Transport {
                sample_position: (seconds.max(0.0) * r.sample_rate) as u64,
                tempo,
                playing: t.flags & CLAP_TRANSPORT_IS_PLAYING != 0,
            });
        }
        if let Some(planes) = out64 {
            let audio = &mut r.audio64[..frames * 2];
            if r.processor
                .process_automated64(audio, &r.midi, &r.automation)
                .is_err()
            {
                return CLAP_PROCESS_ERROR;
            }
            write_stereo(planes, audio);
        } else if let Some(planes) = out32 {
            let audio = &mut r.audio[..frames * 2];
            if r.processor
                .process_with_sidechain(
                    audio,
                    if i.sidechain {
                        &r.sidechain[..frames * 2]
                    } else {
                        &[]
                    },
                    &r.midi,
                    &r.automation,
                )
                .is_err()
            {
                return CLAP_PROCESS_ERROR;
            }
            write_stereo(planes, audio);
        }
        output.constant_mask = 0;
        CLAP_PROCESS_CONTINUE
    }
}
unsafe extern "C" fn extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    // SAFETY: id is host-owned, NUL-terminated, and live for this query.
    unsafe {
        if id.is_null() {
            return ptr::null();
        }
        let id = CStr::from_ptr(id);
        #[cfg(feature = "editor")]
        if id == clap_sys::ext::gui::CLAP_EXT_GUI {
            return (&gui::GUI as *const clap_sys::ext::gui::clap_plugin_gui).cast();
        }
        if id == CLAP_EXT_PARAMS {
            (&PARAMS as *const clap_plugin_params).cast()
        } else if id == CLAP_EXT_AUDIO_PORTS {
            (&AUDIO_PORTS as *const clap_plugin_audio_ports).cast()
        } else if id == CLAP_EXT_NOTE_PORTS {
            (&NOTE_PORTS as *const clap_plugin_note_ports).cast()
        } else if id == CLAP_EXT_STATE {
            (&STATE as *const clap_plugin_state).cast()
        } else if id == CLAP_EXT_LATENCY {
            (&LATENCY as *const clap_plugin_latency).cast()
        } else if id == CLAP_EXT_TAIL {
            (&TAIL as *const clap_plugin_tail).cast()
        } else if id == CLAP_EXT_RENDER {
            (&RENDER as *const clap_plugin_render).cast()
        } else {
            ptr::null()
        }
    }
}
fn text<const N: usize>(target: &mut [c_char; N], value: &str) {
    target.fill(0);
    for (destination, source) in target
        .iter_mut()
        .take(N.saturating_sub(1))
        .zip(value.bytes())
    {
        *destination = source as c_char;
    }
}
unsafe extern "C" fn param_count(plugin: *const clap_plugin) -> u32 {
    unsafe { instance(plugin).map_or(0, |i| i.parameters.len() as u32) }
}
unsafe extern "C" fn param_info(
    plugin: *const clap_plugin,
    index: u32,
    out: *mut clap_param_info,
) -> bool {
    // SAFETY: out is host-provided writable storage for one info struct.
    unsafe {
        let (Some(i), Some(out)) = (instance(plugin), out.as_mut()) else {
            return false;
        };
        let Some(p) = i.parameters.get(index as usize) else {
            return false;
        };
        let info = p.info();
        *out = clap_param_info {
            id: parameter_id(info.id),
            flags: CLAP_PARAM_IS_AUTOMATABLE
                | if p.step_count() > 0 {
                    CLAP_PARAM_IS_STEPPED
                } else {
                    0
                }
                | if p.is_enum() { CLAP_PARAM_IS_ENUM } else { 0 },
            cookie: ptr::null_mut(),
            name: [0; 256],
            module: [0; 1024],
            min_value: info.min as f64,
            max_value: info.max as f64,
            default_value: info.default as f64,
        };
        text(&mut out.name, info.name);
        text(&mut out.module, p.group());
        true
    }
}
unsafe extern "C" fn param_value(plugin: *const clap_plugin, id: u32, out: *mut f64) -> bool {
    unsafe {
        let (Some(i), Some(out)) = (instance(plugin), out.as_mut()) else {
            return false;
        };
        if let Some(index) = i
            .parameters
            .iter()
            .position(|p| parameter_id(p.info().id) == id)
        {
            let epoch = i.pending_epoch.load(Ordering::SeqCst);
            let mut value = i.parameters[index].get();
            if epoch.is_multiple_of(2) && epoch != i.applied_epoch.load(Ordering::SeqCst) {
                let pending = f32::from_bits(i.pending[index].load(Ordering::SeqCst));
                if epoch == i.pending_epoch.load(Ordering::SeqCst) {
                    value = pending;
                }
            }
            *out = value as f64;
            true
        } else {
            false
        }
    }
}
unsafe extern "C" fn value_text(
    plugin: *const clap_plugin,
    id: u32,
    value: f64,
    buffer: *mut c_char,
    capacity: u32,
) -> bool {
    // SAFETY: output has capacity writable bytes, owned by host for the call.
    unsafe {
        let Some(i) = instance(plugin) else {
            return false;
        };
        if i.parameter(id).is_none() || buffer.is_null() || capacity == 0 || !value.is_finite() {
            return false;
        }
        let string = i.parameter(id).expect("checked").format_value(value as f32);
        let bytes = string.as_bytes();
        let count = bytes.len().min(capacity as usize - 1);
        ptr::copy_nonoverlapping(bytes.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
        true
    }
}
unsafe extern "C" fn text_value(
    plugin: *const clap_plugin,
    id: u32,
    value: *const c_char,
    out: *mut f64,
) -> bool {
    unsafe {
        let (Some(i), Some(out)) = (instance(plugin), out.as_mut()) else {
            return false;
        };
        let Some(p) = i.parameter(id) else {
            return false;
        };
        if value.is_null() {
            return false;
        }
        if let Some(v) = p.parse_value(&CStr::from_ptr(value).to_string_lossy()) {
            *out = v as f64;
            return true;
        }
        false
    }
}
unsafe extern "C" fn flush(
    plugin: *const clap_plugin,
    events: *const clap_input_events,
    out: *const clap_output_events,
) {
    // SAFETY: CLAP serializes flush with process, on the appropriate host thread.
    unsafe {
        let Some(i) = instance(plugin) else {
            return;
        };
        i.apply_pending();
        #[cfg(feature = "editor")]
        emit_editor(i, &mut *i.runtime.get(), out);
        #[cfg(not(feature = "editor"))]
        let _ = out;
        let Some(list) = events.as_ref() else {
            return;
        };
        let (Some(size), Some(get)) = (list.size, list.get) else {
            return;
        };
        for index in 0..size(list).min(LIMIT as u32) {
            let header = get(list, index);
            let Some(h) = header.as_ref() else {
                continue;
            };
            if h.space_id == CLAP_CORE_EVENT_SPACE_ID
                && h.type_ == CLAP_EVENT_PARAM_VALUE
                && h.size as usize >= std::mem::size_of::<clap_event_param_value>()
            {
                let e = &*header.cast::<clap_event_param_value>();
                if e.note_id == -1 && e.channel == -1 && e.key == -1 && e.port_index == -1 {
                    if let Some(p) = i.parameter(e.param_id) {
                        let _ = p.set(e.value as f32);
                    }
                }
            }
        }
    }
}
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(param_count),
    get_info: Some(param_info),
    get_value: Some(param_value),
    value_to_text: Some(value_text),
    text_to_value: Some(text_value),
    flush: Some(flush),
};
unsafe extern "C" fn audio_count(plugin: *const clap_plugin, input: bool) -> u32 {
    unsafe {
        instance(plugin).map_or(0, |i| {
            if input && i.definition.instrument {
                0
            } else if input && i.sidechain {
                2
            } else {
                1
            }
        })
    }
}
unsafe extern "C" fn audio_info(
    plugin: *const clap_plugin,
    index: u32,
    input: bool,
    out: *mut clap_audio_port_info,
) -> bool {
    unsafe {
        let (Some(i), Some(out)) = (instance(plugin), out.as_mut()) else {
            return false;
        };
        if index >= audio_count(plugin, input) {
            return false;
        }
        *out = clap_audio_port_info {
            id: if input {
                if index == 0 {
                    0
                } else {
                    2
                }
            } else {
                1
            },
            name: [0; 256],
            flags: (if index == 0 {
                CLAP_AUDIO_PORT_IS_MAIN
            } else {
                0
            }) | if i.supports_f64 {
                CLAP_AUDIO_PORT_SUPPORTS_64BITS | CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE
            } else {
                0
            },
            channel_count: 2,
            port_type: CLAP_PORT_STEREO.as_ptr(),
            in_place_pair: if i.definition.instrument || index != 0 {
                u32::MAX
            } else if input {
                1
            } else {
                0
            },
        };
        text(
            &mut out.name,
            if index == 1 {
                "Sidechain"
            } else if input {
                "Stereo Input"
            } else {
                "Stereo Output"
            },
        );
        true
    }
}
static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(audio_count),
    get: Some(audio_info),
};
unsafe extern "C" fn note_count(plugin: *const clap_plugin, input: bool) -> u32 {
    unsafe { instance(plugin).map_or(0, |i| u32::from(input && i.definition.instrument)) }
}
unsafe extern "C" fn note_info(
    plugin: *const clap_plugin,
    index: u32,
    input: bool,
    out: *mut clap_note_port_info,
) -> bool {
    unsafe {
        let (Some(i), Some(out)) = (instance(plugin), out.as_mut()) else {
            return false;
        };
        if !input || index != 0 || !i.definition.instrument {
            return false;
        }
        *out = clap_note_port_info {
            id: 0,
            supported_dialects: CLAP_NOTE_DIALECT_MIDI,
            preferred_dialect: CLAP_NOTE_DIALECT_MIDI,
            name: [0; 256],
        };
        text(&mut out.name, "MIDI Input");
        true
    }
}
static NOTE_PORTS: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_count),
    get: Some(note_info),
};
unsafe extern "C" fn save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    // SAFETY: host streams are valid for the entire state callback. State callbacks
    // run on the main thread; only scalar atomics are read from the render side.
    unsafe {
        let (Some(i), Some(stream)) = (instance(plugin), stream.as_ref()) else {
            return false;
        };
        let Some(write) = stream.write else {
            return false;
        };
        let product = CStr::from_ptr(i.definition.descriptor.raw.id).to_string_lossy();
        let Ok(mut preset) = Preset::capture(&product, "CLAP State", &i.parameters) else {
            return false;
        };
        let epoch = i.pending_epoch.load(Ordering::SeqCst);
        if epoch.is_multiple_of(2) && epoch != i.applied_epoch.load(Ordering::SeqCst) {
            let values: Vec<_> = i
                .pending
                .iter()
                .map(|p| f32::from_bits(p.load(Ordering::SeqCst)))
                .collect();
            if epoch == i.pending_epoch.load(Ordering::SeqCst) {
                for (p, value) in i.parameters.iter().zip(values) {
                    preset.parameters.insert(p.info().id.to_owned(), value);
                }
            }
        }
        // SAFETY: save/load run on the main thread. Project metadata is never
        // accessed by the render thread; only its scalar snapshot crosses over.
        let json = if let Some(project) = &*i.project.get() {
            let mut project = project.clone();
            project.parameters = preset.parameters;
            project.to_json()
        } else {
            preset.to_json()
        };
        let Ok(json) = json else {
            return false;
        };
        if json.len() > MAX_STATE {
            return false;
        }
        let mut remaining = json.as_bytes();
        while !remaining.is_empty() {
            let n = write(stream, remaining.as_ptr().cast(), remaining.len() as u64);
            if n <= 0 || n as usize > remaining.len() {
                return false;
            }
            remaining = &remaining[n as usize..];
        }
        true
    }
}
unsafe extern "C" fn load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    unsafe {
        let (Some(i), Some(stream)) = (instance(plugin), stream.as_ref()) else {
            return false;
        };
        let Some(read) = stream.read else {
            return false;
        };
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = read(stream, buffer.as_mut_ptr().cast(), buffer.len() as u64);
            if n < 0 || n as usize > buffer.len() {
                return false;
            }
            if n == 0 {
                break;
            }
            if bytes.len() + n as usize > MAX_STATE {
                return false;
            }
            bytes.extend_from_slice(&buffer[..n as usize]);
        }
        let Ok(json) = std::str::from_utf8(&bytes) else {
            return false;
        };
        let preset = if let Ok(preset) = Preset::from_json(json) {
            preset
        } else {
            let Ok(project) = ProjectState::from_json(json) else {
                return false;
            };
            let product = CStr::from_ptr(i.definition.descriptor.raw.id).to_string_lossy();
            if project.product_id != product || project.parameters.len() != i.parameters.len() {
                return false;
            }
            for p in &i.parameters {
                let info = p.info();
                let Some(value) = project.parameters.get(info.id) else {
                    return false;
                };
                if !value.is_finite() || !(info.min..=info.max).contains(value) {
                    return false;
                }
            }
            let Some(previous) = &*i.project.get() else {
                return false;
            };
            let mut a = previous.clone();
            let mut b = project.clone();
            a.parameters.clear();
            b.parameters.clear();
            a.name.clear();
            b.name.clear();
            if a != b {
                // Structural rebuild performs I/O/allocation only while deactivated.
                if i.active.load(Ordering::Acquire) {
                    return false;
                }
                let runtime = &mut *i.runtime.get();
                if !matches!(
                    catch_unwind(AssertUnwindSafe(|| runtime.processor.load_state(json))),
                    Ok(Ok(()))
                ) {
                    return false;
                }
            }
            *i.project.get() = Some(project.clone());
            Preset {
                schema_version: 1,
                product_id: project.product_id,
                name: project.name,
                parameters: project.parameters,
            }
        };
        let product = CStr::from_ptr(i.definition.descriptor.raw.id).to_string_lossy();
        if preset.schema_version != 1
            || preset.product_id != product
            || preset.parameters.len() != i.parameters.len()
        {
            return false;
        }
        for p in &i.parameters {
            let info = p.info();
            let Some(value) = preset.parameters.get(info.id) else {
                return false;
            };
            if !value.is_finite() || !(info.min..=info.max).contains(value) {
                return false;
            }
        }
        i.pending_epoch.fetch_add(1, Ordering::SeqCst);
        for (pending, p) in i.pending.iter().zip(&i.parameters) {
            pending.store(preset.parameters[p.info().id].to_bits(), Ordering::SeqCst);
        }
        i.pending_epoch.fetch_add(1, Ordering::SeqCst);
        if !i.processing.load(Ordering::Acquire) {
            i.apply_pending();
        }
        if let Some(get) = (*i.host).get_extension {
            let host_params = get(i.host, CLAP_EXT_PARAMS.as_ptr()).cast::<clap_host_params>();
            if let Some(params) = host_params.as_ref() {
                if let Some(rescan) = params.rescan {
                    rescan(i.host, CLAP_PARAM_RESCAN_VALUES);
                }
            }
        }
        if let Some(request) = (*i.host).request_process {
            request(i.host);
        }
        true
    }
}
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(save),
    load: Some(load),
};
unsafe extern "C" fn latency(plugin: *const clap_plugin) -> u32 {
    unsafe { instance(plugin).map_or(0, |i| i.latency.load(Ordering::Acquire)) }
}
unsafe extern "C" fn tail(plugin: *const clap_plugin) -> u32 {
    unsafe { instance(plugin).map_or(0, |i| i.tail.load(Ordering::Acquire)) }
}
unsafe extern "C" fn hard_rt(_: *const clap_plugin) -> bool {
    false
}
unsafe extern "C" fn render(_: *const clap_plugin, mode: i32) -> bool {
    mode == CLAP_RENDER_REALTIME || mode == CLAP_RENDER_OFFLINE
}
static LATENCY: clap_plugin_latency = clap_plugin_latency { get: Some(latency) };
static TAIL: clap_plugin_tail = clap_plugin_tail { get: Some(tail) };
static RENDER: clap_plugin_render = clap_plugin_render {
    has_hard_realtime_requirement: Some(hard_rt),
    set: Some(render),
};

#[cfg(feature = "editor")]
unsafe fn emit_editor(i: &Instance, runtime: &mut Runtime, out: *const clap_output_events) {
    unsafe {
        let epoch = i.applied_epoch.load(Ordering::SeqCst);
        if epoch != runtime.editor_epoch {
            runtime.editor_epoch = epoch;
            // Restoring host state wins over outstanding GUI edits. An already
            // emitted BEGIN must still get its END, even if its VALUE is canceled.
            if let Some((edit, stage)) = runtime.editor_pending.take() {
                runtime.editor_events.acknowledge(edit);
                if stage > 0 {
                    runtime.editor_pending = Some((edit, 2));
                }
            }
            for _ in 0..LIMIT {
                let Ok(edit) = runtime.editor_events.pop() else {
                    break;
                };
                runtime.editor_events.acknowledge(edit);
            }
        }
        let Some(out) = out.as_ref() else {
            return;
        };
        let Some(push) = out.try_push else {
            return;
        };
        for _ in 0..LIMIT * 3 {
            let (edit, stage) = match runtime.editor_pending {
                Some(e) => e,
                None => {
                    let Ok(e) = runtime.editor_events.pop() else {
                        break;
                    };
                    runtime.editor_pending = Some((e, 0));
                    (e, 0)
                }
            };
            let Some(p) = i.parameters.get(edit.index) else {
                runtime.editor_pending = None;
                continue;
            };
            let id = parameter_id(p.info().id);
            let accepted = if stage == 1 {
                let e = clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: CLAP_EVENT_IS_LIVE,
                    },
                    param_id: id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: edit.value as f64,
                };
                if push(out, &e.header) {
                    let _ = p.set(edit.value);
                    runtime.editor_events.acknowledge(edit);
                    true
                } else {
                    false
                }
            } else {
                let e = clap_event_param_gesture {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_gesture>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: if stage == 0 {
                            CLAP_EVENT_PARAM_GESTURE_BEGIN
                        } else {
                            CLAP_EVENT_PARAM_GESTURE_END
                        },
                        flags: CLAP_EVENT_IS_LIVE,
                    },
                    param_id: id,
                };
                push(out, &e.header)
            };
            if !accepted {
                break;
            }
            runtime.editor_pending = if stage == 2 {
                None
            } else {
                Some((edit, stage + 1))
            };
        }
    }
}

#[cfg(test)]
mod tests;
