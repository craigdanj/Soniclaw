//! Minimal independent CLAP host: loads the actual exported shared library.
#![deny(unsafe_op_in_unsafe_fn)]
use clap_sys::{
    audio_buffer::*,
    entry::*,
    events::*,
    ext::{audio_ports::*, latency::*, note_ports::*, params::*, render::*, state::*, tail::*},
    factory::plugin_factory::*,
    host::*,
    process::*,
    stream::*,
    version::*,
};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    ptr,
};
struct Events {
    pointers: Vec<*const clap_event_header>,
}
unsafe extern "C" fn event_size(list: *const clap_input_events) -> u32 {
    unsafe { (&*((*list).ctx.cast::<Events>())).pointers.len() as u32 }
}
unsafe extern "C" fn event_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    unsafe {
        (&*((*list).ctx.cast::<Events>()))
            .pointers
            .get(index as usize)
            .copied()
            .unwrap_or(ptr::null())
    }
}
unsafe extern "C" fn host_extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn request(_: *const clap_host) {}
struct Stream {
    bytes: Vec<u8>,
    position: usize,
}
unsafe extern "C" fn write(stream: *const clap_ostream, buffer: *const c_void, size: u64) -> i64 {
    unsafe {
        let stream = &mut *((*stream).ctx.cast::<Stream>());
        let count = (size as usize).min(7);
        stream
            .bytes
            .extend_from_slice(std::slice::from_raw_parts(buffer.cast::<u8>(), count));
        count as i64
    }
}
unsafe extern "C" fn read(stream: *const clap_istream, buffer: *mut c_void, size: u64) -> i64 {
    unsafe {
        let stream = &mut *((*stream).ctx.cast::<Stream>());
        let count = (size as usize)
            .min(5)
            .min(stream.bytes.len() - stream.position);
        ptr::copy_nonoverlapping(
            stream.bytes[stream.position..].as_ptr(),
            buffer.cast::<u8>(),
            count,
        );
        stream.position += count;
        count as i64
    }
}
fn header<T>(kind: u16, time: u32) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags: 0,
    }
}
fn value(id: u32, amount: f64, time: u32) -> clap_event_param_value {
    clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
        param_id: id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: amount,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let default = if cfg!(target_os = "windows") {
        "target/debug/soniclaw_clap_demo.dll"
    } else if cfg!(target_os = "macos") {
        "target/debug/libsoniclaw_clap_demo.dylib"
    } else {
        "target/debug/libsoniclaw_clap_demo.so"
    };
    let path = std::env::args().nth(1).unwrap_or_else(|| default.into());
    // SAFETY: The shared library stays loaded until all instances are destroyed.
    // Every input pointer targets live storage; callbacks are serialized, buffers
    // meet the negotiated layout, and all state/event storage outlives callbacks.
    unsafe {
        let library = libloading::Library::new(&path)?;
        let symbol: libloading::Symbol<*const clap_plugin_entry> = library.get(b"clap_entry\0")?;
        let entry = &**symbol;
        assert!(clap_version_is_compatible(entry.clap_version));
        let cpath = CString::new(path)?;
        assert!(entry.init.unwrap()(cpath.as_ptr()));
        let factory = &*entry.get_factory.unwrap()(CLAP_PLUGIN_FACTORY_ID.as_ptr())
            .cast::<clap_plugin_factory>();
        let count = factory.get_plugin_count.unwrap()(factory);
        assert_eq!(count, 2);
        let host = clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"Soniclaw independent smoke host".as_ptr(),
            vendor: c"Soniclaw tests".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.3.0".as_ptr(),
            get_extension: Some(host_extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(request),
        };
        for index in 0..count {
            let descriptor = factory.get_plugin_descriptor.unwrap()(factory, index);
            assert!(!descriptor.is_null());
            let name = CStr::from_ptr((*descriptor).name).to_string_lossy();
            let plugin = factory.create_plugin.unwrap()(factory, &host, (*descriptor).id);
            assert!(!plugin.is_null());
            let p = &*plugin;
            assert!(p.init.unwrap()(plugin));
            let ext = |id: &CStr| p.get_extension.unwrap()(plugin, id.as_ptr());
            assert!(ext(c"org.soniclaw.unsupported").is_null());
            let ports = &*ext(CLAP_EXT_AUDIO_PORTS).cast::<clap_plugin_audio_ports>();
            assert_eq!(ports.count.unwrap()(plugin, false), 1);
            assert_eq!(
                ports.count.unwrap()(plugin, true),
                if index == 0 { 1 } else { 0 }
            );
            let notes = &*ext(CLAP_EXT_NOTE_PORTS).cast::<clap_plugin_note_ports>();
            assert_eq!(
                notes.count.unwrap()(plugin, true),
                if index == 1 { 1 } else { 0 }
            );
            let params = &*ext(CLAP_EXT_PARAMS).cast::<clap_plugin_params>();
            assert_eq!(params.count.unwrap()(plugin), 1);
            let mut info = std::mem::zeroed::<clap_param_info>();
            assert!(params.get_info.unwrap()(plugin, 0, &mut info));
            let mut display = [0i8; 32];
            assert!(params.value_to_text.unwrap()(
                plugin,
                info.id,
                0.5,
                display.as_mut_ptr(),
                32
            ));
            let mut parsed = 0.0;
            assert!(params.text_to_value.unwrap()(
                plugin,
                info.id,
                display.as_ptr(),
                &mut parsed
            ));
            assert_eq!(parsed, 0.5);
            assert!(!p.activate.unwrap()(plugin, 0.0, 1, 256));
            assert!(p.activate.unwrap()(plugin, 48000.0, 1, 256));
            assert!(p.start_processing.unwrap()(plugin));
            let mut left = [1.0f32; 256];
            let mut right = [-0.5f32; 256];
            let mut inputs = [left.as_mut_ptr(), right.as_mut_ptr()];
            let mut out_l = [0.0f32; 256];
            let mut out_r = [0.0f32; 256];
            let mut outputs = [out_l.as_mut_ptr(), out_r.as_mut_ptr()];
            let input = clap_audio_buffer {
                data32: inputs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let mut output = clap_audio_buffer {
                data32: outputs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let midi = clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 32),
                port_index: 0,
                data: [0x90, 60, 100],
            };
            let mut events = Events {
                pointers: if index == 1 {
                    vec![(&midi as *const clap_event_midi).cast()]
                } else {
                    vec![]
                },
            };
            let event_list = clap_input_events {
                ctx: (&mut events as *mut Events).cast(),
                size: Some(event_size),
                get: Some(event_get),
            };
            let mut context = clap_process {
                steady_time: 0,
                frames_count: 256,
                transport: ptr::null(),
                audio_inputs: if index == 0 { &input } else { ptr::null() },
                audio_outputs: &mut output,
                audio_inputs_count: if index == 0 { 1 } else { 0 },
                audio_outputs_count: 1,
                in_events: &event_list,
                out_events: ptr::null(),
            };
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            if index == 0 {
                assert!(out_l.iter().all(|s| (*s - 0.5).abs() < 1e-6));
                assert!(out_r.iter().all(|s| (*s + 0.25).abs() < 1e-6));
            } else {
                assert!(out_l[..32].iter().all(|s| *s == 0.0));
                assert!(out_l[33..].iter().any(|s| s.abs() > 0.001));
                assert_eq!(out_l, out_r);
            }
            let automation = value(info.id, 0.25, 64);
            events.pointers = vec![(&automation as *const clap_event_param_value).cast()];
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            if index == 0 {
                assert!((out_l[63] - 0.5).abs() < 1e-6);
                assert!(out_l[64] < 0.5 && out_l[64] > 0.49);
            }
            events.pointers.clear();
            for _ in 0..4 {
                assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            }
            let mut actual = 0.0;
            assert!(params.get_value.unwrap()(plugin, info.id, &mut actual));
            assert_eq!(actual, 0.25);
            let state = &*ext(CLAP_EXT_STATE).cast::<clap_plugin_state>();
            let mut stream = Stream {
                bytes: vec![],
                position: 0,
            };
            let writer = clap_ostream {
                ctx: (&mut stream as *mut Stream).cast(),
                write: Some(write),
            };
            assert!(state.save.unwrap()(plugin, &writer));
            assert!(!stream.bytes.is_empty());
            p.stop_processing.unwrap()(plugin);
            let change = value(info.id, 0.8, 0);
            events.pointers = vec![(&change as *const clap_event_param_value).cast()];
            params.flush.unwrap()(plugin, &event_list, ptr::null());
            assert!(params.get_value.unwrap()(plugin, info.id, &mut actual));
            assert!((actual - 0.8).abs() < 1e-6);
            let reader = clap_istream {
                ctx: (&mut stream as *mut Stream).cast(),
                read: Some(read),
            };
            assert!(state.load.unwrap()(plugin, &reader));
            assert!(params.get_value.unwrap()(plugin, info.id, &mut actual));
            assert_eq!(actual, 0.25);
            let latency = &*ext(CLAP_EXT_LATENCY).cast::<clap_plugin_latency>();
            assert_eq!(latency.get.unwrap()(plugin), 0);
            let tail = &*ext(CLAP_EXT_TAIL).cast::<clap_plugin_tail>();
            assert_eq!(
                tail.get.unwrap()(plugin),
                if index == 1 { 24000 } else { 0 }
            );
            let render = &*ext(CLAP_EXT_RENDER).cast::<clap_plugin_render>();
            assert!(render.set.unwrap()(plugin, CLAP_RENDER_OFFLINE));
            assert!(!render.set.unwrap()(plugin, 99));
            assert!(p.start_processing.unwrap()(plugin));
            context.frames_count = 257;
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_ERROR);
            context.frames_count = 256;
            events.pointers.clear();
            p.reset.unwrap()(plugin);
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            p.stop_processing.unwrap()(plugin);
            p.deactivate.unwrap()(plugin);
            p.destroy.unwrap()(plugin);
            println!("PASS {name}: ports, MIDI timing/audio, automation, text conversion, partial-stream state restore, lifecycle, tail, latency, invalid block handling");
        }
        entry.deinit.unwrap()();
    }
    Ok(())
}
