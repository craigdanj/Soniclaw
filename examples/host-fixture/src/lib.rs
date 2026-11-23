//! Independent MIDI-only/timed-gain CLAP fixture enforcing thread/lifecycle calls.
use clap_sys::{
    entry::*,
    events::*,
    ext::{
        audio_ports::*, audio_ports_config::*, gui::*, latency::*, note_ports::*, params::*,
        posix_fd_support::*, state::*, thread_check::*, timer_support::*,
    },
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use std::{
    ffi::{c_char, c_void, CStr},
    ptr,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
struct State {
    plugin: clap_plugin,
    host: *const clap_host,
    owner: std::thread::ThreadId,
    configuration: AtomicU32,
    requested: AtomicBool,
    delay: std::cell::UnsafeCell<([f32; 32], usize)>,
    gain: AtomicU64,
    callbacks: AtomicU32,
    active: AtomicBool,
    processing: AtomicBool,
    max: AtomicU32,
    gui_created: AtomicBool,
    gui_visible: AtomicBool,
    timer_id: AtomicU32,
    #[cfg(unix)]
    fd_read: std::sync::atomic::AtomicI32,
    #[cfg(unix)]
    fd_write: std::sync::atomic::AtomicI32,
    gui_size: std::sync::atomic::AtomicU64,
}
unsafe fn state<'a>(p: *const clap_plugin) -> &'a State {
    unsafe { &*((*p).plugin_data.cast::<State>()) }
}
unsafe fn thread(s: &State, audio: bool) -> bool {
    unsafe {
        let ext = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_THREAD_CHECK.as_ptr())
            .cast::<clap_host_thread_check>();
        if ext.is_null() {
            return false;
        }
        let t = &*ext;
        if audio {
            std::thread::current().id() != s.owner
                && t.is_audio_thread.unwrap()(s.host)
                && !t.is_main_thread.unwrap()(s.host)
        } else {
            std::thread::current().id() == s.owner
                && t.is_main_thread.unwrap()(s.host)
                && !t.is_audio_thread.unwrap()(s.host)
        }
    }
}
unsafe extern "C" fn init(p: *const clap_plugin) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) {
            return false;
        }
        let timer = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_TIMER_SUPPORT.as_ptr())
            .cast::<clap_host_timer_support>();
        if !timer.is_null() {
            let mut id = u32::MAX;
            if (*timer).register_timer.unwrap()(s.host, 1, &mut id) {
                s.timer_id.store(id, Ordering::Relaxed);
            }
        }
        #[cfg(unix)]
        {
            let ext = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_POSIX_FD_SUPPORT.as_ptr())
                .cast::<clap_host_posix_fd_support>();
            if !ext.is_null() {
                let mut fds = [-1; 2];
                if libc::pipe(fds.as_mut_ptr()) == 0 {
                    libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK);
                    if (*ext).register_fd.unwrap()(s.host, fds[0], CLAP_POSIX_FD_READ) {
                        s.fd_read.store(fds[0], Ordering::Relaxed);
                        s.fd_write.store(fds[1], Ordering::Relaxed);
                        let byte = 1u8;
                        libc::write(fds[1], (&byte as *const u8).cast(), 1);
                    } else {
                        libc::close(fds[0]);
                        libc::close(fds[1]);
                    }
                }
            }
        }
        true
    }
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    unsafe {
        let s = state(p);
        if !thread(s, false)
            || s.gui_created.load(Ordering::Relaxed)
            || s.active.load(Ordering::Relaxed)
            || s.processing.load(Ordering::Relaxed)
        {
            std::process::abort()
        }
        let timer = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_TIMER_SUPPORT.as_ptr())
            .cast::<clap_host_timer_support>();
        let id = s.timer_id.load(Ordering::Relaxed);
        if !timer.is_null() && id != u32::MAX {
            (*timer).unregister_timer.unwrap()(s.host, id);
        }
        #[cfg(unix)]
        {
            let fd = s.fd_read.load(Ordering::Relaxed);
            if fd >= 0 {
                let ext =
                    (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_POSIX_FD_SUPPORT.as_ptr())
                        .cast::<clap_host_posix_fd_support>();
                if !ext.is_null() {
                    (*ext).unregister_fd.unwrap()(s.host, fd);
                }
                libc::close(fd);
                libc::close(s.fd_write.load(Ordering::Relaxed));
            }
        }
        drop(Box::from_raw((*p).plugin_data.cast::<State>()));
    }
}
unsafe extern "C" fn activate(p: *const clap_plugin, _: f64, _: u32, max: u32) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || s.active.load(Ordering::Relaxed) {
            return false;
        }
        s.active.store(true, Ordering::Relaxed);
        s.max.store(max, Ordering::Relaxed);
        *s.delay.get() = ([0.; 32], 0);
        true
    }
}
unsafe extern "C" fn deactivate(p: *const clap_plugin) {
    unsafe {
        let s = state(p);
        if !thread(s, false) || s.processing.load(Ordering::Relaxed) {
            std::process::abort()
        }
        s.active.store(false, Ordering::Relaxed);
    }
}
unsafe extern "C" fn start(p: *const clap_plugin) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, true)
            || !s.active.load(Ordering::Relaxed)
            || s.processing.load(Ordering::Relaxed)
        {
            return false;
        }
        s.processing.store(true, Ordering::Relaxed);
        true
    }
}
unsafe extern "C" fn stop(p: *const clap_plugin) {
    unsafe {
        let s = state(p);
        if !thread(s, true) {
            std::process::abort()
        }
        s.processing.store(false, Ordering::Relaxed);
    }
}
unsafe extern "C" fn reset(p: *const clap_plugin) {
    unsafe {
        if !thread(state(p), true) {
            std::process::abort()
        }
    }
}
unsafe extern "C" fn on_main(p: *const clap_plugin) {
    unsafe {
        let s = state(p);
        if !thread(s, false) {
            std::process::abort()
        }
        s.callbacks.fetch_add(1, Ordering::Relaxed);
    }
}
unsafe extern "C" fn process(p: *const clap_plugin, c: *const clap_process) -> i32 {
    unsafe {
        let s = state(p);
        if !thread(s, true) || !s.processing.load(Ordering::Relaxed) || c.is_null() {
            return CLAP_PROCESS_ERROR;
        }
        if let Ok(fault) = std::env::var("SONICLAW_FIXTURE_PROCESS_FAULT") {
            if fault == "crash" {
                std::process::abort();
            }
            if fault == "hang" {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
        let c = &*c;
        if c.frames_count > s.max.load(Ordering::Relaxed)
            || c.audio_inputs_count != if sidechain(s) { 2 } else { 1 }
            || c.audio_outputs_count != 1
        {
            return CLAP_PROCESS_ERROR;
        }
        let input = &*c.audio_inputs;
        let output = &mut *c.audio_outputs;
        if input.channel_count != 2 || output.channel_count != 2 {
            return CLAP_PROCESS_ERROR;
        }
        output.constant_mask = 0;
        let list = &*c.in_events;
        let n = list.size.unwrap()(list);
        let mut e = 0;
        for frame in 0..c.frames_count {
            while e < n {
                let h = list.get.unwrap()(list, e);
                if (*h).time > frame {
                    break;
                }
                if (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                    let v = &*h.cast::<clap_event_param_value>();
                    if v.param_id != 9 {
                        return CLAP_PROCESS_ERROR;
                    }
                    s.gain.store((v.value).to_bits(), Ordering::Relaxed);
                } else if (*h).type_ != CLAP_EVENT_MIDI {
                    return CLAP_PROCESS_ERROR;
                }
                if !c.out_events.is_null() && !(*c.out_events).try_push.unwrap()(c.out_events, h) {
                    return CLAP_PROCESS_ERROR;
                }
                e += 1;
            }
            for channel in 0..2 {
                let value = *(*input.data32.add(channel)).add(frame as usize)
                    * f64::from_bits(s.gain.load(Ordering::Relaxed)) as f32
                    * if c.audio_inputs_count == 2 {
                        1. / (1. + (*(*(*c.audio_inputs.add(1)).data32).add(frame as usize)).abs())
                    } else {
                        1.
                    };
                let value = if s.configuration.load(Ordering::Relaxed) == 1 {
                    let (data, cursor) = &mut *s.delay.get();
                    let old = data[*cursor + channel];
                    data[*cursor + channel] = value;
                    old
                } else {
                    value
                };
                *(*output.data32.add(channel)).add(frame as usize) = value;
            }
            if s.configuration.load(Ordering::Relaxed) == 1 {
                let (_, cursor) = &mut *s.delay.get();
                *cursor = (*cursor + 2) % 32;
            }
        }
        if std::env::var_os("SONICLAW_FIXTURE_REQUEST_RECONFIGURE").is_some()
            && !s.requested.swap(true, Ordering::Relaxed)
        {
            (*s.host).request_restart.unwrap()(s.host);
            let ext = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_AUDIO_PORTS.as_ptr())
                .cast::<clap_host_audio_ports>();
            if !ext.is_null() {
                (*ext).rescan.unwrap()(s.host, CLAP_AUDIO_PORTS_RESCAN_LIST);
            }
        }
        if s.callbacks.load(Ordering::Relaxed) == 0 {
            (*s.host).request_callback.unwrap()(s.host);
        }
        CLAP_PROCESS_CONTINUE
    }
}
fn sidechain(s: &State) -> bool {
    s.configuration.load(Ordering::Relaxed) == 1
        || std::env::var_os("SONICLAW_FIXTURE_MULTIBUS").is_some()
}
unsafe extern "C" fn audio_count(p: *const clap_plugin, input: bool) -> u32 {
    if input && sidechain(unsafe { state(p) }) {
        2
    } else {
        1
    }
}
unsafe extern "C" fn audio_get(
    p: *const clap_plugin,
    index: u32,
    input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index >= unsafe { audio_count(p, input) } {
        return false;
    }
    unsafe {
        *info = clap_audio_port_info {
            id: index,
            name: [0; 256],
            flags: if index == 0 {
                CLAP_AUDIO_PORT_IS_MAIN
            } else {
                0
            },
            channel_count: if index == 0 { 2 } else { 1 },
            port_type: CLAP_PORT_STEREO.as_ptr(),
            in_place_pair: 0,
        };
    }
    true
}
unsafe extern "C" fn note_count(_: *const clap_plugin, _: bool) -> u32 {
    1
}
unsafe extern "C" fn note_get(
    _: *const clap_plugin,
    index: u32,
    _: bool,
    info: *mut clap_note_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    unsafe {
        *info = clap_note_port_info {
            id: 0,
            supported_dialects: CLAP_NOTE_DIALECT_MIDI,
            preferred_dialect: CLAP_NOTE_DIALECT_MIDI,
            name: [0; 256],
        };
    }
    true
}
unsafe extern "C" fn param_count(_: *const clap_plugin) -> u32 {
    1
}
unsafe extern "C" fn param_info(
    p: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    unsafe {
        if !thread(state(p), false) || index != 0 {
            return false;
        }
        *info = std::mem::zeroed();
        (*info).id = 9;
        (*info).flags = CLAP_PARAM_IS_AUTOMATABLE;
        (*info).min_value = 0.0;
        (*info).max_value = 1.0;
        (*info).default_value = 0.5;
        for (i, b) in b"Gain\0".iter().enumerate() {
            (*info).name[i] = *b as c_char;
        }
        true
    }
}
unsafe extern "C" fn param_get(p: *const clap_plugin, id: u32, v: *mut f64) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || id != 9 {
            return false;
        }
        *v = f64::from_bits(s.gain.load(Ordering::Relaxed));
        true
    }
}
unsafe extern "C" fn param_flush(
    p: *const clap_plugin,
    input: *const clap_input_events,
    out: *const clap_output_events,
) {
    unsafe {
        let s = state(p);
        if !thread(s, s.active.load(Ordering::Relaxed)) {
            std::process::abort();
        }
        if !input.is_null() {
            for i in 0..(*input).size.unwrap()(input) {
                let h = (*input).get.unwrap()(input, i);
                if !h.is_null() && (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                    let v = &*h.cast::<clap_event_param_value>();
                    if v.param_id == 9 && v.value.is_finite() && (0.0..=1.0).contains(&v.value) {
                        s.gain.store(v.value.to_bits(), Ordering::Relaxed);
                    }
                }
            }
        }
        let e = clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: 9,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: f64::from_bits(s.gain.load(Ordering::Relaxed)),
        };
        if !out.is_null() {
            (*out).try_push.unwrap()(out, &e.header);
        }
    }
}
unsafe extern "C" fn save(p: *const clap_plugin, out: *const clap_ostream) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) {
            return false;
        }
        let mut b = [0_u8; 16];
        b[..4].copy_from_slice(b"SLHF");
        b[4..12].copy_from_slice(&f64::from_bits(s.gain.load(Ordering::Relaxed)).to_le_bytes());
        b[12..].copy_from_slice(&s.callbacks.load(Ordering::Relaxed).to_le_bytes());
        let mut at = 0;
        while at < b.len() {
            let n = (*out).write.unwrap()(out, b[at..].as_ptr().cast(), (b.len() - at) as u64);
            if n <= 0 {
                return false;
            }
            at += n as usize;
        }
        true
    }
}
unsafe extern "C" fn load(p: *const clap_plugin, input: *const clap_istream) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) {
            return false;
        }
        let mut b = [0_u8; 16];
        let mut at = 0;
        while at < b.len() {
            let n =
                (*input).read.unwrap()(input, b[at..].as_mut_ptr().cast(), (b.len() - at) as u64);
            if n <= 0 {
                return false;
            }
            at += n as usize;
        }
        if &b[..4] != b"SLHF" {
            return false;
        }
        let gain = f64::from_le_bytes(b[4..12].try_into().unwrap());
        if !gain.is_finite() || !(0.0..=1.0).contains(&gain) {
            return false;
        }
        s.gain.store((gain).to_bits(), Ordering::Relaxed);
        s.callbacks.store(
            u32::from_le_bytes(b[12..].try_into().unwrap()),
            Ordering::Relaxed,
        );
        true
    }
}

