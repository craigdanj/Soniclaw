//! ABI-level state/expression checks through the same callbacks exported by CLAP.
use super::*;
use clap_sys::audio_buffer::clap_audio_buffer;
use soniclaw::{
    plugin::PluginDescriptor,
    state::{ProcessorKind, ProjectState, RackNode},
};
use std::ffi::CString;
fn synth() -> PluginInstance {
    let mut e = soniclaw::engine::Engine::default();
    e.push(soniclaw::dsp::ExpressiveSynth::<8>::default());
    PluginInstance::new(
        PluginDescriptor {
            id: "org.soniclaw.test-expression",
            name: "Expression",
            vendor: "Soniclaw",
            version: "0.146.0",
            is_instrument: true,
            accepts_midi: true,
        },
        e,
        vec![],
    )
    .unwrap()
}
fn rack() -> PluginInstance {
    let mut p = ProjectState::empty("org.soniclaw.test-rack", "Rack");
    p.nodes.push(RackNode {
        custom: None,
        voice_modulation: None,
        asset: None,
        id: "amp".into(),
        kind: ProcessorKind::Gain,
        settings: std::collections::BTreeMap::from([("gain".into(), 0.5)]),
        input_gain: 1.0,
    });
    p.outputs.insert("amp".into(), 1.0);
    soniclaw::project::create_plugin(
        PluginDescriptor {
            id: "org.soniclaw.test-rack",
            name: "Rack",
            vendor: "Soniclaw",
            version: "0.146.0",
            is_instrument: false,
            accepts_midi: false,
        },
        p,
        ".",
    )
    .unwrap()
}
static SYNTH: ClapDescriptor = descriptor(
    c"org.soniclaw.test-expression",
    c"Expression",
    c"Soniclaw",
    c"0.146.0",
    true,
);
static RACK: ClapDescriptor = descriptor(
    c"org.soniclaw.test-rack",
    c"Rack",
    c"Soniclaw",
    c"0.146.0",
    false,
);
static DEFS: &[Definition] = &[
    Definition {
        descriptor: &SYNTH,
        create: synth,
        instrument: true,
        tail_seconds: 1.0,
    },
    Definition {
        descriptor: &RACK,
        create: rack,
        instrument: false,
        tail_seconds: 0.0,
    },
];
unsafe extern "C" fn extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
struct Harness {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    instrument: bool,
}
impl Harness {
    fn new(instrument: bool) -> Self {
        assert!(initialize(DEFS));
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"test".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.146.0".as_ptr(),
            get_extension: Some(extension),
            request_restart: None,
            request_process: None,
            request_callback: None,
        });
        let id = CString::new(if instrument {
            "org.soniclaw.test-expression"
        } else {
            "org.soniclaw.test-rack"
        })
        .unwrap();
        // SAFETY: host allocation is stable until destroy; id lives through create.
        unsafe {
            let plugin = create(ptr::null(), &*host, id.as_ptr());
            assert!(!plugin.is_null());
            assert!((*plugin).init.unwrap()(plugin));
            Self {
                plugin,
                _host: host,
                instrument,
            }
        }
    }
    fn start(&self) {
        unsafe {
            assert!((*self.plugin).activate.unwrap()(
                self.plugin,
                48000.0,
                1,
                128
            ));
            assert!((*self.plugin).start_processing.unwrap()(self.plugin));
        }
    }
    fn stop(&self) {
        unsafe {
            (*self.plugin).stop_processing.unwrap()(self.plugin);
            (*self.plugin).deactivate.unwrap()(self.plugin);
        }
    }
    fn render(&self, events: &[*const clap_event_header]) -> ([f32; 128], [f32; 128]) {
        unsafe extern "C" fn size(list: *const clap_input_events) -> u32 {
            unsafe { (&*((*list).ctx.cast::<Vec<*const clap_event_header>>())).len() as u32 }
        }
        unsafe extern "C" fn get(
            list: *const clap_input_events,
            index: u32,
        ) -> *const clap_event_header {
            unsafe { (&*((*list).ctx.cast::<Vec<*const clap_event_header>>()))[index as usize] }
        }
        let mut pointers = events.to_vec();
        let event_list = clap_input_events {
            ctx: (&mut pointers as *mut Vec<*const clap_event_header>).cast(),
            size: Some(size),
            get: Some(get),
        };
        let (mut left, mut right) = ([0.0; 128], [0.0; 128]);
        let mut outputs = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut input = [1.0f32; 128];
        let mut inputs = [input.as_mut_ptr(), input.as_mut_ptr()];
        let source = clap_audio_buffer {
            data32: inputs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut dest = clap_audio_buffer {
            data32: outputs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let process = clap_process {
            steady_time: 0,
            frames_count: 128,
            transport: ptr::null(),
            audio_inputs: if self.instrument {
                ptr::null()
            } else {
                &source
            },
            audio_outputs: &mut dest,
            audio_inputs_count: if self.instrument { 0 } else { 1 },
            audio_outputs_count: 1,
            in_events: &event_list,
            out_events: ptr::null(),
        };
        // SAFETY: all event/buffer pointers refer to live, correctly sized storage.
        unsafe {
            assert_eq!(
                (*self.plugin).process.unwrap()(self.plugin, &process),
                CLAP_PROCESS_CONTINUE
            );
        }
        (left, right)
    }
    fn state(&self) -> &clap_plugin_state {
        unsafe {
            &*(*self.plugin).get_extension.unwrap()(self.plugin, CLAP_EXT_STATE.as_ptr())
                .cast::<clap_plugin_state>()
        }
    }
    fn save(&self) -> String {
        let mut blob = Blob {
            bytes: vec![],
            position: 0,
        };
        let stream = clap_ostream {
            ctx: (&mut blob as *mut Blob).cast(),
            write: Some(write),
        };
        unsafe {
            assert!(self.state().save.unwrap()(self.plugin, &stream));
        }
        String::from_utf8(blob.bytes).unwrap()
    }
    fn load(&self, json: &str) -> bool {
        let mut blob = Blob {
            bytes: json.as_bytes().to_vec(),
            position: 0,
        };
        let stream = clap_istream {
            ctx: (&mut blob as *mut Blob).cast(),
            read: Some(read),
        };
        unsafe { self.state().load.unwrap()(self.plugin, &stream) }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        unsafe {
            (*self.plugin).destroy.unwrap()(self.plugin);
        }
    }
}
struct Blob {
    bytes: Vec<u8>,
    position: usize,
}
unsafe extern "C" fn write(s: *const clap_ostream, data: *const c_void, size: u64) -> i64 {
    unsafe {
        let b = &mut *(*s).ctx.cast::<Blob>();
        let count = (size as usize).min(7);
        b.bytes
            .extend_from_slice(std::slice::from_raw_parts(data.cast::<u8>(), count));
        count as i64
    }
}
unsafe extern "C" fn read(s: *const clap_istream, data: *mut c_void, size: u64) -> i64 {
    unsafe {
        let b = &mut *(*s).ctx.cast::<Blob>();
        let count = (size as usize).min(11).min(b.bytes.len() - b.position);
        ptr::copy_nonoverlapping(b.bytes[b.position..].as_ptr(), data.cast::<u8>(), count);
        b.position += count;
        count as i64
    }
}
fn header<T>(type_: u16, time: u32) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_,
        flags: 0,
    }
}
#[test]
fn clap_note_expression_routes_with_sample_offsets_and_note_identity() {
    let h = Harness::new(true);
    h.start();
    let note = clap_event_note {
        header: header::<clap_event_note>(CLAP_EVENT_NOTE_ON, 8),
        note_id: 77,
        port_index: 0,
        channel: 1,
        key: 60,
        velocity: 1.0,
    };
    let pan = clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 32),
        expression_id: CLAP_NOTE_EXPRESSION_PAN,
        note_id: 77,
        port_index: 0,
        channel: -1,
        key: -1,
        value: 0.0,
    };
    let volume = clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 64),
        expression_id: CLAP_NOTE_EXPRESSION_VOLUME,
        value: 0.0,
        ..pan
    };
    let (left, right) = h.render(&[
        (&note as *const clap_event_note).cast(),
        (&pan as *const clap_event_note_expression).cast(),
        (&volume as *const clap_event_note_expression).cast(),
    ]);
    assert!(left[..8].iter().all(|x| *x == 0.0));
    assert!(left[10..32].iter().any(|x| x.abs() > 0.0001));
    assert_eq!(left[10..32], right[10..32]);
    assert!(right[32..].iter().all(|x| *x == 0.0));
    assert!(left[32..64].iter().any(|x| x.abs() > 0.0001));
    assert!(left[64..].iter().all(|x| *x == 0.0));
    h.stop();
}
#[test]
fn clap_project_roundtrip_and_structural_restart_guard() {
    let h = Harness::new(false);
    h.start();
    assert!(h.render(&[]).0.iter().all(|x| *x == 0.5));
    let original = h.save();
    let mut project = ProjectState::from_json(&original).unwrap();
    project.nodes[0].input_gain = 0.25;
    let changed = project.to_json().unwrap();
    assert!(!h.load(&changed));
    assert_eq!(h.save(), original);
    h.stop();
    assert!(h.load(&changed));
    h.start();
    assert!(h.render(&[]).0.iter().all(|x| *x == 0.125));
    project.parameters.insert("amp.gain".into(), 999.0);
    assert!(!h.load(&project.to_json().unwrap()));
    assert!(h.render(&[]).0.iter().all(|x| *x == 0.125));
    let legacy = Preset {
        schema_version: 1,
        product_id: project.product_id,
        name: "legacy".into(),
        parameters: std::collections::BTreeMap::from([("amp.gain".into(), 0.8)]),
    };
    assert!(h.load(&legacy.to_json().unwrap()));
    for _ in 0..5 {
        h.render(&[]);
    }
    assert!((h.render(&[]).0[0] - 0.2).abs() < 1e-5);
    h.stop();
}
