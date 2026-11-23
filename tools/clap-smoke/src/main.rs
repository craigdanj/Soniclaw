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
// SAFETY: caller holds the loaded library, host and valid CLAP factory. This host
// serializes callbacks and owns every buffer/stream for its complete callback.
unsafe fn project_smoke(factory: &clap_plugin_factory, host: &clap_host) {
    unsafe {
        assert_eq!(factory.get_plugin_count.unwrap()(factory), 1);
        let descriptor = factory.get_plugin_descriptor.unwrap()(factory, 0);
        assert!(!descriptor.is_null());
        assert_eq!(
            CStr::from_ptr((*descriptor).id),
            c"org.soniclaw.project-rack"
        );
        let plugin = factory.create_plugin.unwrap()(factory, host, (*descriptor).id);
        assert!(!plugin.is_null());
        let p = &*plugin;
        assert!(p.init.unwrap()(plugin));
        let state =
            &*p.get_extension.unwrap()(plugin, CLAP_EXT_STATE.as_ptr()).cast::<clap_plugin_state>();
        let save = || {
            let mut s = Stream {
                bytes: vec![],
                position: 0,
            };
            let writer = clap_ostream {
                ctx: (&mut s as *mut Stream).cast(),
                write: Some(write),
            };
            assert!(state.save.unwrap()(plugin, &writer));
            s.bytes
        };
        let load = |bytes: Vec<u8>| {
            let mut s = Stream { bytes, position: 0 };
            let reader = clap_istream {
                ctx: (&mut s as *mut Stream).cast(),
                read: Some(read),
            };
            state.load.unwrap()(plugin, &reader)
        };
        let render = || {
            let (mut left, mut right) = ([0.0_f32; 128], [0.0_f32; 128]);
            let mut source = [1.0_f32; 128];
            let mut inputs = [source.as_mut_ptr(), source.as_mut_ptr()];
            let mut outputs = [left.as_mut_ptr(), right.as_mut_ptr()];
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
            let context = clap_process {
                steady_time: 0,
                frames_count: 128,
                transport: ptr::null(),
                audio_inputs: &input,
                audio_outputs: &mut output,
                audio_inputs_count: 1,
                audio_outputs_count: 1,
                in_events: ptr::null(),
                out_events: ptr::null(),
            };
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            left
        };
        assert!(p.activate.unwrap()(plugin, 48000.0, 1, 128));
        assert!(p.start_processing.unwrap()(plugin));
        let original = render();
        assert!(original.iter().any(|x| x.abs() > 0.001));
        let bytes = save();
        let mut document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["schema_version"], 2);
        assert_eq!(document["nodes"].as_array().unwrap().len(), 6);
        document["outputs"]["limiter"] = serde_json::json!(0.25);
        let changed = serde_json::to_vec(&document).unwrap();
        assert!(!load(changed.clone()));
        assert_eq!(save(), bytes);
        p.stop_processing.unwrap()(plugin);
        p.deactivate.unwrap()(plugin);
        assert!(load(changed));
        assert!(p.activate.unwrap()(plugin, 48000.0, 1, 128));
        assert!(p.start_processing.unwrap()(plugin));
        let quarter = render();
        for (a, b) in original.iter().zip(quarter) {
            assert!((*a * 0.25 - b).abs() < 1e-6);
        }
        let before = save();
        document["parameters"]["gain.gain"] = serde_json::json!(999.0);
        assert!(!load(serde_json::to_vec(&document).unwrap()));
        assert_eq!(save(), before);
        document["parameters"]["gain.gain"] = serde_json::json!(0.5);
        assert!(load(serde_json::to_vec(&document).unwrap()));
        render();
        let actual: serde_json::Value = serde_json::from_slice(&save()).unwrap();
        assert_eq!(actual["parameters"]["gain.gain"], 0.5);
        p.stop_processing.unwrap()(plugin);
        p.deactivate.unwrap()(plugin);
        p.destroy.unwrap()(plugin);
        println!("PASS Soniclaw Project Rack: dynamic CLAP export, full graph state, partial-stream restore, active structural guard, deactivated rebuild, scalar staging, malformed-state rejection and rendered routing gain");
    }
}