unsafe extern "C" fn config_count(_: *const clap_plugin) -> u32 {
    2
}
unsafe extern "C" fn config_get(
    p: *const clap_plugin,
    index: u32,
    out: *mut clap_audio_ports_config,
) -> bool {
    unsafe {
        if index > 1 || out.is_null() || !thread(state(p), false) {
            return false;
        }
        *out = std::mem::zeroed();
        (*out).id = index;
        (*out).input_port_count = if index == 1 { 2 } else { 1 };
        (*out).output_port_count = 1;
        (*out).has_main_input = true;
        (*out).has_main_output = true;
        (*out).main_input_channel_count = 2;
        (*out).main_output_channel_count = 2;
        (*out).main_input_port_type = CLAP_PORT_STEREO.as_ptr();
        (*out).main_output_port_type = CLAP_PORT_STEREO.as_ptr();
        true
    }
}
unsafe extern "C" fn config_select(p: *const clap_plugin, id: u32) -> bool {
    unsafe {
        let s = state(p);
        if id > 1 || !thread(s, false) || s.active.load(Ordering::Relaxed) {
            return false;
        }
        s.configuration.store(id, Ordering::Relaxed);
        true
    }
}
unsafe extern "C" fn latency_get(p: *const clap_plugin) -> u32 {
    unsafe {
        if state(p).configuration.load(Ordering::Relaxed) == 1 {
            16
        } else {
            0
        }
    }
}
static CONFIGS: clap_plugin_audio_ports_config = clap_plugin_audio_ports_config {
    count: Some(config_count),
    get: Some(config_get),
    select: Some(config_select),
};
static LATENCY: clap_plugin_latency = clap_plugin_latency {
    get: Some(latency_get),
};
static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(audio_count),
    get: Some(audio_get),
};
static NOTES: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_count),
    get: Some(note_get),
};
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(param_count),
    get_info: Some(param_info),
    get_value: Some(param_get),
    value_to_text: None,
    text_to_value: None,
    flush: Some(param_flush),
};
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(save),
    load: Some(load),
};
// ABI-only floating editor: verifies lifecycle/thread calls without creating an OS window.
unsafe extern "C" fn gui_supported(
    p: *const clap_plugin,
    _: *const c_char,
    _floating: bool,
) -> bool {
    unsafe { thread(state(p), false) }
}
unsafe extern "C" fn gui_create(p: *const clap_plugin, api: *const c_char, floating: bool) -> bool {
    unsafe {
        gui_supported(p, api, floating) && !state(p).gui_created.swap(true, Ordering::Relaxed)
    }
}
unsafe extern "C" fn gui_destroy(p: *const clap_plugin) {
    unsafe {
        let s = state(p);
        if !thread(s, false) || !s.gui_created.swap(false, Ordering::Relaxed) {
            std::process::abort();
        }
        s.gui_visible.store(false, Ordering::Relaxed);
    }
}
unsafe extern "C" fn gui_show(p: *const clap_plugin) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || !s.gui_created.load(Ordering::Relaxed) {
            return false;
        }
        s.gui_visible.store(true, Ordering::Relaxed);
        let params = (*s.host).get_extension.unwrap()(s.host, CLAP_EXT_PARAMS.as_ptr())
            .cast::<clap_host_params>();
        if !params.is_null() {
            if let Some(flush) = (*params).request_flush {
                flush(s.host);
            }
        }
        true
    }
}
unsafe extern "C" fn gui_hide(p: *const clap_plugin) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || !s.gui_created.load(Ordering::Relaxed) {
            return false;
        }
        s.gui_visible.store(false, Ordering::Relaxed);
        true
    }
}
unsafe extern "C" fn gui_size(p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || w.is_null() || h.is_null() {
            return false;
        }
        let size = s.gui_size.load(Ordering::Relaxed);
        *w = (size >> 32) as u32;
        *h = size as u32;
        true
    }
}
unsafe extern "C" fn gui_set_size(p: *const clap_plugin, w: u32, h: u32) -> bool {
    unsafe {
        let s = state(p);
        if !thread(s, false) || !(64..=8192).contains(&w) || !(64..=8192).contains(&h) {
            return false;
        }
        s.gui_size
            .store(((w as u64) << 32) | h as u64, Ordering::Relaxed);
        true
    }
}
unsafe extern "C" fn gui_resize(p: *const clap_plugin) -> bool {
    unsafe { thread(state(p), false) }
}
unsafe extern "C" fn gui_scale(p: *const clap_plugin, v: f64) -> bool {
    unsafe { thread(state(p), false) && v.is_finite() && (0.5..=4.0).contains(&v) }
}
unsafe extern "C" fn gui_parent(p: *const clap_plugin, w: *const clap_window) -> bool {
    unsafe { thread(state(p), false) && !w.is_null() }
}
unsafe extern "C" fn on_timer(p: *const clap_plugin, _: u32) {
    unsafe {
        if !thread(state(p), false) {
            std::process::abort();
        }
    }
}
unsafe extern "C" fn on_fd(p: *const clap_plugin, fd: i32, flags: u32) {
    unsafe {
        if !thread(state(p), false) {
            std::process::abort();
        }
        #[cfg(unix)]
        if flags & CLAP_POSIX_FD_READ != 0 {
            let mut byte = 0u8;
            libc::read(fd, (&mut byte as *mut u8).cast(), 1);
        }
        #[cfg(not(unix))]
        let _ = (fd, flags);
    }
}
static TIMER: clap_plugin_timer_support = clap_plugin_timer_support {
    on_timer: Some(on_timer),
};
static FDS: clap_plugin_posix_fd_support = clap_plugin_posix_fd_support { on_fd: Some(on_fd) };
static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(gui_supported),
    get_preferred_api: None,
    create: Some(gui_create),
    destroy: Some(gui_destroy),
    set_scale: Some(gui_scale),
    get_size: Some(gui_size),
    can_resize: Some(gui_resize),
    get_resize_hints: None,
    adjust_size: None,
    set_size: Some(gui_set_size),
    set_parent: Some(gui_parent),
    set_transient: None,
    suggest_title: None,
    show: Some(gui_show),
    hide: Some(gui_hide),
};
unsafe extern "C" fn extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    unsafe {
        let id = CStr::from_ptr(id);
        if id == CLAP_EXT_TIMER_SUPPORT {
            (&TIMER as *const clap_plugin_timer_support).cast()
        } else if id == CLAP_EXT_POSIX_FD_SUPPORT {
            (&FDS as *const clap_plugin_posix_fd_support).cast()
        } else if id == CLAP_EXT_GUI {
            (&GUI as *const clap_plugin_gui).cast()
        } else if id == CLAP_EXT_AUDIO_PORTS_CONFIG {
            (&CONFIGS as *const clap_plugin_audio_ports_config).cast()
        } else if id == CLAP_EXT_LATENCY {
            (&LATENCY as *const clap_plugin_latency).cast()
        } else if id == CLAP_EXT_AUDIO_PORTS {
            (&PORTS as *const clap_plugin_audio_ports).cast()
        } else if id == CLAP_EXT_NOTE_PORTS {
            (&NOTES as *const clap_plugin_note_ports).cast()
        } else if id == CLAP_EXT_PARAMS {
            (&PARAMS as *const clap_plugin_params).cast()
        } else if id == CLAP_EXT_STATE {
            (&STATE as *const clap_plugin_state).cast()
        } else {
            ptr::null()
        }
    }
}
static DESC: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: c"org.soniclaw.host-fixture".as_ptr(),
    name: c"Soniclaw MIDI-only Host Fixture".as_ptr(),
    vendor: c"Soniclaw".as_ptr(),
    url: c"".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.146.0".as_ptr(),
    description: c"Thread, lifecycle, echo and timing contract".as_ptr(),
    features: ptr::null(),
};
unsafe extern "C" fn count(_: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn descriptor(
    _: *const clap_plugin_factory,
    i: u32,
) -> *const clap_plugin_descriptor {
    if i == 0 {
        &DESC
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    unsafe {
        if CStr::from_ptr(id) != c"org.soniclaw.host-fixture" {
            return ptr::null();
        }
        let mut s = Box::new(State {
            plugin: clap_plugin {
                desc: &DESC,
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
                on_main_thread: Some(on_main),
            },
            host,
            owner: std::thread::current().id(),
            configuration: AtomicU32::new(0),
            requested: AtomicBool::new(false),
            delay: std::cell::UnsafeCell::new(([0.; 32], 0)),
            gain: AtomicU64::new(0.5f64.to_bits()),
            callbacks: AtomicU32::new(0),
            active: AtomicBool::new(false),
            processing: AtomicBool::new(false),
            max: AtomicU32::new(0),
            gui_created: AtomicBool::new(false),
            gui_visible: AtomicBool::new(false),
            timer_id: AtomicU32::new(u32::MAX),
            #[cfg(unix)]
            fd_read: std::sync::atomic::AtomicI32::new(-1),
            #[cfg(unix)]
            fd_write: std::sync::atomic::AtomicI32::new(-1),
            gui_size: std::sync::atomic::AtomicU64::new((320u64 << 32) | 180),
        });
        s.plugin.plugin_data = (&mut *s as *mut State).cast();
        let p = &s.plugin as *const clap_plugin;
        let _ = Box::into_raw(s);
        p
    }
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(count),
    get_plugin_descriptor: Some(descriptor),
    create_plugin: Some(create),
};
unsafe extern "C" fn entry_init(_: *const c_char) -> bool {
    true
}
unsafe extern "C" fn entry_deinit() {}
unsafe extern "C" fn factory(id: *const c_char) -> *const c_void {
    unsafe {
        if CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
            (&FACTORY as *const clap_plugin_factory).cast()
        } else {
            ptr::null()
        }
    }
}
#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(factory),
};
