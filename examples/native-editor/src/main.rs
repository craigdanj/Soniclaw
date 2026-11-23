use soniclaw::{
    automation::Mode,
    core::*,
    studio::{
        automation_write,
        history::History,
        native_edits::{Gestures, Phase},
        project::{Project, ProjectError},
        workspace, Descriptor,
    },
};
use soniclaw_host::{
    formats::{self, PluginFormat, TrustedLoader},
    studio::NativeFactory,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn main() -> Result<(), ProjectError> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).ok_or("NEW_FOLDER TRUSTED_PROTOCOL_FIXTURE")?);
    let module = Path::new(args.get(2).ok_or("trusted fixture")?).canonicalize()?;
    std::fs::create_dir(&out)?;
    let mut p = workspace::blank("Native edit".into(), 48000, 128)?;
    // SAFETY: explicitly selected owned native fixture.
    let product = unsafe { formats::products(PluginFormat::Vst3, &module) }?.remove(0);
    let d = Descriptor {
        id: "gain".into(),
        kind: "vst3".into(),
        state: serde_json::json!({"path":module,"class_id":product.id,"automation_ids":[9],"state_mode":"parameters"}),
    };
    p.document.tracks[0].inserts.push(d.clone());
    p.document
        .parameter_values
        .insert("gain.native.9".into(), 0.5);
    automation_write::arm(&mut p, "gain.native.9", 0.5, Mode::Touch)?;
    let mut writer = automation_write::Writer::new(&p, "gain.native.9", 0.)?;
    // SAFETY: trusted module capability constructed on this native owning thread.
    let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&module)) }?;
    let mut owners = vec![];
    let mut f = NativeFactory {
        helpers: &Default::default(),
        bus_graph: &mut Default::default(),
        spec: p.document.spec(),
        mode: soniclaw_host::studio::Mode::Live,
        timeline: &p.document.timeline,
        assets: &p.assets,
        states: &p.native_states,
        loader: &loader,
        owners: &mut owners,
        last_error: None,
    };
    let mut built = f.create_detailed(&d)?;
    built.processor.prepare(p.document.spec())?;
    let binary = if module.is_file() {
        module.clone()
    } else {
        module.join("Contents/x86_64-linux/HostProtocolFixture.so")
    };
    // SAFETY: same explicitly trusted owned fixture; exported seam performs legal main-thread native editor callbacks.
    let library = unsafe { libloading::Library::new(&binary) }?;
    let changed = unsafe {
        let edit: libloading::Symbol<unsafe extern "C" fn(u32, f64) -> bool> =
            library.get(b"soniclaw_fixture_editor_edit\0")?;
        edit(9, 0.8)
    };
    assert!(changed);
    owners[0].poll()?;
    let edits = owners[0].take_editor_edits();
    assert_eq!(
        edits.iter().map(|e| e.phase).collect::<Vec<_>>(),
        vec![Phase::Begin, Phase::Value, Phase::End]
    );
    let mut gestures = Gestures::default();
    let mut history = History::default();
    for e in &edits {
        gestures.apply(e, &mut p, None, Some(&mut writer), &mut history, 0.5)?;
    }
    gestures.finish(&mut p, &mut history)?;
    writer.commit(&mut p)?;
    assert_eq!(history.stats()?.undo, 1);
    assert_eq!(p.document.parameter_values["gain.native.9"], 0.8);
    history.undo(&mut p)?;
    assert_eq!(p.document.parameter_values["gain.native.9"], 0.5);
    history.redo(&mut p)?;
    let mut allocations = 0;
    let mut peak = 0f32;
    for n in 0..32 {
        let mut data = [0.25; 256];
        allocations += allocation_counter::measure(|| {
            built.processor.process(
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
    assert!((peak - 0.2).abs() < 1e-6);
    drop(built);
    let blob = owners[0].snapshot()?;
    assert!((blob.metadata().parameters[&9] - 0.8).abs() < 1e-6);
    p.native_states.insert("gain".into(), blob);
    p.document.tracks[0].inserts[0].state["native_state"] = serde_json::json!(true);
    p.save_new(&out.join("song"))?;
    let q = Project::open(&out.join("song"))?;
    assert_eq!(q.document.parameter_values["gain.native.9"], 0.8);
    assert!(q.document.automation.lanes[0]
        .points
        .iter()
        .any(|e| (e.value - 0.8).abs() < 1e-6));
    owners[0].take_editor_edits();
    assert!(owners[0]
        .accept_editor_edit(600, Phase::Value, Some(0.5))
        .is_err());
    let mut overflow = false;
    for _ in 0..100 {
        if owners[0]
            .accept_editor_edit(9, Phase::Value, Some(0.2))
            .is_err()
        {
            overflow = true;
            break;
        }
    }
    assert!(overflow);
    assert!(owners[0].take_editor_edits().len() <= 256);
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","native_callback_order":["Begin","Value","End"],"host_value":0.8,"output_peak":peak,"callback_allocations":allocations,"undo_redo":true,"touch_lane_saved":true,"native_snapshot_value":0.8,"readonly_rejected":true,"overflow_reported":true,"native_window_opened":false}),
        )?,
    )?;
    Ok(())
}
