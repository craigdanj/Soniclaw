//! Trusted in-process CLAP hosting. Main owners are !Send; audio is a scoped
//! lease on another OS thread, joined before state, deactivation or destruction.
pub mod buses;
pub mod direct;
pub mod editor_workspace;
pub mod editors;
pub mod isolation;
pub mod live;
pub mod reconfigure;
pub mod services;
pub mod session;
use clap_sys::{
    audio_buffer::*,
    entry::*,
    events::*,
    ext::{audio_ports::*, latency::*, note_ports::*, params::*, state::*, thread_check::*},
    factory::plugin_factory::*,
    fixedpoint::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use serde::{Deserialize, Serialize};
use soniclaw_core::*;
use std::{
    cell::Cell,
    ffi::{c_char, c_void, CStr, CString},
    path::{Path, PathBuf},
    ptr,
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
};
pub type HostError = Box<dyn std::error::Error + Send + Sync>;
const LIMIT: usize = 1024;
const STATE_LIMIT: usize = 4 * 1024 * 1024;
thread_local! {static AUDIO:Cell<bool>=const{Cell::new(false)};}
#[derive(Clone, Debug, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub value: f64,
    pub flags: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct ParameterEvent {
    pub frame: usize,
    pub id: u32,
    pub value: f64,
}
#[derive(Clone, Copy, Debug)]
pub enum OutputEvent {
    Midi(Event),
    Parameter(ParameterEvent),
    Gesture { frame: usize, id: u32, begin: bool },
}
pub fn discover(root: &Path) -> Result<Vec<PathBuf>, HostError> {
    if root.is_file() {
        return Ok(vec![root.canonicalize()?]);
    }
    let mut out = vec![];
    for e in std::fs::read_dir(root)?.take(2048) {
        let p = e?.path();
        if p.extension().is_some_and(|x| x == "clap") {
            if p.is_file() {
                out.push(p)
            } else {
                let d = p.join("Contents/MacOS");
                if d.is_dir() {
                    for e in std::fs::read_dir(d)?.take(32) {
                        let p = e?.path();
                        if p.is_file() {
                            out.push(p)
                        }
                    }
                }
            }
        }
        if out.len() >= 512 {
            break;
        }
    }
    out.sort();
    Ok(out)
}
struct LibraryData {
    _library: libloading::Library,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
    plugins: Vec<PluginInfo>,
}
impl Drop for LibraryData {
    fn drop(&mut self) {
        // SAFETY: last main-thread Rc; all instances already destroyed.
        unsafe {
            if let Some(f) = (*self.entry).deinit {
                f();
            }
        }
    }
}
#[derive(Clone)]
pub struct ClapLibrary {
    data: Rc<LibraryData>,
}
impl ClapLibrary {
    /// # Safety
    /// The selected native library must be trusted, conform to CLAP's ABI and
    /// lifecycle, and retain all pointers for their documented lifetimes. Clone
    /// this handle rather than independently initializing an already loaded file.
    pub unsafe fn load(path: &Path) -> Result<Self, HostError> {
        unsafe {
            let absolute = path.canonicalize()?;
            let cpath = CString::new(absolute.to_str().ok_or("CLAP path must be UTF-8")?)?;
            let library = libloading::Library::new(absolute)?;
            let entry = *library.get::<*const clap_plugin_entry>(b"clap_entry\0")?;
            if entry.is_null() || !clap_version_is_compatible((*entry).clap_version) {
                return Err("incompatible entry".into());
            }
            let init = (*entry).init.ok_or("missing entry init")?;
            let factory_fn = (*entry).get_factory.ok_or("missing factory callback")?;
            if (*entry).deinit.is_none() || !init(cpath.as_ptr()) {
                return Err("entry init failed".into());
            }
            let factory = factory_fn(CLAP_PLUGIN_FACTORY_ID.as_ptr()).cast::<clap_plugin_factory>();
            let mut data = LibraryData {
                _library: library,
                entry,
                factory,
                plugins: vec![],
            };
            if factory.is_null() {
                return Err("no CLAP factory".into());
            }
            let n = (*factory).get_plugin_count.ok_or("missing count")?(factory);
            if n == 0 || n > 256 {
                return Err("invalid product count".into());
            }
            let get = (*factory)
                .get_plugin_descriptor
                .ok_or("missing descriptor")?;
            if (*factory).create_plugin.is_none() {
                return Err("missing create".into());
            }
            for i in 0..n {
                let d = get(factory, i);
                if d.is_null() || !clap_version_is_compatible((*d).clap_version) {
                    return Err("invalid descriptor".into());
                }
                let id = string((*d).id);
                if id.is_empty() || data.plugins.iter().any(|p| p.id == id) {
                    return Err("invalid product identity".into());
                }
                data.plugins.push(PluginInfo {
                    id,
                    name: string((*d).name),
                    vendor: string((*d).vendor),
                    version: string((*d).version),
                });
            }
            Ok(Self {
                data: Rc::new(data),
            })
        }
    }
    pub fn plugins(&self) -> &[PluginInfo] {
        &self.data.plugins
    }
    pub fn instantiate(&self, id: &str) -> Result<ClapInstance, HostError> {
        unsafe {
            if !self.data.plugins.iter().any(|p| p.id == id) {
                return Err("unknown product".into());
            }
            let id = CString::new(id)?;
            let mut context = Box::new(HostContext {
                host: clap_host {
                    clap_version: CLAP_VERSION,
                    host_data: ptr::null_mut(),
                    name: c"Soniclaw Host".as_ptr(),
                    vendor: c"Soniclaw".as_ptr(),
                    url: c"".as_ptr(),
                    version: c"0.146.0".as_ptr(),
                    get_extension: Some(extension),
                    request_restart: Some(restart),
                    request_process: Some(request_process),
                    request_callback: Some(callback),
                },
                main: std::thread::current().id(),
                flags: AtomicU32::new(0),
                gui_flags: AtomicU32::new(0),
                gui_embedded: std::sync::atomic::AtomicBool::new(false),
                gui_resize: std::sync::atomic::AtomicU64::new(0),
                gui_scale: std::sync::atomic::AtomicU64::new(1.0f64.to_bits()),
                gui_last_size: std::sync::atomic::AtomicU64::new(0),
                gui_last_embedded: std::sync::atomic::AtomicBool::new(false),
                services: std::sync::Mutex::new(services::Services::default()),
            });
            context.host.host_data = (&*context as *const HostContext).cast_mut().cast();
            let factory = &*self.data.factory;
            let plugin =
                factory.create_plugin.expect("checked")(factory, &context.host, id.as_ptr());
            if plugin.is_null() {
                return Err("create failed".into());
            }
            let mut instance = ClapInstance {
                _library: self.clone(),
                context,
                plugin,
                active: false,
                latency: 0,
                parameters: vec![],
                resources: None,
                spec: None,
                input_channels: 0,
                output_channels: 0,
                note_dialect: 0,
                input_ports: vec![],
                output_ports: vec![],
                supports_f64: false,
            };
            let p = &*plugin;
            if p.destroy.is_none() {
                std::mem::forget(instance);
                return Err("malformed ABI: missing destroy".into());
            }
            if p.init.is_none()
                || p.activate.is_none()
                || p.deactivate.is_none()
                || p.start_processing.is_none()
                || p.stop_processing.is_none()
                || p.process.is_none()
                || p.get_extension.is_none()
            {
                return Err("missing required lifecycle callback".into());
            }
            if !p.init.expect("checked")(plugin) {
                return Err("plug-in init failed".into());
            }
            instance.inspect()?;
            Ok(instance)
        }
    }
}
unsafe fn string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
struct HostContext {
    host: clap_host,
    main: std::thread::ThreadId,
    flags: AtomicU32,
    gui_flags: AtomicU32,
    gui_embedded: std::sync::atomic::AtomicBool,
    gui_resize: std::sync::atomic::AtomicU64,
    gui_scale: std::sync::atomic::AtomicU64,
    gui_last_size: std::sync::atomic::AtomicU64,
    gui_last_embedded: std::sync::atomic::AtomicBool,
    services: std::sync::Mutex<services::Services>,
}
// SAFETY: host table/main ID/pointer are immutable; flags atomic, audio scoped.
unsafe impl Sync for HostContext {}
unsafe fn context<'a>(h: *const clap_host) -> &'a HostContext {
    unsafe { &*((*h).host_data.cast::<HostContext>()) }
}
unsafe extern "C" fn restart(h: *const clap_host) {
    unsafe { context(h) }.flags.fetch_or(1, Ordering::Relaxed);
}
unsafe extern "C" fn callback(h: *const clap_host) {
    unsafe { context(h) }.flags.fetch_or(2, Ordering::Relaxed);
}
unsafe extern "C" fn request_process(h: *const clap_host) {
    unsafe { context(h) }.flags.fetch_or(4, Ordering::Relaxed);
}
unsafe extern "C" fn rescan(h: *const clap_host, _: u32) {
    unsafe { context(h) }.flags.fetch_or(8, Ordering::Relaxed);
}
unsafe extern "C" fn clear(h: *const clap_host, _: u32, _: u32) {
    unsafe { context(h) }.flags.fetch_or(8, Ordering::Relaxed);
}
unsafe extern "C" fn flush(h: *const clap_host) {
    unsafe { context(h) }.flags.fetch_or(16, Ordering::Relaxed);
}
unsafe extern "C" fn dirty(h: *const clap_host) {
    unsafe { context(h) }.flags.fetch_or(32, Ordering::Relaxed);
}
unsafe extern "C" fn is_main(h: *const clap_host) -> bool {
    std::thread::current().id() == unsafe { context(h) }.main && !AUDIO.get()
}
unsafe extern "C" fn is_audio(_: *const clap_host) -> bool {
    AUDIO.get()
}
static THREAD: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main),
    is_audio_thread: Some(is_audio),
};
static PARAMS: clap_host_params = clap_host_params {
    rescan: Some(rescan),
    clear: Some(clear),
    request_flush: Some(flush),
};
static STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(dirty),
};
static LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(restart),
};
unsafe extern "C" fn port_rescan_supported(_: *const clap_host, flag: u32) -> bool {
    flag != 0 && flag & !63 == 0
}
unsafe extern "C" fn port_rescan(host: *const clap_host, _: u32) {
    unsafe { context(host) }
        .flags
        .fetch_or(64, Ordering::Release);
}
static AUDIO_PORT_HOST: clap_host_audio_ports = clap_host_audio_ports {
    is_rescan_flag_supported: Some(port_rescan_supported),
    rescan: Some(port_rescan),
};