// SAFETY: the caller retains the library and serializes callbacks; every pointer
// below targets owned live storage for the complete host/plug-in call.
unsafe fn dsp_smoke(factory: &clap_plugin_factory, host: &clap_host, workbench: bool) {
    unsafe {
        let count = if workbench { 4 } else { 2 };
        assert_eq!(factory.get_plugin_count.unwrap()(factory), count);
        for index in 0..count {
            let d = factory.get_plugin_descriptor.unwrap()(factory, index);
            assert!(!d.is_null());
            let id = CStr::from_ptr((*d).id);
            assert_eq!(
                id,
                if workbench && index == 0 {
                    c"org.soniclaw.workbench.fx"
                } else if workbench && index == 1 {
                    c"org.soniclaw.workbench.granular"
                } else if workbench && index == 2 {
                    c"org.soniclaw.workbench.polymod"
                } else if workbench {
                    c"org.soniclaw.workbench.sampler"
                } else if index == 0 {
                    c"org.soniclaw.dsp-lab.effect"
                } else {
                    c"org.soniclaw.dsp-lab.wavetable"
                }
            );
            let plugin = factory.create_plugin.unwrap()(factory, host, (*d).id);
            assert!(!plugin.is_null());
            let p = &*plugin;
            assert!(p.init.unwrap()(plugin));
            let params = &*p.get_extension.unwrap()(plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>();
            assert!(params.count.unwrap()(plugin) >= 6);
            let state = &*p.get_extension.unwrap()(plugin, CLAP_EXT_STATE.as_ptr())
                .cast::<clap_plugin_state>();
            let latency = &*p.get_extension.unwrap()(plugin, CLAP_EXT_LATENCY.as_ptr())
                .cast::<clap_plugin_latency>();
            assert!(p.activate.unwrap()(plugin, 48000.0, 1, 256));
            assert!(p.start_processing.unwrap()(plugin));
            assert_eq!(
                latency.get.unwrap()(plugin),
                if workbench {
                    252
                } else if index == 0 {
                    1276
                } else {
                    0
                }
            );
            let mut input = [0.25_f32; 256];
            let mut inputs = [input.as_mut_ptr(), input.as_mut_ptr()];
            let mut left = [0.0; 256];
            let mut right = [0.0; 256];
            let mut outputs = [left.as_mut_ptr(), right.as_mut_ptr()];
            let source = clap_audio_buffer {
                data32: inputs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let mut destination = clap_audio_buffer {
                data32: outputs.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            };
            let note = clap_event_note {
                header: header::<clap_event_note>(CLAP_EVENT_NOTE_ON, 32),
                note_id: 77,
                port_index: 0,
                channel: 1,
                key: 60,
                velocity: 0.8,
            };
            let mut events = Events { pointers: vec![] };
            let list = clap_input_events {
                ctx: (&mut events as *mut Events).cast(),
                size: Some(event_size),
                get: Some(event_get),
            };
            let context = clap_process {
                steady_time: 0,
                frames_count: 256,
                transport: ptr::null(),
                audio_inputs: if index == 0 { &source } else { ptr::null() },
                audio_outputs: &mut destination,
                audio_inputs_count: if index == 0 { 1 } else { 0 },
                audio_outputs_count: 1,
                in_events: &list,
                out_events: ptr::null(),
            };
            let mut audible = false;
            for block in 0..32 {
                events.pointers.clear();
                if index > 0 && block == 0 {
                    events
                        .pointers
                        .push((&note as *const clap_event_note).cast());
                }
                assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
                assert!(left.iter().chain(&right).all(|x| x.is_finite()));
                audible |= left.iter().any(|x| x.abs() > 1e-5);
            }
            assert!(audible);
            let mut stream = Stream {
                bytes: vec![],
                position: 0,
            };
            let writer = clap_ostream {
                ctx: (&mut stream as *mut Stream).cast(),
                write: Some(write),
            };
            assert!(state.save.unwrap()(plugin, &writer));
            let doc: serde_json::Value = serde_json::from_slice(&stream.bytes).unwrap();
            assert_eq!(doc["schema_version"], 2);
            assert!(!doc["nodes"].as_array().unwrap().is_empty());
            let reader = clap_istream {
                ctx: (&mut stream as *mut Stream).cast(),
                read: Some(read),
            };
            assert!(state.load.unwrap()(plugin, &reader));
            p.stop_processing.unwrap()(plugin);
            p.deactivate.unwrap()(plugin);
            stream.position = 0;
            assert!(state.load.unwrap()(plugin, &reader));
            assert!(p.activate.unwrap()(plugin, 48000.0, 1, 256));
            assert!(p.start_processing.unwrap()(plugin));
            events.pointers.clear();
            assert_eq!(p.process.unwrap()(plugin, &context), CLAP_PROCESS_CONTINUE);
            p.stop_processing.unwrap()(plugin);
            p.deactivate.unwrap()(plugin);
            p.destroy.unwrap()(plugin);
            println!("PASS {}: independent dynamic CLAP load, parameters, note/audio, declared latency, complete-state save, partial-stream scalar restore and stopped rebuild",CStr::from_ptr((*d).name).to_string_lossy());
        }
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
        let project_mode = std::env::args().nth(2).as_deref() == Some("--project");
        let dsp_mode = std::env::args().nth(2).as_deref() == Some("--dsp");
        let workbench_mode = std::env::args().nth(2).as_deref() == Some("--workbench");
        if !project_mode && !dsp_mode && !workbench_mode {
            assert_eq!(count, 2);
        }
        let host = clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"Soniclaw independent smoke host".as_ptr(),
            vendor: c"Soniclaw tests".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.146.0".as_ptr(),
            get_extension: Some(host_extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(request),
        };
        if dsp_mode || workbench_mode {
            dsp_smoke(factory, &host, workbench_mode);
            entry.deinit.unwrap()();
            return Ok(());
        }
        if project_mode {
            project_smoke(factory, &host);
            entry.deinit.unwrap()();
            return Ok(());
        }
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
