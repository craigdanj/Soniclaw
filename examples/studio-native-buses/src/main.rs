use soniclaw::{
    core::*,
    studio::{
        native_buses::{self, Request, Return},
        project::{Project, ProjectError},
        workspace::{self, Command, Kind},
        Descriptor,
    },
};
use soniclaw_host::{
    formats::{self, PluginFormat, TrustedLoader},
    studio::{Mode, NativeFactory},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
fn fixture(module: &Path, loader: &TrustedLoader) -> Result<Project, ProjectError> {
    let mut p = workspace::blank("Native ports".into(), 48000, 128)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "key".into(),
            name: "Key".into(),
            kind: Kind::Audio,
        },
    )?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "return".into(),
            name: "Output 2".into(),
            kind: Kind::Audio,
        },
    )?;
    for (i, value) in [(0, 0.25), (1, 0.5)] {
        let id = format!("pcm{i}");
        p.assets.insert(
            id.clone(),
            soniclaw::clips::AudioAsset {
                sample_rate: 48000,
                channels: 1,
                samples: vec![value; 8192].into(),
            },
        );
        p.document.tracks[i].audio.push(soniclaw::clips::AudioClip {
            id: id.clone(),
            asset: id,
            track: i as u16,
            start_frame: 0,
            length_frames: 8192,
            source_offset: 0,
            source_length: 8192,
            looping: false,
            gain: 1.,
            fade_in: 0,
            fade_out: 0,
            enabled: true,
            edits: None,
        });
    }
    // SAFETY: explicitly selected owned multi-bus fixture; its environment flag is scoped to this example process.
    let product = unsafe { formats::products(PluginFormat::Vst3, module) }?.remove(0);
    let bus = loader.load_vst3_buses(&product, p.document.spec())?;
    let layout = bus.layout().clone();
    drop(bus);
    assert_eq!(layout.inputs.len(), 2);
    assert_eq!(layout.outputs.len(), 2);
    let mut d = Descriptor {
        id: "native.fx".into(),
        kind: "vst3".into(),
        state: serde_json::json!({"path":module,"class_id":product.id,"automation_ids":[9],"state_mode":"parameters"}),
    };
    Request {
        layout,
        sidechains: BTreeMap::from([(1, "key".into())]),
    }
    .store(&mut d)?;
    p.document.tracks[0].inserts.push(d);
    p.document.tracks[2].instrument = Some(
        Return {
            processor: "native.fx".into(),
            bus: 1,
            channels: vec![0, 1],
        }
        .descriptor("native.return")?,
    );
    p.document.mixer.strips[0].mute = true;
    p.document.mixer.strips[1].mute = true;
    p.validate()?;
    Ok(p)
}
fn main() -> Result<(), ProjectError> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).ok_or("NEW_FOLDER TRUSTED_PROTOCOL_FIXTURE")?);
    let module = Path::new(args.get(2).ok_or("fixture path")?).canonicalize()?;
    std::fs::create_dir(&out)?;
    // Fixture-only configuration, not a user-device setting.
    std::env::set_var("SONICLAW_FIXTURE_BUSES", "1");
    // SAFETY: explicit trusted owned fixture selection on the owning thread.
    let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&module)) }?;
    let mut p = fixture(&module, &loader)?;
    let mut owners = Vec::new();
    let mut registry = native_buses::Registry::default();
    let mut prepared = {
        let mut f = NativeFactory {
            helpers: &Default::default(),
            bus_graph: &mut registry,
            spec: p.document.spec(),
            mode: Mode::Live,
            timeline: &p.document.timeline,
            assets: &p.assets,
            states: &p.native_states,
            loader: &loader,
            owners: &mut owners,
            last_error: None,
        };
        p.document.compile(&p.assets, &mut f)?
    };
    assert_eq!(prepared.latency_frames(), 1024);
    let mut allocations = 0;
    let mut peak = 0f32;
    for n in 0..48 {
        let mut data = [0.; 256];
        allocations += allocation_counter::measure(|| {
            prepared.process(
                &mut AudioBuffer::new(&mut data, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        playing: true,
                        sample_position: n * 128,
                        ..Default::default()
                    },
                },
            )
        })
        .count_total;
        peak = peak.max(data.iter().copied().fold(0f32, |a, b| a.max(b.abs())));
        std::thread::sleep(Duration::from_millis(4));
    }
    assert_eq!(allocations, 0);
    let main = registry.outputs["native.fx"][0].sample(6143, 0);
    let separate = registry.outputs["native.fx"][1].sample(6143, 0);
    assert!((main - 0.0625).abs() < 1e-6, "main {main}");
    assert!((separate - 0.25).abs() < 1e-6, "separate {separate}");
    assert!(peak > 0.1);
    drop(prepared);
    let state = owners[0].snapshot()?;
    owners.clear();
    p.native_states.insert("native.fx".into(), state);
    p.document.tracks[0].inserts[0].state["native_state"] = serde_json::json!(true);
    p.save_new(&out.join("song"))?;
    let p = Project::open(&out.join("song"))?;
    let mut registry = native_buses::Registry::default();
    let mut offline = {
        let mut f = NativeFactory {
            helpers: &Default::default(),
            bus_graph: &mut registry,
            spec: p.document.spec(),
            mode: Mode::Offline,
            timeline: &p.document.timeline,
            assets: &p.assets,
            states: &p.native_states,
            loader: &loader,
            owners: &mut owners,
            last_error: None,
        };
        p.document.compile_offline(&p.assets, &p.disk, &mut f)?
    };
    assert_eq!(offline.latency_frames(), 0);
    let mut data = [0.; 256];
    offline.render_block(
        &mut AudioBuffer::new(&mut data, 2)?,
        &ProcessContext {
            events: &[],
            transport: Transport {
                playing: true,
                ..Default::default()
            },
        },
    )?;
    assert!((registry.outputs["native.fx"][0].sample(127, 0) - 0.0625).abs() < 1e-6);
    assert!((registry.outputs["native.fx"][1].sample(127, 1) - 0.25).abs() < 1e-6);
    drop(offline);
    owners.clear();
    let mut cyclic = p.clone();
    let d = &mut cyclic.document.tracks[0].inserts[0];
    let mut r = Request::from_descriptor(d)?.expect("saved");
    r.sidechains.insert(1, "return".into());
    r.store(d)?;
    assert!(cyclic.validate().is_err());
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","live_latency":1024,"offline_latency":0,"native_keyed_main":main,"separate_output":separate,"muted_key_still_drives_native_input":true,"independent_return_track":true,"callback_allocations":allocations,"save_reopen":true,"cycle_rejected":true}),
        )?,
    )?;
    Ok(())
}
