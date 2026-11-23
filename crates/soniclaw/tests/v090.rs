use soniclaw::{plugin::*, prelude::*, project::*, registry::*, state::*};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn registry() -> Arc<ProcessorRegistry> {
    let mut r = ProcessorRegistry::default();
    soniclaw_example_processors::register(&mut r).unwrap();
    Arc::new(r)
}
fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "test.registry",
        name: "Registered effect",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: false,
        accepts_midi: false,
    }
}
fn state(registry: &ProcessorRegistry) -> ProjectState {
    let mut s = ProjectState::empty("test.registry", "Tape test");
    let mut n = registry
        .node("tape", soniclaw_example_processors::TAPE_ID)
        .unwrap();
    n.input_gain = 1.0;
    s.nodes.push(n);
    s.outputs.insert("tape".into(), 1.0);
    s
}
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    }
}
fn plugin(r: Arc<ProcessorRegistry>, s: ProjectState) -> PluginInstance {
    create_plugin_with_registry(descriptor(), s, ".", r).unwrap()
}
#[test]
fn compiled_pack_discovery_and_registered_audio_has_no_callback_heap_work() {
    let r = registry();
    assert_eq!(r.entries().count(), 1);
    assert_eq!(
        r.metadata(soniclaw_example_processors::TAPE_ID)
            .unwrap()
            .category,
        "Distortion"
    );
    let s = state(&r);
    let mut p = plugin(r, s);
    assert_eq!(p.parameters().len(), 4);
    assert_eq!(p.parameters()[0].info().id, "tape.drive");
    p.activate(spec()).unwrap();
    let mut data = [0.2; 512];
    let allocations = allocation_counter::measure(|| p.process(&mut data, &[]).unwrap());
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
    assert!(data.iter().all(|v| v.is_finite()));
    assert!(data.iter().any(|v| (*v - 0.2).abs() > 0.01));
}
#[test]
fn old_custom_state_migrates_and_host_restore_reuses_exact_parameter_handles() {
    let r = registry();
    let mut s = state(&r);
    let c = s.nodes[0].custom.as_mut().unwrap();
    c.version = 1;
    c.data = serde_json::json!({"bias":0.1});
    s.parameters.insert("tape.drive".into(), 3.0);
    let json = s.to_json().unwrap();
    let mut p = plugin(r.clone(), s.clone());
    assert_eq!(
        p.project_state().unwrap().nodes[0]
            .custom
            .as_ref()
            .unwrap()
            .version,
        2
    );
    let params = p.parameters().to_vec();
    p.parameters()[0].set(5.0).unwrap();
    p.load_state(&json).unwrap();
    assert_eq!(p.parameters()[0].get(), 3.0);
    assert!(params
        .iter()
        .zip(p.parameters())
        .all(|(a, b)| Arc::ptr_eq(a, b)));
    let saved = ProjectState::from_json(&p.save_state().unwrap()).unwrap();
    let c = saved.nodes[0].custom.as_ref().unwrap();
    assert_eq!(c.version, 2);
    assert_eq!(c.data, serde_json::json!({"asymmetry":0.1}));
}
#[test]
fn failed_restore_and_missing_registration_leave_state_and_values_intact() {
    let r = registry();
    let s = state(&r);
    let mut p = plugin(r.clone(), s.clone());
    p.parameters()[0].set(4.0).unwrap();
    let before = p.save_state().unwrap();
    let mut bad = s.clone();
    bad.nodes[0].custom.as_mut().unwrap().data = serde_json::json!({"asymmetry":2.0});
    assert!(p.load_state(&bad.to_json().unwrap()).is_err());
    assert_eq!(p.save_state().unwrap(), before);
    bad = s.clone();
    bad.nodes[0].custom.as_mut().unwrap().processor_id = "vendor.missing".into();
    let e = p.load_state(&bad.to_json().unwrap()).unwrap_err();
    assert!(e.to_string().contains("vendor.missing"));
    assert_eq!(p.save_state().unwrap(), before);
    bad = s.clone();
    bad.nodes[0].custom.as_mut().unwrap().version = 99;
    assert!(p.load_state(&bad.to_json().unwrap()).is_err());
    assert_eq!(p.save_state().unwrap(), before);
    bad = s;
    bad.parameters.insert("tape.drive".into(), 999.0);
    assert!(p.load_state(&bad.to_json().unwrap()).is_err());
    assert_eq!(p.save_state().unwrap(), before);
}
#[test]
fn duplicate_registration_bad_metadata_and_settings_are_rejected() {
    let mut r = ProcessorRegistry::default();
    soniclaw_example_processors::register(&mut r).unwrap();
    assert!(soniclaw_example_processors::register(&mut r).is_err());
    assert_eq!(r.entries().count(), 1);
    let mut s = state(&r);
    s.nodes[0].settings.insert("unknown".into(), 1.0);
    assert!(create_plugin_with_registry(descriptor(), s, ".", Arc::new(r)).is_err());
    let r = registry();
    let mut s = state(&r);
    s.nodes[0].custom = None;
    assert!(s.validate("test.registry").is_err());
    let mut s = state(&r);
    s.nodes[0].custom.as_mut().unwrap().data = serde_json::Value::String("x".repeat(65537));
    assert!(s.to_json().is_err());
}
#[test]
fn sample_accurate_automation_is_partition_independent_for_registered_nodes() {
    let r = registry();
    let s = state(&r);
    let render = |block: usize| {
        let mut p = plugin(r.clone(), s.clone());
        p.activate(spec()).unwrap();
        let mut data = vec![0.2; 1024];
        for (i, b) in data.chunks_mut(block * 2).enumerate() {
            let start = i * block;
            let end = start + b.len() / 2;
            let controls = if (start..end).contains(&128) {
                vec![AutomationEvent {
                    frame: 128 - start,
                    parameter: parameter_id("tape.drive"),
                    value: 8.0,
                }]
            } else {
                vec![]
            };
            p.process_automated(b, &[], &controls).unwrap();
        }
        data
    };
    let a = render(256);
    let b = render(17);
    assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6));
    assert!(a[700] > a[10]);
}
#[test]
fn repeated_construction_frees_owned_parameter_metadata() {
    let r = registry();
    let s = state(&r);
    let count = allocation_counter::measure(|| {
        for _ in 0..16 {
            drop(plugin(r.clone(), s.clone()));
        }
    });
    assert_eq!(count.count_current, 0);
}