unsafe extern "C" fn gui_show(h: *const clap_host) -> bool {
    unsafe { context(h) }
        .gui_flags
        .fetch_or(1, Ordering::Relaxed);
    true
}
unsafe extern "C" fn gui_hide(h: *const clap_host) -> bool {
    unsafe { context(h) }
        .gui_flags
        .fetch_or(2, Ordering::Relaxed);
    true
}
unsafe extern "C" fn gui_closed(h: *const clap_host, destroyed: bool) {
    unsafe { context(h) }
        .gui_flags
        .fetch_or(if destroyed { 8 } else { 4 }, Ordering::Relaxed);
}
unsafe extern "C" fn gui_resize(host: *const clap_host, w: u32, h: u32) -> bool {
    let c = unsafe { context(host) };
    if !(64..=8192).contains(&w)
        || !(64..=8192).contains(&h)
        || !c.gui_embedded.load(Ordering::Acquire)
    {
        return false;
    }
    c.gui_resize
        .store(((w as u64) << 32) | h as u64, Ordering::Release);
    true
}
unsafe extern "C" fn gui_hints(_: *const clap_host) {}
static GUI: clap_sys::ext::gui::clap_host_gui = clap_sys::ext::gui::clap_host_gui {
    resize_hints_changed: Some(gui_hints),
    request_resize: Some(gui_resize),
    request_show: Some(gui_show),
    request_hide: Some(gui_hide),
    closed: Some(gui_closed),
};
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = unsafe { CStr::from_ptr(id) };
    if let Some(service) = services::extension(id) {
        return service;
    }
    if id == clap_sys::ext::gui::CLAP_EXT_GUI {
        (&GUI as *const clap_sys::ext::gui::clap_host_gui).cast()
    } else if id == CLAP_EXT_AUDIO_PORTS {
        (&AUDIO_PORT_HOST as *const clap_host_audio_ports).cast()
    } else if id == CLAP_EXT_THREAD_CHECK {
        (&THREAD as *const clap_host_thread_check).cast()
    } else if id == CLAP_EXT_PARAMS {
        (&PARAMS as *const clap_host_params).cast()
    } else if id == CLAP_EXT_STATE {
        (&STATE as *const clap_host_state).cast()
    } else if id == CLAP_EXT_LATENCY {
        (&LATENCY as *const clap_host_latency).cast()
    } else {
        ptr::null()
    }
}
pub struct ClapInstance {
    _library: ClapLibrary,
    context: Box<HostContext>,
    plugin: *const clap_plugin,
    active: bool,
    latency: usize,
    parameters: Vec<ParameterInfo>,
    resources: Option<Resources>,
    spec: Option<ProcessSpec>,
    input_channels: usize,
    output_channels: usize,
    note_dialect: u32,
    input_ports: Vec<buses::Port>,
    output_ports: Vec<buses::Port>,
    supports_f64: bool,
}
impl ClapInstance {
    fn inspect(&mut self) -> Result<(), HostError> {
        unsafe {
            let p = &*self.plugin;
            let ports =
                p.get_extension.expect("checked")(self.plugin, CLAP_EXT_AUDIO_PORTS.as_ptr())
                    .cast::<clap_plugin_audio_ports>();
            if ports.is_null() {
                return Err("audio ports extension required".into());
            }
            let ports = &*ports;
            let count = ports.count.ok_or("missing port count")?;
            let get = ports.get.ok_or("missing port getter")?;
            self.supports_f64 = true;
            self.input_ports.clear();
            self.output_ports.clear();
            for input in [false, true] {
                let n = count(self.plugin, input);
                if n > 8 || (!input && n == 0) {
                    return Err(
                        "supports at most 8 ports per direction and requires an output".into(),
                    );
                }
                let mut total = 0;
                for index in 0..n {
                    let mut info: clap_audio_port_info = std::mem::zeroed();
                    if !get(self.plugin, index, input, &mut info)
                        || !(1..=64).contains(&info.channel_count)
                    {
                        return Err("invalid bus channel layout".into());
                    }
                    self.supports_f64 &= info.flags & CLAP_AUDIO_PORT_SUPPORTS_64BITS != 0;
                    total += info.channel_count;
                    if total > 64 {
                        return Err("aggregate bus channels exceed 64".into());
                    }
                    let name = String::from_utf8_lossy(
                        &info
                            .name
                            .iter()
                            .take_while(|c| **c != 0)
                            .map(|c| *c as u8)
                            .collect::<Vec<_>>(),
                    )
                    .into_owned();
                    let port = buses::Port {
                        id: info.id,
                        name,
                        channels: info.channel_count as usize,
                        main: info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0,
                    };
                    if input {
                        self.input_ports.push(port);
                    } else {
                        self.output_ports.push(port);
                    }
                }
            }
            self.input_channels = self.input_ports.first().map_or(0, |p| p.channels);
            self.output_channels = self.output_ports[0].channels;
            self.note_dialect = 0;
            let notes =
                p.get_extension.expect("checked")(self.plugin, CLAP_EXT_NOTE_PORTS.as_ptr())
                    .cast::<clap_plugin_note_ports>();
            if !notes.is_null() {
                let notes = &*notes;
                let n = notes.count.ok_or("missing note count")?(self.plugin, true);
                if n > 1 {
                    return Err("multiple note inputs unsupported".into());
                }
                if n == 1 {
                    let mut info: clap_note_port_info = std::mem::zeroed();
                    if !notes.get.ok_or("missing note get")?(self.plugin, 0, true, &mut info) {
                        return Err("note query failed".into());
                    }
                    self.note_dialect = if info.supported_dialects & CLAP_NOTE_DIALECT_CLAP != 0 {
                        CLAP_NOTE_DIALECT_CLAP
                    } else if info.supported_dialects & CLAP_NOTE_DIALECT_MIDI != 0 {
                        CLAP_NOTE_DIALECT_MIDI
                    } else {
                        return Err("unsupported note dialect".into());
                    };
                }
            }
            self.parameters.clear();
            let ext = p.get_extension.expect("checked")(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>();
            if !ext.is_null() {
                let ext = &*ext;
                let n = ext.count.ok_or("missing param count")?(self.plugin);
                if n > 4096 {
                    return Err("parameter limit exceeded".into());
                }
                for i in 0..n {
                    let mut info: clap_param_info = std::mem::zeroed();
                    if !ext.get_info.ok_or("missing get_info")?(self.plugin, i, &mut info) {
                        return Err("param query failed".into());
                    }
                    let mut value = info.default_value;
                    if let Some(get) = ext.get_value {
                        if !get(self.plugin, info.id, &mut value) {
                            return Err("value query failed".into());
                        }
                    }
                    if info.id == u32::MAX
                        || self.parameters.iter().any(|p| p.id == info.id)
                        || ![info.min_value, info.max_value, info.default_value, value]
                            .iter()
                            .all(|x| x.is_finite())
                        || info.min_value > info.max_value
                        || !(info.min_value..=info.max_value).contains(&info.default_value)
                        || !(info.min_value..=info.max_value).contains(&value)
                    {
                        return Err("invalid parameter metadata".into());
                    }
                    let n = info
                        .name
                        .iter()
                        .position(|x| *x == 0)
                        .unwrap_or(info.name.len());
                    let name = String::from_utf8_lossy(
                        &info.name[..n].iter().map(|x| *x as u8).collect::<Vec<_>>(),
                    )
                    .into_owned();
                    self.parameters.push(ParameterInfo {
                        id: info.id,
                        name,
                        min: info.min_value,
                        max: info.max_value,
                        default: info.default_value,
                        value,
                        flags: info.flags,
                    });
                }
            }
            Ok(())
        }
    }
    pub fn parameters(&self) -> &[ParameterInfo] {
        &self.parameters
    }
    /// Control-thread native parameter display conversion, with bounded output text.
    pub fn parameter_text(&self, id: u32, value: f64) -> Result<String, HostError> {
        let info = self
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or("unknown parameter")?;
        if !value.is_finite() || !(info.min..=info.max).contains(&value) {
            return Err("invalid parameter value".into());
        }
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                clap_sys::ext::params::CLAP_EXT_PARAMS.as_ptr(),
            )
            .cast::<clap_sys::ext::params::clap_plugin_params>();
            if ext.is_null() {
                return Err("missing parameter extension".into());
            }
            let mut buffer = [0 as c_char; 256];
            if !(*ext).value_to_text.ok_or("missing value formatter")?(
                self.plugin,
                id,
                value,
                buffer.as_mut_ptr(),
                256,
            ) {
                return Err("native value formatter refused".into());
            }
            let bytes = std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len());
            Ok(CStr::from_bytes_until_nul(bytes)?
                .to_string_lossy()
                .into_owned())
        }
    }
    /// Control-thread native text parsing. Verifies the resulting finite value is within the declared range.
    pub fn parameter_from_text(&self, id: u32, text: &str) -> Result<f64, HostError> {
        let info = self
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or("unknown parameter")?;
        if text.len() > 255 {
            return Err("parameter text too long".into());
        }
        let text = CString::new(text)?;
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                clap_sys::ext::params::CLAP_EXT_PARAMS.as_ptr(),
            )
            .cast::<clap_sys::ext::params::clap_plugin_params>();
            if ext.is_null() {
                return Err("missing parameter extension".into());
            }
            let mut value = 0.;
            if !(*ext).text_to_value.ok_or("missing parameter parser")?(
                self.plugin,
                id,
                text.as_ptr(),
                &mut value,
            ) || !value.is_finite()
                || !(info.min..=info.max).contains(&value)
            {
                return Err("native parameter text invalid".into());
            }
            Ok(value)
        }
    }
    pub fn main_channels(&self) -> (usize, usize) {
        (self.input_channels, self.output_channels)
    }
    pub fn latency_frames(&self) -> usize {
        self.latency
    }
    pub fn supports_f64(&self) -> bool {
        self.supports_f64
    }

    pub fn activate(&mut self, s: ProcessSpec) -> Result<(), HostError> {
        s.validate()?;
        if self.active {
            return Err("deactivate before activation".into());
        }
        self.inspect()?;
        if self.input_ports.len() > 1
            || self.output_ports.len() != 1
            || self.input_channels > 2
            || self.output_channels > 2
        {
            return Err("use with_bus_audio for this multi-bus layout".into());
        }
        if s.channels != self.output_channels || s.max_block_frames > 8192 {
            return Err("unsupported stream layout/block limit".into());
        }
        unsafe {
            if !(*self.plugin).activate.expect("checked")(
                self.plugin,
                s.sample_rate,
                1,
                s.max_block_frames as u32,
            ) {
                return Err("activation failed".into());
            }
            self.active = true;
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                CLAP_EXT_LATENCY.as_ptr(),
            )
            .cast::<clap_plugin_latency>();
            self.latency = if ext.is_null() {
                0
            } else {
                (*ext).get.ok_or("missing latency getter")?(self.plugin) as usize
            };
        }
        self.resources = Some(Resources::new(s.max_block_frames, self.supports_f64));
        self.spec = Some(s);
        Ok(())
    }
    pub fn deactivate(&mut self) {
        if self.active {
            unsafe { (*self.plugin).deactivate.expect("checked")(self.plugin) }
            self.active = false;
        }
        self.resources = None;
        self.spec = None;
    }
    pub fn service_main_thread(&mut self) -> u32 {
        let flags = self.context.flags.swap(0, Ordering::AcqRel);
        if flags & 2 != 0 {
            unsafe {
                if let Some(f) = (*self.plugin).on_main_thread {
                    f(self.plugin)
                }
            }
        }
        services::pump(self);
        flags
    }
    /// Main waits for the complete scoped block loop; service callbacks between scopes.
    pub fn run_audio<R: Send, F: for<'a> FnOnce(&mut HostedAudio<'a>) -> R + Send>(
        &mut self,
        f: F,
    ) -> Result<R, HostError> {
        if !self.active {
            return Err("activate before audio".into());
        }
        let resources = self
            .resources
            .take()
            .ok_or("reactivate after panicked audio scope")?;
        let mut audio = HostedAudio {
            plugin: self.plugin,
            _context: &self.context,
            parameters: &self.parameters,
            resources,
            spec: self.spec.expect("active"),
            input_channels: self.input_channels,
            note_dialect: self.note_dialect,
            processing: false,
            steady: 0,
        };
        let r = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    AUDIO.set(true);
                    unsafe {
                        if !(*audio.plugin).start_processing.expect("checked")(audio.plugin) {
                            AUDIO.set(false);
                            return Err("start_processing failed");
                        }
                    }
                    audio.processing = true;
                    let result = f(&mut audio);
                    audio.stop();
                    let resources = std::mem::replace(&mut audio.resources, Resources::empty());
                    AUDIO.set(false);
                    Ok((resources, result))
                })
                .join()
        });
        match r {
            Ok(Ok((resources, result))) => {
                self.resources = Some(resources);
                Ok(result)
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err("audio scope panicked; deactivate/reactivate".into()),
        }
    }
    pub fn save_state(&self) -> Result<Vec<u8>, HostError> {
        if self.active {
            return Err("deactivate before state I/O".into());
        }
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                CLAP_EXT_STATE.as_ptr(),
            )
            .cast::<clap_plugin_state>();
            if ext.is_null() {
                return Err("state extension unavailable".into());
            }
            let mut s = StateStream {
                bytes: vec![],
                position: 0,
                failed: false,
            };
            let stream = clap_ostream {
                ctx: (&mut s as *mut StateStream).cast(),
                write: Some(write),
            };
            if !(*ext).save.ok_or("missing save")?(self.plugin, &stream) || s.failed {
                return Err("state save failed/too large".into());
            }
            Ok(s.bytes)
        }
    }
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), HostError> {
        if self.active || data.len() > STATE_LIMIT {
            return Err("deactivate before bounded state load".into());
        }
        unsafe {
            let ext = (*self.plugin).get_extension.expect("checked")(
                self.plugin,
                CLAP_EXT_STATE.as_ptr(),
            )
            .cast::<clap_plugin_state>();
            if ext.is_null() {
                return Err("state extension unavailable".into());
            }
            let mut s = StateStream {
                bytes: data.to_vec(),
                position: 0,
                failed: false,
            };
            let stream = clap_istream {
                ctx: (&mut s as *mut StateStream).cast(),
                read: Some(read),
            };
            if !(*ext).load.ok_or("missing load")?(self.plugin, &stream) || s.failed {
                return Err("state restore failed; rollback is plug-in-defined".into());
            }
            self.inspect()
        }
    }
}
impl Drop for ClapInstance {
    fn drop(&mut self) {
        self.deactivate();
        unsafe {
            if let Some(f) = (*self.plugin).destroy {
                f(self.plugin)
            }
        }
    }
}
struct StateStream {
    bytes: Vec<u8>,
    position: usize,
    failed: bool,
}
unsafe extern "C" fn write(s: *const clap_ostream, p: *const c_void, n: u64) -> i64 {
    let s = unsafe { &mut *((*s).ctx.cast::<StateStream>()) };
    let n = n.min(65536) as usize;
    if p.is_null() || s.bytes.len().saturating_add(n) > STATE_LIMIT {
        s.failed = true;
        return -1;
    }
    s.bytes
        .extend_from_slice(unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n) });
    n as i64
}
unsafe extern "C" fn read(s: *const clap_istream, p: *mut c_void, n: u64) -> i64 {
    let s = unsafe { &mut *((*s).ctx.cast::<StateStream>()) };
    let n = (n.min(65536) as usize).min(s.bytes.len() - s.position);
    if p.is_null() {
        s.failed = true;
        return -1;
    }
    unsafe { ptr::copy_nonoverlapping(s.bytes[s.position..].as_ptr(), p.cast::<u8>(), n) }
    s.position += n;
    n as i64
}
#[repr(C)]
#[derive(Clone, Copy)]
union NativeEvent {
    header: clap_event_header,
    note: clap_event_note,
    param: clap_event_param_value,
    midi: clap_event_midi,
    expression: clap_event_note_expression,
}
struct Planes<T> {
    input: [Vec<T>; 2],
    output: [Vec<T>; 2],
}
impl<T: Copy + Default> Planes<T> {
    fn new(max: usize) -> Self {
        Self {
            input: std::array::from_fn(|_| vec![T::default(); max]),
            output: std::array::from_fn(|_| vec![T::default(); max]),
        }
    }
    fn prepare(
        &mut self,
        data: &[T],
        channels: usize,
        input_channels: usize,
        average: impl Fn(T, T) -> T,
    ) {
        for (i, frame) in data.chunks_exact(channels).enumerate() {
            self.input[0][i] = if input_channels == 1 && channels == 2 {
                average(frame[0], frame[1])
            } else {
                frame[0]
            };
            self.input[1][i] = frame[usize::from(channels == 2)];
            self.output[0][i] = T::default();
            self.output[1][i] = T::default();
        }
    }
    fn read(&self, data: &mut [T], channels: usize, constant: u64, finite: impl Fn(T) -> bool) {
        for (frame, samples) in data.chunks_exact_mut(channels).enumerate() {
            for (channel, sample) in samples.iter_mut().enumerate() {
                let index = if constant & (1 << channel) != 0 {
                    0
                } else {
                    frame
                };
                let value = self.output[channel][index];
                *sample = if finite(value) { value } else { T::default() };
            }
        }
    }
}