#[test]
fn standalone_session_keeps_registry_across_edits_undo_redo_and_failure() {
    let r = registry();
    let s = state(&r);
    let mut session = ProjectSession::with_registry(s, ".", 8, r).unwrap();
    session.build(descriptor()).unwrap();
    let p = session
        .edit(descriptor(), "Drive", |s| {
            s.nodes[0].settings.insert("drive".into(), 5.0);
        })
        .unwrap();
    assert_eq!(p.parameters()[0].get(), 5.0);
    assert_eq!(
        session.undo(descriptor()).unwrap().unwrap().parameters()[0].get(),
        2.0
    );
    assert_eq!(
        session.redo(descriptor()).unwrap().unwrap().parameters()[0].get(),
        5.0
    );
    let before = session.history().current().clone();
    assert!(session
        .edit(descriptor(), "Missing", |s| s.nodes[0]
            .custom
            .as_mut()
            .unwrap()
            .processor_id =
            "missing.pack".into())
        .is_err());
    assert_eq!(session.history().current(), &before);
    let p = session
        .edit(descriptor(), "Legacy", |s| {
            let c = s.nodes[0].custom.as_mut().unwrap();
            c.version = 1;
            c.data = serde_json::json!({"bias":0.1});
        })
        .unwrap();
    assert_eq!(
        p.project_state().unwrap().nodes[0]
            .custom
            .as_ref()
            .unwrap()
            .version,
        2
    );
    assert_eq!(
        session.history().current().nodes[0]
            .custom
            .as_ref()
            .unwrap()
            .version,
        2
    );
}
#[test]
fn registry_is_used_by_background_load_and_bundle_roundtrip() {
    let r = registry();
    let mut s = state(&r);
    s.nodes[0].custom.as_mut().unwrap().version = 1;
    s.nodes[0].custom.as_mut().unwrap().data = serde_json::json!({"bias":0.05});
    let root = std::env::temp_dir().join(format!(
        "soniclaw-registry-roundtrip-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let bundle = root.join("project.soniclaw");
    soniclaw::project_storage::pack_bundle(&s, &root, &bundle).unwrap();
    let worker_r = r.clone();
    let mut loader = soniclaw::project_loading::AsyncProjectLoader::with_factory(move |s, root| {
        create_plugin_with_registry(descriptor(), s.clone(), root, worker_r.clone())
    });
    loader
        .submit(
            soniclaw::project_loading::LoadRequest::Bundle {
                path: bundle,
                destination: root.join("unpacked"),
            },
            Some(spec()),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let loaded = loop {
        if let Some(o) = loader.try_poll() {
            break o.result.unwrap();
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(loaded.state().nodes[0].custom.as_ref().unwrap().version, 2);
    assert!(loaded.prepared_spec().is_some());
    drop(loaded);
    drop(loader);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn graph_modulation_routes_to_registered_parameters() {
    use soniclaw::dsp::modulation::*;
    let r = registry();
    let mut s = state(&r);
    s.modulation = Some(ModulationConfig {
        sources: vec![SourceConfig::Macro { index: 0 }],
        routes: vec![ModulationRoute {
            source: 0,
            destination: "tape.drive".into(),
            amount: 0.2,
        }],
        macros: vec![1.0],
    });
    let mut p = plugin(r, s);
    p.activate(spec()).unwrap();
    p.process(&mut [0.2; 128], &[]).unwrap();
    let drive = &p.parameters()[0];
    assert_eq!(drive.get(), 2.0);
    assert!(drive.modulated() > 2.0);
}
#[test]
fn registration_default_state_and_invalid_ranges_are_checked() {
    struct Passthrough;
    impl AudioProcessor for Passthrough {
        fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
            s.validate().map(|_| ())
        }
        fn process(&mut self, _: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {}
        fn reset(&mut self) {}
    }
    let metadata = ProcessorMetadata {
        id: "test.custom",
        name: "Custom",
        category: "Test",
        version: 1,
        instrument: false,
    };
    let mut r = ProcessorRegistry::default();
    r.register(
        Registration::new(metadata, vec![], |c| {
            assert_eq!(
                c.node.custom.as_ref().unwrap().data,
                serde_json::json!({"mode":"clean"})
            );
            Ok(Box::new(Passthrough))
        })
        .with_default_state(serde_json::json!({"mode":"clean"})),
    )
    .unwrap();
    let n = r.node("clean", "test.custom").unwrap();
    let mut s = ProjectState::empty("test.registry", "test");
    s.nodes.push(n.clone());
    s.outputs.insert("clean".into(), 1.0);
    r.instantiate(&n, &s, Path::new("."), &[]).unwrap();
    let mut r = ProcessorRegistry::default();
    assert!(r
        .register(Registration::new(
            metadata,
            vec![ParameterSpec {
                key: "x",
                name: "X",
                min: 1.0,
                max: 0.0,
                default: 0.5,
                unit: ""
            }],
            |_| Ok(Box::new(Passthrough))
        ))
        .is_err());
    assert_eq!(r.entries().count(), 0);
}

#[cfg(feature = "ui")]
#[test]
fn optional_registered_editor_renders_and_reports_fallback() {
    let mut r = ProcessorRegistry::default();
    r.register(
        Registration::new(
            ProcessorMetadata {
                id: "test.editor",
                name: "Editor",
                category: "Test",
                version: 1,
                instrument: false,
            },
            vec![],
            |_| Err("unused factory".into()),
        )
        .with_editor(|ui, _| {
            ui.label("Registered custom editor");
            true
        }),
    )
    .unwrap();
    let ctx = soniclaw::ui::egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ctx| {
        soniclaw::ui::egui::CentralPanel::default().show(ctx, |ui| {
            assert_eq!(r.show_editor("missing", ui, &[]), None);
            assert_eq!(r.show_editor("test.editor", ui, &[]), Some(true));
        });
    });
    assert!(!output.shapes.is_empty());
    output.textures_delta.clear();
}