enum Pcm<'a> {
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
}
impl Pcm<'_> {
    fn len(&self) -> usize {
        match self {
            Self::F32(x) => x.len(),
            Self::F64(x) => x.len(),
        }
    }
    fn clear(&mut self) {
        match self {
            Self::F32(x) => x.fill(0.),
            Self::F64(x) => x.fill(0.),
        }
    }
}

struct Resources {
    pcm: Planes<f32>,
    wide: Option<Planes<f64>>,
    events: Vec<NativeEvent>,
    pointers: Vec<*const clap_event_header>,
    out: Vec<OutputEvent>,
    dropped: u64,
}
// SAFETY: pointers refer to the owned event Vec allocation, used exclusively in a joined scope.
unsafe impl Send for Resources {}
impl Resources {
    fn new(max: usize, wide: bool) -> Self {
        Self {
            pcm: Planes::new(max),
            wide: wide.then(|| Planes::new(max)),
            events: Vec::with_capacity(LIMIT),
            pointers: Vec::with_capacity(LIMIT),
            out: Vec::with_capacity(LIMIT),
            dropped: 0,
        }
    }
    fn empty() -> Self {
        Self {
            pcm: Planes::new(0),
            wide: None,
            events: Vec::new(),
            pointers: Vec::new(),
            out: Vec::new(),
            dropped: 0,
        }
    }
}
pub struct HostedAudio<'a> {
    plugin: *const clap_plugin,
    _context: &'a HostContext,
    parameters: &'a [ParameterInfo],
    resources: Resources,
    spec: ProcessSpec,
    input_channels: usize,
    note_dialect: u32,
    processing: bool,
    steady: i64,
}
// SAFETY: exclusive instance borrow; endpoint moves only into a scoped audio thread
// which joins before main methods and destruction. No resource lifetime escapes.
unsafe impl Send for HostedAudio<'_> {}
impl HostedAudio<'_> {
    fn stop(&mut self) {
        if self.processing {
            unsafe { (*self.plugin).stop_processing.expect("checked")(self.plugin) }
            self.processing = false;
        }
    }
    pub fn output_events(&self) -> &[OutputEvent] {
        &self.resources.out
    }
    pub fn dropped_output_events(&self) -> u64 {
        self.resources.dropped
    }
    pub fn process_block(
        &mut self,
        data: &mut [f32],
        events: &[Event],
        automation: &[ParameterEvent],
        transport: Transport,
    ) -> Result<i32, Error> {
        self.process_context(Pcm::F32(data), events, automation, transport, None)
    }
    /// Process directly through CLAP data64. All negotiated audio ports must advertise f64.
    pub fn process_block64(
        &mut self,
        data: &mut [f64],
        events: &[Event],
        automation: &[ParameterEvent],
        transport: Transport,
    ) -> Result<i32, Error> {
        if self.resources.wide.is_none() {
            data.fill(0.);
            return Err(Error::InvalidSpec);
        }
        self.process_context(Pcm::F64(data), events, automation, transport, None)
            .inspect_err(|_| data.fill(0.))
    }

    /// Supply the shared clock's mapped PPQ, meter and loop metadata at block start.
    pub fn process_musical(
        &mut self,
        data: &mut [f32],
        events: &[Event],
        automation: &[ParameterEvent],
        frame: soniclaw_transport::Frame,
    ) -> Result<i32, Error> {
        let p = frame.position;
        if !p.beat.is_finite()
            || p.beat < 0.0
            || !p.beat_in_bar.is_finite()
            || p.beat_in_bar < 0.0
            || p.numerator == 0
            || p.denominator == 0
            || !p.denominator.is_power_of_two()
            || p.bar > i32::MAX as u64
            || frame
                .loop_range
                .is_some_and(|r| r.iter().any(|x| !x.is_finite() || *x < 0.0) || r[0] >= r[1])
            || frame
                .loop_seconds
                .is_some_and(|r| r.iter().any(|x| !x.is_finite() || *x < 0.0) || r[0] >= r[1])
        {
            data.fill(0.0);
            return Err(Error::InvalidEvents);
        }
        self.process_context(
            Pcm::F32(data),
            events,
            automation,
            frame.core_transport(),
            Some(frame),
        )
    }
    fn process_context(
        &mut self,
        mut data: Pcm<'_>,
        events: &[Event],
        automation: &[ParameterEvent],
        transport: Transport,
        musical: Option<soniclaw_transport::Frame>,
    ) -> Result<i32, Error> {
        let channels = self.spec.channels;
        if !data.len().is_multiple_of(channels)
            || data.len() == 0
            || data.len() / channels > self.spec.max_block_frames
        {
            return Err(Error::InvalidBuffer);
        }
        let frames = data.len() / channels;
        validate_events(events, frames)?;
        let n = events
            .iter()
            .map(|e| {
                if matches!(e.message, MidiMessage::AllNotesOff)
                    && self.note_dialect != CLAP_NOTE_DIALECT_CLAP
                {
                    16
                } else {
                    1
                }
            })
            .sum::<usize>();
        if n + automation.len() > LIMIT
            || !transport.tempo.is_finite()
            || !(20.0..=400.0).contains(&transport.tempo)
            || (!events.is_empty() && self.note_dialect == 0)
        {
            return Err(Error::InvalidEvents);
        }
        let mut last = 0;
        for a in automation {
            let Some(p) = self.parameters.iter().find(|p| p.id == a.id) else {
                return Err(Error::InvalidParameter);
            };
            if a.frame >= frames
                || a.frame < last
                || !a.value.is_finite()
                || !(p.min..=p.max).contains(&a.value)
                || p.flags & CLAP_PARAM_IS_READONLY != 0
            {
                return Err(Error::InvalidParameter);
            }
            last = a.frame;
        }
        let r = &mut self.resources;
        r.events.clear();
        r.pointers.clear();
        r.out.clear();
        for e in events {
            if matches!(e.message, MidiMessage::AllNotesOff)
                && self.note_dialect != CLAP_NOTE_DIALECT_CLAP
            {
                for channel in 0..16 {
                    r.events.push(native(
                        &Event {
                            frame: e.frame,
                            message: MidiMessage::ControlChange {
                                channel,
                                controller: 123,
                                value: 0,
                            },
                        },
                        self.note_dialect,
                    )?);
                }
            } else {
                r.events.push(native(e, self.note_dialect)?);
            }
        }
        for a in automation {
            r.events.push(NativeEvent {
                param: clap_event_param_value {
                    header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, a.frame),
                    param_id: a.id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value: a.value,
                },
            });
        }
        for e in &r.events {
            r.pointers.push((e as *const NativeEvent).cast())
        }
        for i in 1..r.pointers.len() {
            let mut j = i;
            while j > 0 && unsafe { (*r.pointers[j - 1]).time > (*r.pointers[j]).time } {
                r.pointers.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut ip32;
        let mut op32;
        let mut ip64;
        let mut op64;
        let mut input = clap_audio_buffer {
            data32: ptr::null_mut(),
            data64: ptr::null_mut(),
            channel_count: self.input_channels as u32,
            latency: 0,
            constant_mask: 0,
        };
        let mut output = clap_audio_buffer {
            channel_count: channels as u32,
            ..input
        };
        match &data {
            Pcm::F32(data) => {
                let pcm = &mut r.pcm;
                pcm.prepare(data, channels, self.input_channels, |a, b| (a + b) * 0.5);
                ip32 = pcm.input.each_mut().map(|p| p.as_mut_ptr());
                op32 = pcm.output.each_mut().map(|p| p.as_mut_ptr());
                input.data32 = ip32.as_mut_ptr();
                output.data32 = op32.as_mut_ptr();
            }
            Pcm::F64(data) => {
                let pcm = r.wide.as_mut().ok_or(Error::InvalidSpec)?;
                pcm.prepare(data, channels, self.input_channels, |a, b| {
                    a * 0.5 + b * 0.5
                });
                ip64 = pcm.input.each_mut().map(|p| p.as_mut_ptr());
                op64 = pcm.output.each_mut().map(|p| p.as_mut_ptr());
                input.data64 = ip64.as_mut_ptr();
                output.data64 = op64.as_mut_ptr();
            }
        }
        let ie = clap_input_events {
            ctx: (&r.pointers as *const Vec<*const clap_event_header>)
                .cast_mut()
                .cast(),
            size: Some(event_size),
            get: Some(event_get),
        };
        let mut out = OutputContext {
            events: &mut r.out,
            frames,
            dropped: &mut r.dropped,
        };
        let oe = clap_output_events {
            ctx: (&mut out as *mut OutputContext<'_>).cast(),
            try_push: Some(event_push),
        };
        let seconds = transport.sample_position as f64 / self.spec.sample_rate;
        let mut t = clap_event_transport {
            header: header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, 0),
            flags: CLAP_TRANSPORT_HAS_TEMPO
                | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
                | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
                | if transport.playing {
                    CLAP_TRANSPORT_IS_PLAYING
                } else {
                    0
                },
            song_pos_beats: (seconds * transport.tempo / 60.0 * CLAP_BEATTIME_FACTOR as f64)
                .min(i64::MAX as f64) as i64,
            song_pos_seconds: (seconds * CLAP_SECTIME_FACTOR as f64).min(i64::MAX as f64) as i64,
            tempo: transport.tempo,
            tempo_inc: 0.0,
            loop_start_beats: 0,
            loop_end_beats: 0,
            loop_start_seconds: 0,
            loop_end_seconds: 0,
            bar_start: 0,
            bar_number: 0,
            tsig_num: 4,
            tsig_denom: 4,
        };
        if let Some(frame) = musical {
            let p = frame.position;
            t.flags |= CLAP_TRANSPORT_HAS_TIME_SIGNATURE;
            t.song_pos_beats = (p.beat * CLAP_BEATTIME_FACTOR as f64) as i64;
            t.bar_start = ((p.beat - p.beat_in_bar) * CLAP_BEATTIME_FACTOR as f64) as i64;
            t.bar_number = p.bar as i32;
            t.tsig_num = p.numerator;
            t.tsig_denom = p.denominator;
            if let (Some([a, b]), Some([x, y])) = (frame.loop_range, frame.loop_seconds) {
                t.flags |= CLAP_TRANSPORT_IS_LOOP_ACTIVE;
                t.loop_start_beats = (a * CLAP_BEATTIME_FACTOR as f64) as i64;
                t.loop_end_beats = (b * CLAP_BEATTIME_FACTOR as f64) as i64;
                t.loop_start_seconds = (x * CLAP_SECTIME_FACTOR as f64) as i64;
                t.loop_end_seconds = (y * CLAP_SECTIME_FACTOR as f64) as i64;
            }
        }
        let c = clap_process {
            steady_time: self.steady,
            frames_count: frames as u32,
            transport: &t,
            audio_inputs: if self.input_channels == 0 {
                ptr::null()
            } else {
                &input
            },
            audio_outputs: &mut output,
            audio_inputs_count: if self.input_channels == 0 { 0 } else { 1 },
            audio_outputs_count: 1,
            in_events: &ie,
            out_events: &oe,
        };
        let status = unsafe { (*self.plugin).process.expect("checked")(self.plugin, &c) };
        self.steady = self.steady.saturating_add(frames as i64);
        if ![
            CLAP_PROCESS_CONTINUE,
            CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
            CLAP_PROCESS_TAIL,
            CLAP_PROCESS_SLEEP,
        ]
        .contains(&status)
        {
            data.clear();
            return Err(Error::InvalidBuffer);
        }
        match data {
            Pcm::F32(data) => r
                .pcm
                .read(data, channels, output.constant_mask, f32::is_finite),
            Pcm::F64(data) => r.wide.as_ref().expect("prepared").read(
                data,
                channels,
                output.constant_mask,
                f64::is_finite,
            ),
        }
        Ok(status)
    }
}
impl Drop for HostedAudio<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
impl AudioProcessor for HostedAudio<'_> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        if s.sample_rate != self.spec.sample_rate
            || s.channels != self.spec.channels
            || s.max_block_frames > self.spec.max_block_frames
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        if self
            .process_block(b.samples_mut(), c.events, &[], c.transport)
            .is_err()
        {
            b.clear();
        }
    }
    fn reset(&mut self) {
        unsafe {
            if let Some(f) = (*self.plugin).reset {
                f(self.plugin)
            }
        }
    }
}
fn header<T>(kind: u16, frame: usize) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time: frame as u32,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags: 0,
    }
}
unsafe extern "C" fn event_size(l: *const clap_input_events) -> u32 {
    unsafe { (&*((*l).ctx.cast::<Vec<*const clap_event_header>>())).len() as u32 }
}
unsafe extern "C" fn event_get(l: *const clap_input_events, i: u32) -> *const clap_event_header {
    unsafe {
        (&*((*l).ctx.cast::<Vec<*const clap_event_header>>()))
            .get(i as usize)
            .copied()
            .unwrap_or(ptr::null())
    }
}
fn native(e: &Event, dialect: u32) -> Result<NativeEvent, Error> {
    let time = e.frame;
    if dialect == CLAP_NOTE_DIALECT_MIDI {
        let data = match e.message {
            MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            } => Some([0x90 | channel, note, velocity]),
            MidiMessage::NoteOnId {
                channel,
                note,
                velocity,
                ..
            } => Some([0x90 | channel, note, (velocity * 127.0).round() as u8]),
            MidiMessage::NoteOff {
                channel,
                note,
                velocity,
            } => Some([0x80 | channel, note, velocity]),
            MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => {
                if target.channel < 0 || target.key < 0 {
                    return Err(Error::InvalidEvents);
                }
                Some([0x80 | target.channel as u8, target.key as u8, 0])
            }
            MidiMessage::NoteExpression {
                target,
                expression: NoteExpression::Pressure,
                value,
            } => {
                if target.channel < 0 || target.key < 0 {
                    return Err(Error::InvalidEvents);
                }
                Some([
                    0xa0 | target.channel as u8,
                    target.key as u8,
                    (value * 127.0).round() as u8,
                ])
            }
            MidiMessage::NoteExpression { .. } => return Err(Error::InvalidEvents),
            _ => None,
        };
        if let Some(data) = data {
            return Ok(NativeEvent {
                midi: clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
                    port_index: 0,
                    data,
                },
            });
        }
    }
    if matches!(e.message, MidiMessage::AllNotesOff) {
        return Ok(NativeEvent {
            note: clap_event_note {
                header: header::<clap_event_note>(CLAP_EVENT_NOTE_CHOKE, time),
                note_id: -1,
                port_index: 0,
                channel: -1,
                key: -1,
                velocity: 0.0,
            },
        });
    }
    let mut kind = CLAP_EVENT_NOTE_ON;
    let (channel, key, id, velocity) = match e.message {
        MidiMessage::NoteOnId {
            channel,
            note,
            velocity,
            note_id,
        } => (channel as i16, note as i16, note_id, velocity as f64),
        MidiMessage::NoteOn {
            channel,
            note,
            velocity,
        } => (channel as i16, note as i16, -1, velocity as f64 / 127.0),
        MidiMessage::NoteOff {
            channel,
            note,
            velocity,
        } => {
            kind = CLAP_EVENT_NOTE_OFF;
            (channel as i16, note as i16, -1, velocity as f64 / 127.0)
        }
        MidiMessage::NoteOffId { target } | MidiMessage::NoteChoke { target } => {
            kind = if matches!(e.message, MidiMessage::NoteChoke { .. }) {
                CLAP_EVENT_NOTE_CHOKE
            } else {
                CLAP_EVENT_NOTE_OFF
            };
            (target.channel, target.key, target.note_id, 0.0)
        }
        MidiMessage::NoteExpression {
            target,
            expression,
            value,
        } => {
            return Ok(NativeEvent {
                expression: clap_event_note_expression {
                    header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, time),
                    expression_id: match expression {
                        NoteExpression::Tuning => CLAP_NOTE_EXPRESSION_TUNING,
                        NoteExpression::Pressure => CLAP_NOTE_EXPRESSION_PRESSURE,
                        NoteExpression::Timbre => CLAP_NOTE_EXPRESSION_BRIGHTNESS,
                        NoteExpression::Pan => CLAP_NOTE_EXPRESSION_PAN,
                        NoteExpression::Volume => CLAP_NOTE_EXPRESSION_VOLUME,
                    },
                    note_id: target.note_id,
                    port_index: 0,
                    channel: target.channel,
                    key: target.key,
                    value,
                },
            })
        }
        other => {
            let data = match other {
                MidiMessage::ControlChange {
                    channel,
                    controller,
                    value,
                } => [0xb0 | channel, controller, value],
                MidiMessage::PitchBend { channel, value } => {
                    [0xe0 | channel, (value & 127) as u8, (value >> 7) as u8]
                }
                MidiMessage::ChannelPressure { channel, value } => [0xd0 | channel, value, 0],
                MidiMessage::PolyPressure {
                    channel,
                    note,
                    value,
                } => [0xa0 | channel, note, value],
                _ => return Err(Error::InvalidEvents),
            };
            return Ok(NativeEvent {
                midi: clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
                    port_index: 0,
                    data,
                },
            });
        }
    };
    if kind == CLAP_EVENT_NOTE_ON && velocity == 0.0 {
        kind = CLAP_EVENT_NOTE_OFF;
    }
    Ok(NativeEvent {
        note: clap_event_note {
            header: header::<clap_event_note>(kind, time),
            note_id: id,
            port_index: 0,
            channel,
            key,
            velocity,
        },
    })
}
struct OutputContext<'a> {
    events: &'a mut Vec<OutputEvent>,
    frames: usize,
    dropped: &'a mut u64,
}
unsafe extern "C" fn event_push(l: *const clap_output_events, h: *const clap_event_header) -> bool {
    unsafe {
        let c = &mut *((*l).ctx.cast::<OutputContext<'_>>());
        if h.is_null()
            || (*h).size < std::mem::size_of::<clap_event_header>() as u32
            || (*h).space_id != CLAP_CORE_EVENT_SPACE_ID
            || (*h).time as usize >= c.frames
            || c.events.len() == LIMIT
        {
            *c.dropped += 1;
            return false;
        }
        let frame = (*h).time as usize;
        let e = match (*h).type_ {
            CLAP_EVENT_PARAM_VALUE
                if (*h).size as usize >= std::mem::size_of::<clap_event_param_value>() =>
            {
                let e = &*h.cast::<clap_event_param_value>();
                if !e.value.is_finite() {
                    return false;
                }
                OutputEvent::Parameter(ParameterEvent {
                    frame,
                    id: e.param_id,
                    value: e.value,
                })
            }
            CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END
                if (*h).size as usize >= std::mem::size_of::<clap_event_param_gesture>() =>
            {
                let e = &*h.cast::<clap_event_param_gesture>();
                OutputEvent::Gesture {
                    frame,
                    id: e.param_id,
                    begin: (*h).type_ == CLAP_EVENT_PARAM_GESTURE_BEGIN,
                }
            }
            CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE
                if (*h).size as usize >= std::mem::size_of::<clap_event_note>() =>
            {
                let e = &*h.cast::<clap_event_note>();
                if e.port_index != 0 {
                    return false;
                }
                let target = NoteTarget {
                    note_id: e.note_id,
                    channel: e.channel,
                    key: e.key,
                };
                let message = if (*h).type_ == CLAP_EVENT_NOTE_ON {
                    MidiMessage::NoteOnId {
                        channel: e.channel as u8,
                        note: e.key as u8,
                        velocity: e.velocity as f32,
                        note_id: e.note_id,
                    }
                } else if (*h).type_ == CLAP_EVENT_NOTE_OFF {
                    MidiMessage::NoteOffId { target }
                } else {
                    MidiMessage::NoteChoke { target }
                };
                let e = Event { frame, message };
                if validate_events(&[e], c.frames).is_err() {
                    return false;
                }
                OutputEvent::Midi(e)
            }
            CLAP_EVENT_MIDI if (*h).size as usize >= std::mem::size_of::<clap_event_midi>() => {
                let e = &*h.cast::<clap_event_midi>();
                if e.port_index != 0 {
                    return false;
                }
                let [status, a, b] = e.data;
                let channel = status & 15;
                let message = match status >> 4 {
                    8 => MidiMessage::NoteOff {
                        channel,
                        note: a,
                        velocity: b,
                    },
                    9 => MidiMessage::NoteOn {
                        channel,
                        note: a,
                        velocity: b,
                    },
                    10 => MidiMessage::PolyPressure {
                        channel,
                        note: a,
                        value: b,
                    },
                    11 => MidiMessage::ControlChange {
                        channel,
                        controller: a,
                        value: b,
                    },
                    13 => MidiMessage::ChannelPressure { channel, value: a },
                    14 => MidiMessage::PitchBend {
                        channel,
                        value: a as u16 | ((b as u16) << 7),
                    },
                    _ => return false,
                };
                let e = Event { frame, message };
                if validate_events(&[e], c.frames).is_err() {
                    return false;
                }
                OutputEvent::Midi(e)
            }
            CLAP_EVENT_NOTE_EXPRESSION
                if (*h).size as usize >= std::mem::size_of::<clap_event_note_expression>() =>
            {
                let e = &*h.cast::<clap_event_note_expression>();
                if e.port_index != 0 {
                    return false;
                }
                let expression = match e.expression_id {
                    CLAP_NOTE_EXPRESSION_TUNING => NoteExpression::Tuning,
                    CLAP_NOTE_EXPRESSION_PRESSURE => NoteExpression::Pressure,
                    CLAP_NOTE_EXPRESSION_BRIGHTNESS => NoteExpression::Timbre,
                    CLAP_NOTE_EXPRESSION_PAN => NoteExpression::Pan,
                    CLAP_NOTE_EXPRESSION_VOLUME => NoteExpression::Volume,
                    _ => return false,
                };
                let e = Event {
                    frame,
                    message: MidiMessage::NoteExpression {
                        target: NoteTarget {
                            note_id: e.note_id,
                            channel: e.channel,
                            key: e.key,
                        },
                        expression,
                        value: e.value,
                    },
                };
                if validate_events(&[e], c.frames).is_err() {
                    return false;
                }
                OutputEvent::Midi(e)
            }
            _ => return false,
        };
        c.events.push(e);
        true
    }
}

pub mod formats;
#[cfg(feature = "studio")]
pub mod studio;

pub mod offline;
