//! Hardware-free walkthrough/audits and explicit trusted CLAP host.
mod precision_host;
use soniclaw::{
    dsp::voice_mod::*,
    midi::{file::*, performance::*},
    prelude::*,
    state::*,
};
use soniclaw_workbench::{app_descriptor, default_root, preset};
use std::{collections::BTreeMap, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn render_project(s: ProjectState, events: &[Event], frames: usize) -> Result<Vec<f32>> {
    let mut p = soniclaw_workbench::create_plugin(app_descriptor(&s), s, default_root())?;
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 257,
        channels: 2,
    })?;
    let mut data = vec![0.0; frames * 2];
    let mut scratch = Vec::with_capacity(1024);
    let mut at = 0;
    for b in data.chunks_mut(514) {
        scratch.clear();
        let end = at + b.len() / 2;
        for e in events.iter().filter(|e| e.frame >= at && e.frame < end) {
            scratch.push(Event {
                frame: e.frame - at,
                message: e.message,
            });
        }
        p.set_transport(Transport {
            sample_position: at as u64,
            tempo: 120.0,
            playing: true,
        })?;
        p.process(b, &scratch)?;
        at = end;
    }
    Ok(data)
}
fn audit(path: &Path) -> Result<()> {
    if path.exists() {
        return Err("output already exists".into());
    }
    let mut reports = BTreeMap::new();
    for (i, name) in [
        (1, "multiband"),
        (4, "poly_motion"),
        (5, "sampler"),
        (6, "registered_tape"),
    ] {
        let s = preset(i);
        let r = soniclaw::diagnostics::audit(
            || {
                let p = soniclaw_workbench::create_plugin(
                    app_descriptor(&s),
                    s.clone(),
                    default_root(),
                )
                .map_err(|_| Error::InvalidParameter)?;
                Ok(Box::new(soniclaw::project::ProjectProcessor::new(p)))
            },
            &Default::default(),
        )?;
        if !r.finite || r.allocation_clean != Some(true) {
            return Err(format!("audit failed for {name}").into());
        }
        let mut p =
            soniclaw_workbench::create_plugin(app_descriptor(&s), s.clone(), default_root())?;
        soniclaw::diagnostics::audit_state(&mut p)?;
        reports.insert(name, r);
    }
    std::fs::write(path, serde_json::to_string_pretty(&reports)?)?;
    println!("PASS 48 rate/block audits, heap and state transactions");
    Ok(())
}
fn demo(path: &Path) -> Result<()> {
    std::fs::create_dir(path)?;
    soniclaw_workbench::demonstration(&path.join("presets"))?;
    let mut report = serde_json::Map::new();
    let events = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id: 1,
            },
        },
        Event {
            frame: 8400,
            message: MidiMessage::NoteOnId {
                channel: 2,
                note: 67,
                velocity: 0.55,
                note_id: 2,
            },
        },
        Event {
            frame: 18000,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    channel: -1,
                    key: -1,
                    note_id: 2,
                },
                expression: NoteExpression::Tuning,
                value: 7.0,
            },
        },
        Event {
            frame: 22000,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    channel: -1,
                    key: -1,
                    note_id: 1,
                },
                expression: NoteExpression::Pressure,
                value: 1.0,
            },
        },
        Event {
            frame: 72000,
            message: MidiMessage::AllNotesOff,
        },
    ];
    for (name, modulated) in [("per-voice", true), ("voice-reference", false)] {
        let mut s = preset(4);
        if !modulated {
            s.nodes[0].voice_modulation = Some(VoiceModulationConfig::default());
        }
        soniclaw::io::write_wav(
            path.join(format!("{name}.wav")),
            48000,
            2,
            &render_project(s, &events, 96000)?,
        )?;
    }
    let cfg = PerformanceConfig {
        mode: ArpMode::UpDown,
        chord: vec![0, 4, 7, 11],
        swing: 0.2,
        humanize_ms: 3.0,
        velocity_jitter: 8,
        ..Default::default()
    };
    let mut p = MidiPerformance::new(cfg.clone(), 48000.0)?;
    let mut generated = vec![];
    let mut out = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; 1024];
    for at in (0..96000).step_by(257) {
        let frames = 257.min(96000 - at);
        let on = [Event {
            frame: 0,
            message: MidiMessage::NoteOn {
                channel: 1,
                note: 48,
                velocity: 100,
            },
        }];
        let n = p.process(
            frames,
            if at == 0 { &on } else { &[] },
            Transport {
                sample_position: at as u64,
                tempo: 120.0,
                playing: true,
            },
            &mut out,
        )?;
        generated.extend(out[..n].iter().map(|e| Event {
            frame: at + e.frame,
            message: e.message,
        }));
    }
    let clip = MidiClip {
        ppqn: 480,
        tempos: vec![TempoChange {
            tick: 0,
            micros_per_quarter: 500000,
        }],
        events: generated
            .iter()
            .map(|e| ClipEvent {
                tick: (e.frame as f64 / 50.0).round() as u64,
                message: e.message,
            })
            .collect(),
    };
    let midi = clip.write()?;
    std::fs::write(path.join("performance.mid"), &midi)?;
    assert_eq!(MidiClip::read(&midi)?.events.len(), generated.len());
    soniclaw::io::write_wav(
        path.join("performance.wav"),
        48000,
        2,
        &render_project(preset(4), &generated, 96000)?,
    )?;
    let mut s = preset(4);
    s.midi = Some(cfg);
    std::fs::write(path.join("performance-project.json"), s.to_json()?)?;
    report.insert("midi_events".into(), generated.len().into());
    let mut notes = vec![
        SequenceNote {
            beat: 0.13,
            length: 0.4,
            key: 60,
            velocity: 100,
            channel: 1,
        },
        SequenceNote {
            beat: 1.12,
            length: 0.4,
            key: 64,
            velocity: 90,
            channel: 1,
        },
    ];
    quantize_sequence(&mut notes, 0.25, 1.0)?;
    std::fs::write(
        path.join("quantized-sequence.json"),
        serde_json::to_string_pretty(&notes)?,
    )?;
    assert_eq!(sequence_events(&notes, 48000.0, 120.0)?.len(), 4);
    let mut s = preset(5);
    s.zones.clear();
    let mut hits = vec![];
    for i in 0..4 {
        let mut z = SampleZone::new("grain-source");
        z.keys = [36 + i as u8; 2];
        z.start_frame = i * 24000;
        z.end_frame = Some((i + 1) * 24000);
        s.zones.push(z);
        hits.push(Event {
            frame: i * 24000,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 36 + i as u8,
                velocity: 100,
            },
        });
    }
    soniclaw::io::write_wav(
        path.join("sample-slices.wav"),
        48000,
        2,
        &render_project(s.clone(), &hits, 96000)?,
    )?;
    std::fs::write(path.join("slice-project.json"), s.to_json()?)?;
    std::fs::create_dir(path.join("assets"))?;
    for name in ["grain-source.wav", "stereo-ir.wav"] {
        std::fs::copy(
            default_root().join("assets").join(name),
            path.join("assets").join(name),
        )?;
    }
    let state = |gain: f32| {
        let mut s = ProjectState::empty(soniclaw_workbench::APP_ID, "Live graph");
        let mut n = RackNode::new("gain", ProcessorKind::Gain);
        n.input_gain = 1.0;
        n.settings.insert("gain".into(), gain);
        s.nodes.push(n);
        s.outputs.insert("gain".into(), 1.0);
        s
    };
    let processor = |gain| -> Result<Box<dyn AudioProcessor>> {
        let s = state(gain);
        let p = soniclaw_workbench::create_plugin(app_descriptor(&s), s, default_root())?;
        Ok(Box::new(soniclaw::project::ProjectProcessor::new(p)))
    };
    let (mut ctl, mut graph) = soniclaw::graph::live::graph_pair(
        processor(1.0)?,
        ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 256,
            channels: 2,
        },
        64,
        512,
    )?;
    let mut data = vec![0.2; 48000];
    let mut count = 0;
    for (i, b) in data.chunks_mut(512).enumerate() {
        if i == 16 {
            ctl.replace(processor(0.25)?)?;
        }
        let a = allocation_counter::measure(|| {
            graph.process(
                &mut AudioBuffer::new(b, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: (i * 256) as u64,
                        tempo: 120.0,
                        playing: true,
                    },
                },
            )
        });
        count += a.count_total;
        assert_eq!(a.count_current, 0);
    }
    let retired = ctl.collect_retired();
    assert_eq!(count, 0);
    assert_eq!(retired, 1);
    soniclaw::io::write_wav(path.join("live-graph.wav"), 48000, 2, &data)?;
    report.insert("live_graph".into(),serde_json::json!({"latency_frames":graph.latency_frames(),"retired_on_control":retired,"callback_allocations":count}));
    std::fs::write(
        path.join("sdk-report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("PASS voices, slices, MIDI/SMF, quantization and live graph");
    Ok(())
}
fn host(path: &Path, out: &Path) -> Result<()> {
    std::fs::create_dir(out)?;
    // SAFETY: explicit CLI choice of trusted native CLAP library.
    let products = {
        let l = unsafe { soniclaw::host::ClapLibrary::load(path) }.map_err(|e| e.to_string())?;
        l.plugins().to_vec()
    };
    let mut report = vec![];
    for (i, info) in products.iter().enumerate() {
        // SAFETY: same explicitly selected trusted library; scopes join before teardown.
        let first = unsafe { soniclaw_workbench::render_hosted(path, &info.id, &[], None) }
            .map_err(|e| e.to_string())?;
        // SAFETY: repeat in a fresh instance to verify restoration.
        let second =
            unsafe { soniclaw_workbench::render_hosted(path, &info.id, &[], Some(&first.state)) }
                .map_err(|e| e.to_string())?;
        let stable = first.parameters.len() == second.parameters.len()
            && first
                .parameters
                .iter()
                .zip(&second.parameters)
                .all(|(a, b)| a.id == b.id && (a.value - b.value).abs() < 1e-6);
        assert!(stable);
        let channels = first.audio.len() / 96000;
        let peak = first.audio.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
        assert!(peak > 1e-6);
        soniclaw::io::write_wav(
            out.join(format!("product-{i}.wav")),
            48000,
            channels as u16,
            &first.audio,
        )?;
        std::fs::write(out.join(format!("product-{i}.state")), &first.state)?;
        report.push(serde_json::json!({"id":info.id,"channels":channels,"latency_frames":first.latency,"peak":peak,"state_bytes":first.state.len(),"parameter_count":first.parameters.len(),"fresh_instance_restore":stable,"output_events":first.output_events,"debug_processing_ms":first.processing_ms}));
        println!("PASS hosted {}", info.id);
    }
    std::fs::write(
        out.join("host-report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(())
}
fn host_contract(path: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        return Err("output exists".into());
    }
    // SAFETY: explicit first-party independent contract fixture.
    let lib = unsafe { soniclaw::host::ClapLibrary::load(path) }.map_err(|e| e.to_string())?;
    let mut p = lib
        .instantiate("org.soniclaw.host-fixture")
        .map_err(|e| e.to_string())?;
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 64,
        channels: 2,
    })
    .map_err(|e| e.to_string())?;
    let echoed = p
        .run_audio(|a| {
            let mut data = [0.2_f32; 128];
            let e = [
                Event {
                    frame: 3,
                    message: MidiMessage::NoteOnId {
                        channel: 2,
                        note: 60,
                        velocity: 0.8,
                        note_id: 7,
                    },
                },
                Event {
                    frame: 14,
                    message: MidiMessage::NoteOffId {
                        target: NoteTarget {
                            channel: 2,
                            key: 60,
                            note_id: 7,
                        },
                    },
                },
                Event {
                    frame: 31,
                    message: MidiMessage::AllNotesOff,
                },
            ];
            let controls = [soniclaw::host::ParameterEvent {
                frame: 16,
                id: 9,
                value: 0.25,
            }];
            let alloc = allocation_counter::measure(|| {
                a.process_block(
                    &mut data,
                    &e,
                    &controls,
                    Transport {
                        playing: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            });
            assert_eq!(alloc.count_total, 0);
            assert_eq!(alloc.count_current, 0);
            assert!(data[..32].iter().all(|x| (*x - 0.1).abs() < 1e-6));
            assert!(data[32..].iter().all(|x| (*x - 0.05).abs() < 1e-6));
            assert_eq!(a.output_events().len(), 19);
            let mut unchanged = [0.2; 128];
            assert!(a
                .process_block(
                    &mut unchanged,
                    &[],
                    &[soniclaw::host::ParameterEvent {
                        frame: 99,
                        id: 9,
                        value: 0.9
                    }],
                    Transport {
                        playing: true,
                        ..Default::default()
                    }
                )
                .is_err());
            assert_eq!(unchanged, [0.2; 128]);
            a.output_events().len()
        })
        .map_err(|e| e.to_string())?;
    let flags = p.service_main_thread();
    assert_ne!(flags & 2, 0);
    p.deactivate();
    let state = p.save_state().map_err(|e| e.to_string())?;
    assert_eq!(state.len(), 16);
    assert_eq!(u32::from_le_bytes(state[12..16].try_into().unwrap()), 1);
    p.load_state(&state).map_err(|e| e.to_string())?;
    assert!((p.parameters()[0].value - 0.25).abs() < 1e-9);
    std::fs::write(
        out,
        serde_json::to_string_pretty(
            &serde_json::json!({"fixture":"independent MIDI-only CLAP","echo_events":echoed,"host_callback_allocations":0,"timed_gain":true,"bounds_transaction":true,"main_callback":true,"state_roundtrip":true,"separate_main_audio_thread_contract":true}),
        )?,
    )?;
    println!("PASS independent host: threads, MIDI fallback/echo, timing, heap, bounds, state");
    Ok(())
}
fn workflow_demo(path: &Path) -> Result<()> {
    use soniclaw::project_loading::*;
    use soniclaw::project_storage::*;
    std::fs::create_dir(path)?;
    let state = preset(5);
    let bundle = path.join("sampler.soniclaw");
    pack_bundle(&state, &default_root(), &bundle)?;
    let dest = path.join("unpacked");
    let portable = unpack_bundle(&bundle, &dest)?;
    let store_path = path.join("recovery");
    let autosave = AutosaveWorker::new(RecoveryStore::new(&store_path, 5)?);
    autosave.try_checkpoint(portable.clone(), dest.clone())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(result) = autosave.try_poll() {
            result?;
            break;
        }
        if std::time::Instant::now() > deadline {
            return Err("autosave timeout".into());
        }
        std::thread::yield_now();
    }
    let mut loader = AsyncProjectLoader::with_factory(|s, r| {
        soniclaw_workbench::create_plugin(app_descriptor(s), s.clone(), r)
    });
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    };
    let generation = loader.submit(
        LoadRequest::Recovery {
            directory: store_path,
            product: state.product_id.clone(),
        },
        Some(spec),
    )?;
    let loaded = loop {
        if let Some(outcome) = loader.try_poll() {
            assert_eq!(outcome.generation, generation);
            break outcome.result?;
        }
        if std::time::Instant::now() > deadline {
            return Err("load timeout".into());
        }
        std::thread::yield_now();
    };
    assert_eq!(loaded.state(), &portable);
    let mut processor = loaded.into_processor();
    processor.prepare(spec)?;
    let mut audio = vec![0.0; 48000 * 2];
    let notes = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    }];
    for (i, b) in audio.chunks_mut(512).enumerate() {
        processor.process(
            &mut AudioBuffer::new(b, 2)?,
            &ProcessContext {
                events: if i == 0 { &notes } else { &[] },
                transport: Transport {
                    sample_position: (i * 256) as u64,
                    tempo: 120.0,
                    playing: true,
                },
            },
        );
    }
    if !audio.iter().all(|x| x.is_finite()) || !audio.iter().any(|x| x.abs() > 0.001) {
        return Err("recovered render failed".into());
    }
    soniclaw::io::write_wav(path.join("recovered-sampler.wav"), 48000, 2, &audio)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"bundle_roundtrip":true,"background_autosave":true,"background_recovery_load":true,"prepared_render":true,"asset_count":portable.assets.len(),"frames":48000}),
        )?,
    )?;
    println!("PASS bundle, background autosave, recovery load and prepared sampler render");
    Ok(())
}
fn registry_demo(path: &Path) -> Result<()> {
    std::fs::create_dir(path)?;
    let mut state = preset(6);
    state.nodes[0].settings.insert("drive".into(), 4.0);
    state.nodes[0].settings.insert("mix".into(), 1.0);
    let input: Vec<f32> = (0..48000)
        .flat_map(|i| {
            let t = i as f32 / 48000.0;
            let x = (std::f32::consts::TAU * 220.0 * t).sin() * 0.2
                + (std::f32::consts::TAU * 3300.0 * t).sin() * 0.03;
            [x, x]
        })
        .collect();
    let mut p = soniclaw_workbench::create_plugin(app_descriptor(&state), state.clone(), path)?;
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    })?;
    let mut rendered = input.clone();
    for block in rendered.chunks_mut(512) {
        p.process(block, &[])?;
    }
    if !rendered.iter().all(|x| x.is_finite())
        || !rendered
            .iter()
            .zip(&input)
            .any(|(a, b)| (a - b).abs() > 0.01)
    {
        return Err("registered DSP render failed".into());
    }
    p.deactivate();
    let handles = p.parameters().to_vec();
    let mut legacy = state.clone();
    let custom = legacy.nodes[0]
        .custom
        .as_mut()
        .ok_or("missing custom state")?;
    custom.version = 1;
    custom.data = serde_json::json!({"bias":0.1});
    p.load_state(&legacy.to_json()?)?;
    if !handles
        .iter()
        .zip(p.parameters())
        .all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
    {
        return Err("host parameter identity changed".into());
    }
    let saved = ProjectState::from_json(&p.save_state()?)?;
    if saved.nodes[0]
        .custom
        .as_ref()
        .ok_or("missing migrated state")?
        .version
        != 2
    {
        return Err("migration did not persist".into());
    }
    std::fs::write(path.join("migrated-project.json"), saved.to_json()?)?;
    soniclaw::project_storage::pack_bundle(&saved, path, &path.join("tape-colour.soniclaw"))?;
    let root = path.join("unpacked");
    let unpacked =
        soniclaw::project_storage::unpack_bundle(&path.join("tape-colour.soniclaw"), &root)?;
    let _ = soniclaw_workbench::create_plugin(app_descriptor(&unpacked), unpacked, &root)?;
    soniclaw::io::write_wav(path.join("dry.wav"), 48000, 2, &input)?;
    soniclaw::io::write_wav(path.join("tape.wav"), 48000, 2, &rendered)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"registered_crate":"soniclaw-example-processors","processor_id":"org.soniclaw.example.tape","parameters":4,"migration":"1 -> 2","stable_parameter_handles":true,"bundle_reload":true,"frames":48000}),
        )?,
    )?;
    println!("PASS registered DSP audio, state migration, stable controls and portable reload");
    Ok(())
}
fn catalog_demo(path: &Path) -> Result<()> {
    std::fs::create_dir(path)?;
    let mut c = soniclaw::catalog::PresetCatalog::open(path.join("catalog"))?;
    let s = preset(6);
    let id = c.insert(
        &s,
        &default_root(),
        vec!["Saturation".into()],
        "Factory".into(),
    )?;
    c.edit_metadata(
        &id,
        s.name.clone(),
        vec!["Saturation".into()],
        "Factory".into(),
        true,
    )?;
    c.insert(
        &preset(4),
        &default_root(),
        vec!["Synth".into(), "Pad".into()],
        "Instruments".into(),
    )?;
    assert_eq!(c.search("saturation", Some("Factory"), true).len(), 1);
    assert!(c.missing_assets(&id)?.is_empty());
    c.export_preset(&id, &path.join("selected.soniclaw"))?;
    let preview = soniclaw_workbench::render(s, default_root(), 0.5)?;
    soniclaw::io::write_wav(path.join("preview.wav"), 48000, 2, &preview.audio)?;
    drop(c);
    let c = soniclaw::catalog::PresetCatalog::open(path.join("catalog"))?;
    assert_eq!(c.entries().len(), 2);
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"entries":2,"search":true,"favourites":true,"banks":true,"preview":true,"bundle_export":true,"reopen":true}),
        )?,
    )?;
    println!("PASS browser catalog, metadata, search, bank, favourite, preview and export");
    Ok(())
}
fn sampler_demo(path: &Path) -> Result<()> {
    std::fs::create_dir(path)?;
    for (name, bright) in [
        ("soft.wav", false),
        ("hard.wav", true),
        ("release.wav", false),
    ] {
        let frames = if name == "release.wav" { 4800 } else { 48000 };
        let data: Vec<f32> = (0..frames)
            .map(|i| {
                let t = i as f32 / 48000.0;
                let x = (std::f32::consts::TAU * 261.626 * t).sin();
                let x = if bright { (x * 3.0).tanh() } else { x };
                x * 0.25 * (-t * if name == "release.wav" { 40.0 } else { 1.5 }).exp()
            })
            .collect();
        soniclaw::io::write_wav(path.join(name), 48000, 1, &data)?;
    }
    let sfz="<global> key=60 pitch_keycenter=60\n<region> sample=soft.wav xfout_lovel=32 xfout_hivel=96\n<region> sample=hard.wav xfin_lovel=32 xfin_hivel=96\n<region> sample=release.wav trigger=release volume=-6";
    std::fs::write(path.join("instrument.sfz"), sfz)?;
    let (state, root) =
        soniclaw::sampling::import_sfz(&path.join("instrument.sfz"), soniclaw_workbench::APP_ID)?;
    let preview = soniclaw_workbench::render(state.clone(), root, 1.5)?;
    soniclaw::io::write_wav(path.join("instrument.wav"), 48000, 2, &preview.audio)?;
    std::fs::write(path.join("project.json"), state.to_json()?)?;
    let mut samples = vec![0.0; 48000];
    for at in [0, 12000, 24000, 36000] {
        for (i, x) in samples[at..at + 1000].iter_mut().enumerate() {
            *x = (i as f32 * 0.3).sin() * (-(i as f32) / 200.0).exp();
        }
    }
    let audio = soniclaw::io::AudioFile {
        sample_rate: 48000,
        channels: 1,
        samples,
    };
    let markers = soniclaw::sampling::detect_transients(
        &audio,
        soniclaw::sampling::TransientOptions::default(),
    )?;
    soniclaw::io::write_wav(path.join("transients.wav"), 48000, 1, &audio.samples)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"sfz_regions":state.zones.len(),"velocity_crossfades":true,"release_region":true,"slice_markers":markers,"frames":72000}),
        )?,
    )?;
    println!(
        "PASS SFZ import, velocity/release playback, transient detection and project persistence"
    );
    Ok(())
}
#[cfg(feature = "device-tools")]
fn duplex_demo(path: &Path) -> Result<()> {
    use soniclaw::device::*;
    std::fs::create_dir(path)?;
    let selector = DeviceSelector {
        name: "Synthetic interface".into(),
        occurrence: 0,
    };
    let config = OutputRange {
        channels: 2,
        min_rate: 44100,
        max_rate: 96000,
        buffer_range: Some((64, 1024)),
        format: OutputFormat::F32,
    };
    let device = OutputDevice {
        selector: selector.clone(),
        configs: vec![config],
        default_rate: 48000,
        default_channels: 2,
    };
    let catalog = OutputInventory {
        host: "Synthetic".into(),
        hosts: vec!["Synthetic".into()],
        inputs: vec![device.clone()],
        outputs: vec![device],
        default_input: Some(selector.clone()),
        default_output: Some(selector),
        warnings: vec![],
    };
    let prefs = DevicePreferences {
        duplex: Some(Default::default()),
        buffer_frames: Some(256),
        ..Default::default()
    };
    let plan = negotiate_output(&prefs, &catalog)?;
    let s = preset(6);
    let mut plugin = soniclaw_workbench::create_plugin(app_descriptor(&s), s, default_root())?;
    plugin.activate(plan.spec())?;
    let (mut recorder, mut tap) = soniclaw::record::Recorder::new(128);
    let output = path.join("processed-take.wav");
    recorder
        .start(&output, 48000, soniclaw::record::RecordSource::Processed)
        .map_err(|e| e.to_string())?;
    let mut dry = [0.0; 512];
    let mut wet = dry;
    for block in 0..100 {
        for (i, x) in dry.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            x.fill(((block * 256 + i) as f32 * 0.03).sin() * 0.2);
        }
        wet.copy_from_slice(&dry);
        plugin.process(&mut wet, &[])?;
        tap.capture_pair(&dry, &wet);
    }
    recorder.stop().map_err(|e| e.to_string())?;
    let summary = recorder.summary();
    let audio = soniclaw::io::read_wav(&output)?;
    if audio.frames() != 25600 || summary.write_errors != 0 || summary.dropped_frames != 0 {
        return Err("synthetic take was not finalized cleanly".into());
    }
    prefs.save(path.join("audio-settings.json"))?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"synthetic_only":true,"plan":plan,"frames":audio.frames(),"written_frames":summary.written_frames,"dropped_frames":summary.dropped_frames,"physical_device_opened":false}),
        )?,
    )?;
    println!("PASS shared-rate duplex plan, processed recording, finalized WAV and settings");
    Ok(())
}
fn pack(project: &Path, destination: &Path) -> Result<()> {
    let state = ProjectState::from_json(&std::fs::read_to_string(project)?)?;
    soniclaw::project_storage::pack_bundle(
        &state,
        project
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
        destination,
    )
}
fn automation_demo(path: &Path) -> Result<()> {
    use soniclaw::automation::*;
    let mut s = ProjectState::empty("org.soniclaw.workbench", "Automation fade");
    let mut n = RackNode::new("gain", ProcessorKind::Gain);
    n.input_gain = 1.0;
    s.nodes.push(n);
    s.outputs.insert("gain".into(), 1.0);
    let p = soniclaw_workbench::create_plugin(app_descriptor(&s), s.clone(), default_root())?;
    let id = p.parameters()[0].info().id.to_owned();
    let mut doc = Document {
        loop_range: Some([0.0, 4.0]),
        lanes: vec![Lane {
            parameter: id,
            enabled: true,
            mode: Mode::Latch,
            points: vec![
                Point {
                    beat: 0.0,
                    value: 1.0,
                    curve: Curve::Smooth,
                },
                Point {
                    beat: 4.0,
                    value: 0.0,
                    curve: Curve::Linear,
                },
            ],
        }],
    };
    let mut recorder = Recorder::default();
    recorder.begin(0.25)?;
    recorder.tick(&mut doc, 0, 1.0, 0.25)?;
    recorder.end();
    recorder.tick(&mut doc, 0, 1.5, 0.25)?;
    recorder.stop();
    s.automation = Some(doc);
    s.validate(&s.product_id)?;
    let mut p = soniclaw_workbench::create_plugin(app_descriptor(&s), s.clone(), default_root())?;
    p.activate(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 257,
        channels: 2,
    })?;
    let mut audio = vec![0.0; 96000 * 2];
    let mut at = 0;
    for b in audio.chunks_mut(514) {
        for (i, frame) in b.chunks_mut(2).enumerate() {
            frame.fill(
                (std::f64::consts::TAU * 220.0 * (at + i) as f64 / 48000.0).sin() as f32 * 0.1,
            );
        }
        p.set_transport(Transport {
            sample_position: at as u64,
            tempo: 120.0,
            playing: true,
        })?;
        p.process(b, &[])?;
        at += b.len() / 2;
    }
    if audio.iter().any(|x| !x.is_finite()) {
        return Err("nonfinite automation render".into());
    }
    std::fs::create_dir(path)?;
    std::fs::write(path.join("project.json"), s.to_json()?)?;
    soniclaw::io::write_wav(path.join("automation.wav"), 48000, 2, &audio)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_string_pretty(
            &serde_json::json!({"frames":96000,"recorded_latch_points":2,"sample_rate":48000,"finite":true}),
        )?,
    )?;
    println!("PASS automation project, latch recording and 96000-frame render");
    Ok(())
}
fn live_rack_demo(library: &Path, path: &Path) -> Result<()> {
    use soniclaw::host::live::*;
    let entries = vec![
        RackEntry {
            path: library.canonicalize()?,
            plugin_id: "org.soniclaw.host-fixture".into()
        };
        2
    ];
    // SAFETY: explicit command selects our independently built ABI fixture.
    let instances = unsafe { instantiate(&entries) }.map_err(|e| e.to_string())?;
    let (mut rack, mut processor) =
        LiveRack::start(instances, 48000.0).map_err(|e| e.to_string())?;
    processor.prepare(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 257,
        channels: 2,
    })?;
    rack.show_editor(0).map_err(|e| e.to_string())?;
    rack.show_editor(0).map_err(|e| e.to_string())?;
    rack.close_editor(0);
    rack.show_editor(0).map_err(|e| e.to_string())?;
    rack.set_parameter(1, 9, 0.5).map_err(|e| e.to_string())?;
    if rack.set_parameter(1, 999, 0.5).is_ok() || rack.set_parameter(0, 9, f64::NAN).is_ok() {
        return Err("invalid parameter accepted".into());
    }
    let mut audio = Vec::new();
    let mut at = 0;
    let mut callbacks = 0;
    let mut observed = 0;
    // Synthetic clock: wait only on this control thread between callback invocations.
    // The callback itself neither waits nor allocates; block-count polling makes the ABI check reproducible.
    for block in 0..40 {
        let n = if block % 2 == 0 { 255 } else { 257 };
        let mut data = vec![1.0; n * 2];
        processor.process(
            &mut AudioBuffer::new(&mut data, 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: at,
                    tempo: 120.0,
                    playing: true,
                },
            },
        );
        at += n as u64;
        audio.extend(data);
        let expected = at / 256;
        let started = std::time::Instant::now();
        while rack.stats().snapshot().blocks < expected
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            callbacks += rack.poll().iter().filter(|f| **f & 2 != 0).count();
            observed += rack.drain_events().len();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        callbacks += rack.poll().iter().filter(|f| **f & 2 != 0).count();
        observed += rack.drain_events().len();
    }
    let stats = rack.stats().snapshot();
    rack.close_editor(0);
    rack.stop();
    if stats.process_errors != 0
        || stats.input_drops != 0
        || stats.output_drops != 0
        || stats.underruns != 0
        || callbacks == 0
        || observed == 0
    {
        return Err(format!(
            "live fixture contract failed: {stats:?}, callbacks {callbacks}, events {observed}"
        )
        .into());
    }
    if audio[..BRIDGE_LATENCY * 2].iter().any(|x| *x != 0.0)
        || audio[BRIDGE_LATENCY * 2..]
            .iter()
            .any(|x| (*x - 0.25).abs() > 0.00001)
    {
        return Err("serial gain/delay mismatch".into());
    }
    std::fs::create_dir(path)?;
    std::fs::write(path.join("rack.json"), serde_json::to_vec_pretty(&entries)?)?;
    soniclaw::io::write_wav(path.join("serial-gain.wav"), 48000, 2, &audio)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"stats":stats,"main_callbacks":callbacks,"output_events":observed,"floating_editor_abi":true,"actual_editor_window":false,"synthetic_clock":true,"serial_gain":0.25}),
        )?,
    )?;
    println!("PASS two-node live rack, exact bridge delay, output events, main callbacks and floating editor ABI");
    Ok(())
}
fn probe_trusted(library: &Path, report: &Path) -> Result<()> {
    if report.exists() {
        return Err("probe output already exists".into());
    }
    // SAFETY: explicit CLI selects trusted code; caller should isolate this process.
    let lib = unsafe { soniclaw::host::ClapLibrary::load(library) }.map_err(|e| e.to_string())?;
    let mut products = vec![];
    for info in lib.plugins() {
        let p = lib.instantiate(&info.id).map_err(|e| e.to_string())?;
        products.push(serde_json::json!({"product":info,"parameters":p.parameters(),"main_channels":p.main_channels(),"scope":"instantiate/metadata/destroy only; no audio or editor"}));
    }
    std::fs::write(
        report,
        serde_json::to_vec_pretty(
            &serde_json::json!({"library":library.canonicalize()?,"products":products}),
        )?,
    )?;
    println!("PASS isolated metadata probe: {} products", products.len());
    Ok(())
}
fn direct_demo(library: &Path, path: &Path) -> Result<()> {
    use soniclaw::host::live::*;
    let entries = vec![
        RackEntry {
            path: library.canonicalize()?,
            plugin_id: "org.soniclaw.host-fixture".into()
        };
        2
    ];
    // SAFETY: explicit command selects the independent trusted test fixture.
    let instances = unsafe { instantiate(&entries) }.map_err(|e| e.to_string())?;
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        channels: 2,
        max_block_frames: 512,
    };
    let (mut rack, mut processor) =
        LiveRack::start_direct(instances, spec).map_err(|e| e.to_string())?;
    processor.prepare(spec)?;
    rack.set_parameter(0, 9, 0.8).map_err(|e| e.to_string())?;
    let mut audio = vec![];
    let result = std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            let mut position = 0;
            let mut rendered = vec![];
            for frames in [1, 7, 64, 255, 257, 512] {
                let mut data = vec![1.0; frames * 2];
                let a = allocation_counter::measure(|| {
                    processor.process(
                        &mut AudioBuffer::new(&mut data, 2).unwrap(),
                        &ProcessContext {
                            events: &[],
                            transport: Transport {
                                sample_position: position,
                                tempo: 120.0,
                                playing: true,
                            },
                        },
                    )
                });
                if a.count_total != 0 || data.iter().any(|x| (*x - 0.4).abs() > 1e-6) {
                    return Err("direct output/heap contract failed");
                }
                position += frames as u64;
                rendered.extend(data);
            }
            Ok(rendered)
        });
        while !handle.is_finished() {
            rack.poll();
            std::thread::yield_now();
        }
        handle.join().map_err(|_| "audio thread panicked")?
    });
    audio.extend(result.map_err(|e| e.to_string())?);
    let stats = rack.stats().snapshot();
    if stats.bridge_latency_frames != 0 || stats.process_errors != 0 {
        return Err("direct diagnostics failed".into());
    }
    rack.stop();
    drop(rack);
    let silent = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let mut d = [1.0; 8];
                processor.process(
                    &mut AudioBuffer::new(&mut d, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
                d
            })
            .join()
            .unwrap()
    });
    if silent != [0.0; 8] {
        return Err("callback survived owner unload unsafely".into());
    }
    let lag = soniclaw::host::direct::impulse_latency(&[1.0, 0.0], &[0.0, 0.0, 1.0], 16)?;
    std::fs::create_dir(path)?;
    soniclaw::io::write_wav(path.join("direct.wav"), 48000, 2, &audio)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"stats":stats,"first_sample":audio[0],"owner_unload_silence":true,"allocation_free":true,"synthetic_impulse_lag":lag,"hardware_latency_measured":false}),
        )?,
    )?;
    println!("PASS direct callback: zero bridge delay, exact serial gain, no heap, safe owner-first destruction");
    Ok(())
}
fn session_demo(library: &Path, path: &Path) -> Result<()> {
    use soniclaw::host::{live::*, session::*};
    let entries = vec![
        RackEntry {
            path: library.canonicalize()?,
            plugin_id: "org.soniclaw.host-fixture".into()
        };
        2
    ];
    // SAFETY: explicit command selects the independent trusted native fixture.
    let instances = unsafe { instantiate(&entries) }.map_err(|e| e.to_string())?;
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        channels: 2,
        max_block_frames: 32,
    };
    let (mut owner, mut p) = LiveRack::start_direct(instances, spec).map_err(|e| e.to_string())?;
    owner.set_parameter(0, 9, 0.8).map_err(|e| e.to_string())?;
    owner.set_parameter(1, 9, 0.7).map_err(|e| e.to_string())?;
    owner.set_bypass(0, true).map_err(|e| e.to_string())?;
    let first = std::thread::scope(|s| {
        s.spawn(|| {
            let mut d = [1.0; 16];
            p.process(
                &mut AudioBuffer::new(&mut d, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: 0,
                        tempo: 120.0,
                        playing: true,
                    },
                },
            );
            d
        })
        .join()
        .unwrap()
    });
    if first.iter().any(|x| (*x - 0.7).abs() > 1e-6) {
        return Err("live bypass failure".into());
    }
    let mut document =
        Session::capture(&mut owner, &entries, "Saved native sounds").map_err(|e| e.to_string())?;
    drop(owner);
    drop(p);
    if document.slots[0].parameters.get(&9) != Some(&0.8) {
        return Err("capture lost live values".into());
    }
    // Exercise parameter-only fallback alongside opaque state restoration.
    document.slots[1].state_hex = None;
    document.slots[1].state_sha256 = None;
    let mut missing = document.slots[0].clone();
    missing.id = "missing-slot".into();
    missing.entry.path = path.join("not-installed.clap");
    document.slots.insert(1, missing);
    std::fs::create_dir(path)?;
    document
        .write(&path.join("session.json"))
        .map_err(|e| e.to_string())?;
    let loaded = Session::read(&path.join("session.json")).map_err(|e| e.to_string())?;
    let mut resolved =
        unsafe { loaded.resolve(MissingPolicy::Placeholder) }.map_err(|e| e.to_string())?;
    if resolved.missing.len() != 1 || resolved.slot_indices != [0, 2] {
        return Err("missing-slot mapping failed".into());
    }
    let instances = resolved.take_instances();
    let (mut owner, mut p) = LiveRack::start_direct(instances, spec).map_err(|e| e.to_string())?;
    resolved.configure(&mut owner).map_err(|e| e.to_string())?;
    let output = std::thread::scope(|s| {
        s.spawn(|| {
            let mut d = [1.0; 16];
            p.process(
                &mut AudioBuffer::new(&mut d, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: 0,
                        tempo: 120.0,
                        playing: true,
                    },
                },
            );
            d
        })
        .join()
        .unwrap()
    });
    if output.iter().any(|x| (*x - 0.7).abs() > 1e-6) {
        return Err("saved opaque/fallback/bypass restore changed sound".into());
    }
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema":1,"slots":3,"available":2,"missing":resolved.missing,"restored_gain":output[0],"opaque_and_parameter_state":true,"bypass_restored":owner.is_bypassed(0)}),
        )?,
    )?;
    println!("PASS full rack session: opaque state, parameter fallback, bypass and retained missing slot");
    Ok(())
}
fn bus_demo(library: &Path, path: &Path) -> Result<()> {
    // SAFETY: explicit CLI selects the framework's trusted reference exports.
    let lib = unsafe { soniclaw::host::ClapLibrary::load(library) }.map_err(|e| e.to_string())?;
    let mut instance = lib
        .instantiate("org.sonora.ducker")
        .map_err(|e| e.to_string())?;
    let (inputs, outputs) = instance.audio_ports();
    if inputs.len() != 2 || outputs.len() != 1 {
        return Err("reference sidechain ports missing".into());
    }
    let ports = serde_json::json!({"inputs":inputs,"outputs":outputs});
    let result = instance
        .with_bus_audio(48000.0, 256, |audio| {
            let main = [0.5; 512];
            let high = [1.0; 512];
            let low = [0.0; 512];
            let mut output = [0.0; 512];
            let mut at = 0;
            let mut count = 0;
            for _ in 0..128 {
                let a = allocation_counter::measure(|| {
                    audio
                        .process(
                            &[&main, &high],
                            &mut [&mut output],
                            &[],
                            &[],
                            Transport {
                                sample_position: at,
                                tempo: 120.0,
                                playing: true,
                            },
                        )
                        .unwrap()
                });
                count += a.count_total;
                at += 256;
            }
            let ducked = output[511];
            for _ in 0..1024 {
                audio
                    .process(
                        &[&main, &low],
                        &mut [&mut output],
                        &[],
                        &[],
                        Transport {
                            sample_position: at,
                            tempo: 120.0,
                            playing: true,
                        },
                    )
                    .unwrap();
                at += 256;
            }
            let released = output[511];
            let invalid = audio
                .process(&[&main], &mut [&mut output], &[], &[], Transport::default())
                .is_err()
                && output.iter().all(|x| *x == 0.0);
            (ducked, released, count, invalid)
        })
        .map_err(|e| e.to_string())?;
    if result.0 >= 0.2 || result.1 < 0.49 || result.2 != 0 || !result.3 {
        return Err(format!("sidechain contract failed: {result:?}").into());
    }
    // Also exercise the framework's explicit mono -> stereo/aux channel map.
    use soniclaw::bus::flex::*;
    let layout = Layout {
        inputs: vec![Bus {
            id: "mono".into(),
            name: "Mono".into(),
            channels: 1,
            role: Role::Main,
        }],
        outputs: vec![
            Bus {
                id: "main".into(),
                name: "Stereo".into(),
                channels: 2,
                role: Role::Main,
            },
            Bus {
                id: "aux".into(),
                name: "Aux".into(),
                channels: 1,
                role: Role::Auxiliary,
            },
        ],
    };
    let mut patch = PatchBay::new(
        layout.clone(),
        vec![
            Route {
                input_bus: 0,
                input_channel: 0,
                output_bus: 0,
                output_channel: 0,
                gain: 1.0,
            },
            Route {
                input_bus: 0,
                input_channel: 0,
                output_bus: 0,
                output_channel: 1,
                gain: 1.0,
            },
            Route {
                input_bus: 0,
                input_channel: 0,
                output_bus: 1,
                output_channel: 0,
                gain: 0.5,
            },
        ],
        256,
    )?;
    let (mut stereo, mut aux) = ([0.0; 4], [0.0; 2]);
    patch.process(&[&[1.0, 0.5]], &mut [&mut stereo, &mut aux])?;
    if stereo != [1., 1., 0.5, 0.5] || aux != [0.5, 0.25] {
        return Err("auxiliary channel map failed".into());
    }
    std::fs::create_dir(path)?;
    std::fs::write(
        path.join("layout.json"),
        serde_json::to_vec_pretty(&layout)?,
    )?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"ports":ports,"ducked":result.0,"released":result.1,"callback_allocations":result.2,"invalid_buffers_silent":result.3,"aux_routing":true}),
        )?,
    )?;
    println!("PASS named buses, explicit aux routing and actual hosted sidechain processing");
    Ok(())
}
#[cfg(feature = "format-tools")]
fn format_demo(path: &Path) -> Result<()> {
    use soniclaw::formats::*;
    let manager = FormatManager::native_default().map_err(|e| e.to_string())?;
    let audio: Vec<_> = (0..512)
        .flat_map(|i| {
            let v = (std::f32::consts::TAU * 220.0 * i as f32 / 48000.0).sin() * 0.5;
            [v, -v]
        })
        .collect();
    std::fs::create_dir(path)?;
    let mut reports = vec![];
    for (format, name) in [
        (AudioFormat::Wav, "roundtrip.wav"),
        (AudioFormat::Aiff, "roundtrip.aiff"),
        (AudioFormat::Flac, "roundtrip.flac"),
    ] {
        let file = path.join(name);
        let mut writer = manager
            .create(&file, format, Encoding::Pcm24, 48000, 2)
            .map_err(|e| e.to_string())?;
        writer
            .set_metadata("title", "Soniclaw codec round trip")
            .map_err(|e| e.to_string())?;
        writer
            .write_frames(&audio[..510])
            .map_err(|e| e.to_string())?;
        writer
            .write_frames(&audio[510..])
            .map_err(|e| e.to_string())?;
        let frames = writer.finish().map_err(|e| e.to_string())?;
        let mut reader = manager.open(&file).map_err(|e| e.to_string())?;
        if frames != 512
            || reader.info().frames != 512
            || reader.info().channels != 2
            || reader.info().metadata.get("title").map(String::as_str)
                != Some("Soniclaw codec round trip")
        {
            return Err("codec metadata/frame round trip failed".into());
        }
        reader.seek(123).map_err(|e| e.to_string())?;
        let mut region = [0.0; 14];
        if reader.read_frames(&mut region).map_err(|e| e.to_string())? != 7 {
            return Err("codec seek/read failed".into());
        }
        let error = region
            .iter()
            .zip(&audio[246..260])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        if error > 2e-6 {
            return Err("codec decoded samples differ".into());
        }
        if manager.read_all(&file, 8).is_ok() || reader.seek(513).is_ok() {
            return Err("codec size/seek bounds failed".into());
        }
        let decoded = soniclaw::read_audio_with_limit(&file, 8192)?;
        if decoded.samples.len() != audio.len() {
            return Err("facade asset decoder failed".into());
        }
        reports.push(serde_json::json!({"file":name,"info":reader.info(),"seek_error":error,"bounds_checked":true}));
    }
    if manager.supports(AudioFormat::Flac, Encoding::Float32, 48000, 2) {
        return Err("unsupported FLAC float unexpectedly accepted".into());
    }
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"backend":manager.backend_version(),"formats":reports,"backend_kind":"dynamically loaded libsndfile"}),
        )?,
    )?;
    println!(
        "PASS WAV/AIFF/FLAC PCM24, metadata, seek, streaming writes and bounded project decode"
    );
    Ok(())
}
fn midi_io_demo(path: &Path) -> Result<()> {
    use soniclaw::midi::{learning::*, output, schedule::*, wire::*};
    let mut parser = Parser::default();
    let messages: Vec<_> = [0xC0, 9, 0xF0, 1, 0xF8, 2, 0xF7, 0xB0, 7, 127]
        .into_iter()
        .filter_map(|b| parser.feed(b))
        .collect();
    if messages.len() != 4 {
        return Err("MIDI packet parse failed".into());
    }
    let anchor = TimestampMapper::new(1_000_000, 0, 48000.0)?;
    let mut scheduler = Scheduler::new(16)?;
    for (i, p) in messages.iter().enumerate() {
        scheduler.enqueue(anchor.sample(1_000_000 + i as u64 * 1000)?, *p)?;
    }
    let mut out = [Scheduled {
        frame: 0,
        packet: Packet::new(&[0xF8])?,
    }; 16];
    let mut count = 0;
    let alloc = allocation_counter::measure(|| {
        count = scheduler.drain(0, 256, &mut out).unwrap();
    });
    if count != 4 {
        return Err("scheduler lost MIDI packets".into());
    }
    if alloc.count_total != 0 || out[1].frame != 48 {
        return Err("sample MIDI scheduling/heap failed".into());
    }
    let parameter = std::sync::Arc::new(Parameter::new(ParameterInfo {
        id: "gain",
        name: "Gain",
        min: -60.,
        max: 6.,
        default: 0.,
        unit: "dB",
    })?);
    let mut learn = LearnMap::default();
    learn.learn(0, 7, "gain")?;
    let controls = learn.compile(std::slice::from_ref(&parameter))?;
    controls.apply(messages[3].to_core().ok_or("CC conversion failed")?)?;
    if parameter.get() != 6.0 {
        return Err("CC learning failed".into());
    }
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);
    impl output::Sink for Capture {
        fn send(&mut self, p: &Packet) -> std::result::Result<(), output::OutputError> {
            self.0.lock().unwrap().push(p.bytes().to_vec());
            Ok(())
        }
    }
    let log = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let sink_log = log.clone();
    let clock =
        output::Clock::new(48000., 0, std::time::Instant::now()).map_err(|e| e.to_string())?;
    let (mut worker, mut sender) =
        output::OutputWorker::start(16, clock, move || Ok(Box::new(Capture(sink_log))))
            .map_err(|e| e.to_string())?;
    for p in &out[..count] {
        sender.send(p.frame as u64, p.packet)?;
    }
    let started = std::time::Instant::now();
    while sender.stats().sent < 4 && started.elapsed() < std::time::Duration::from_secs(1) {
        std::thread::yield_now();
    }
    let stats = sender.stats();
    if stats.sent != 4 || stats.errors != 0 {
        return Err("scheduled MIDI sink failed".into());
    }
    worker.stop();
    std::fs::create_dir(path)?;
    std::fs::write(
        path.join("controller-map.json"),
        serde_json::to_vec_pretty(&learn)?,
    )?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"packets":messages.iter().map(|p|p.bytes()).collect::<Vec<_>>(),"sample_offsets":out[..count].iter().map(|e|e.frame).collect::<Vec<_>>(),"callback_allocations":alloc.count_total,"output":stats,"hardware_tested":false,"learned_gain":parameter.get()}),
        )?,
    )?;
    println!(
        "PASS complete MIDI packets, sample scheduling, controller learning and mock output worker"
    );
    Ok(())
}
fn transport_demo(library: &Path, path: &Path) -> Result<()> {
    use soniclaw::automation::{Curve, Document, Lane, Mode, Point};
    use soniclaw::transport::*;
    let timeline = Timeline {
        tempo: vec![
            TempoPoint {
                beat: 0.,
                bpm: 60.,
                transition: Transition::Linear,
            },
            TempoPoint {
                beat: 4.,
                bpm: 120.,
                transition: Transition::Step,
            },
        ],
        meters: vec![
            MeterPoint {
                beat: 0.,
                numerator: 4,
                denominator: 4,
            },
            MeterPoint {
                beat: 4.,
                numerator: 7,
                denominator: 8,
            },
        ],
        loop_range: Some([1., 7.5]),
        punch_range: Some([2., 3.]),
        ..Default::default()
    };
    let map = TempoMap::compile(&timeline)?;
    let mut state = ProjectState::empty("org.soniclaw.workbench", "Musical automation");
    let mut n = RackNode::new("gain", ProcessorKind::Gain);
    n.input_gain = 1.;
    state.nodes.push(n);
    state.outputs.insert("gain".into(), 1.);
    let p =
        soniclaw_workbench::create_plugin(app_descriptor(&state), state.clone(), default_root())?;
    let id = p.parameters()[0].info().id.to_owned();
    let lane = Lane {
        parameter: id,
        enabled: true,
        mode: Mode::Read,
        points: vec![
            Point {
                beat: 0.,
                value: 0.25,
                curve: Curve::Linear,
            },
            Point {
                beat: 8.,
                value: 0.75,
                curve: Curve::Linear,
            },
        ],
    };
    state.automation = Some(Document {
        loop_range: None,
        lanes: vec![lane.clone()],
    });
    state.timeline = Some(timeline.clone());
    let mut p =
        soniclaw_workbench::create_plugin(app_descriptor(&state), state.clone(), default_root())?;
    p.activate(ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 257,
    })?;
    let mut data = vec![0.0; 288000 * 2];
    let mut at = 0;
    for b in data.chunks_mut(514) {
        for (i, f) in b.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            f.fill((std::f64::consts::TAU * 220. * (at + i) as f64 / 48000.).sin() as f32 * 0.1);
        }
        p.set_transport(Transport {
            sample_position: at as u64,
            tempo: 120.,
            playing: true,
        })?;
        p.process(b, &[])?;
        at += b.len() / 2;
    }
    let mut clock = Clock::new(&timeline, 48000.)?;
    clock.set_playing(true);
    clock.seek(287999)?;
    let last = clock.frame()?;
    let expected = lane.value_at(last.position.beat).unwrap();
    let info = p.parameters()[0].info();
    let normalized = (p.parameters()[0].get() - info.min) / (info.max - info.min);
    if (expected - normalized).abs() > 1e-6 || data.iter().any(|x| !x.is_finite()) {
        return Err(
            format!("musical project automation mismatch: {expected} / {normalized}").into(),
        );
    }
    // SAFETY: explicit fixture path is trusted native code.
    let lib = unsafe { soniclaw::host::ClapLibrary::load(library) }.map_err(|e| e.to_string())?;
    let mut instance = lib
        .instantiate("org.soniclaw.host-fixture")
        .map_err(|e| e.to_string())?;
    instance
        .activate(ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 16,
        })
        .map_err(|e| e.to_string())?;
    let native = instance
        .run_audio(|audio| {
            let mut d = [1.0; 16];
            audio.process_musical(&mut d, &[], &[], last).unwrap();
            d
        })
        .map_err(|e| e.to_string())?;
    if native.iter().any(|x| (*x - 0.5).abs() > 1e-6) {
        return Err("native musical transport process failed".into());
    }
    instance.deactivate();
    std::fs::create_dir(path)?;
    std::fs::write(path.join("project.json"), state.to_json()?)?;
    soniclaw::io::write_wav(path.join("tempo-map.wav"), 48000, 2, &data)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"last_frame":last,"expected_normalized_gain":expected,"actual_normalized_gain":normalized,"ramp_seconds_at_beat4":map.seconds_at(4.)?,"frames":288000,"native_musical_process":true}),
        )?,
    )?;
    println!("PASS analytic tempo ramp, meter/loop clock, mapped project automation and native musical transport");
    Ok(())
}
#[cfg(target_os = "linux")]
fn editor_demo(library: &Path, path: &Path) -> Result<()> {
    // SAFETY: explicitly selected trusted fixture/framework library.
    let lib = unsafe { soniclaw::host::ClapLibrary::load(library) }.map_err(|e| e.to_string())?;
    let id = lib
        .plugins()
        .first()
        .ok_or("no editor products")?
        .id
        .clone();
    let fixture = id == "org.soniclaw.host-fixture";
    let instance = lib.instantiate(&id).map_err(|e| e.to_string())?;
    let (mut owner, processor) = soniclaw::host::live::LiveRack::start_direct(
        vec![instance],
        ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 256,
        },
    )
    .map_err(|e| e.to_string())?;
    owner
        .show_embedded_editor(0, "Soniclaw embedded editor validation")
        .map_err(|e| e.to_string())?;
    let mut events = vec![];
    let mut pump = |owner: &mut soniclaw::host::live::LiveRack| {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(600) {
            owner.poll();
            events.extend(owner.drain_editor_events());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    pump(&mut owner);
    let resize = owner
        .resize_editor(0, [800, 600])
        .map_err(|e| e.to_string());
    let scale = owner.scale_editor(0, 1.25).map_err(|e| e.to_string());
    if fixture && (resize.is_err() || scale.is_err()) {
        return Err("fixture size/scale lifecycle failed".into());
    }
    pump(&mut owner);
    let first_size = owner.editor_size(0);
    owner.close_editor(0);
    owner
        .show_embedded_editor(0, "Soniclaw reopened editor")
        .map_err(|e| e.to_string())?;
    pump(&mut owner);
    owner.focus_editor(0).map_err(|e| e.to_string())?;
    if first_size != owner.editor_size(0) {
        return Err(format!(
            "editor grew across reopen: {first_size:?} -> {:?}",
            owner.editor_size(0)
        )
        .into());
    }
    let stats = owner.service_stats(0).map_err(|e| e.to_string())?;
    if fixture && (stats.timer_dispatches == 0 || stats.fd_dispatches == 0) {
        return Err(format!("timer/fd callbacks were not serviced: {stats:?}").into());
    }
    std::fs::create_dir(path)?;
    owner
        .capture_editor_ppm(0, &path.join("embedded.ppm"))
        .map_err(|e| e.to_string())?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"product":id,"fixture_gui_mock":fixture,"native_gui_render":!fixture,"first_size":first_size,"reopened_size":owner.editor_size(0),"resize":resize,"scale":scale,"services":stats,"events":events}),
        )?,
    )?;
    owner.close_editor(0);
    drop(owner);
    drop(processor);
    println!("PASS embedded parent, reopen, size/scale negotiation and main-thread services");
    Ok(())
}
#[cfg(feature = "vst3-tools")]
fn vst3_demo(library: &Path, path: &Path, editor: bool) -> Result<()> {
    use soniclaw::{
        host::formats::{Instance as Common, PluginFormat},
        vst3::{self, TransportPosition},
    };
    // SAFETY: an explicitly selected trusted validation bundle.
    let products = unsafe { soniclaw::host::formats::products(PluginFormat::Vst3, library) }
        .map_err(|e| e.to_string())?;
    let selected = products
        .iter()
        .find(|p| p.name.contains("Gain + Filter"))
        .ok_or("select the Soniclaw reference VST3 bundle")?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 256,
    };
    let mut common = unsafe { Common::load(selected, spec) }.map_err(|e| e.to_string())?;
    let params = common.parameters().map_err(|e| e.to_string())?;
    let gain = params
        .iter()
        .find(|p| p.name == "Gain")
        .ok_or("Gain parameter missing")?
        .id;
    let mode = params
        .iter()
        .find(|p| p.name == "Mode")
        .ok_or("Mode parameter missing")?
        .id;
    common
        .set_normalized(mode, 0.5)
        .map_err(|e| e.to_string())?;
    common
        .set_normalized(gain, 0.4)
        .map_err(|e| e.to_string())?;
    let Common::Vst3(mut owner) = common else {
        return Err("wrong host backend".into());
    };
    let initial = owner.save_state().map_err(|e| e.to_string())?;
    let mut corrupt = initial.clone();
    corrupt.sha256 = "bad".into();
    if owner.restore_state(&corrupt).is_ok() {
        return Err("corrupt state accepted".into());
    }
    let mut data = [1.; 512];
    owner
        .process_offline(
            &mut data,
            &[(128, gain, 0.8)],
            TransportPosition {
                samples: 96000,
                quarter_notes: 4.,
            },
            120.,
        )
        .map_err(|e| e.to_string())?;
    if data[..256].iter().any(|x| (*x - 0.4).abs() > 1e-5) || data[256] <= 0.4 || data[510] < 0.49 {
        return Err(format!(
            "timed gain result mismatch: first={}, at128={}, last={}",
            data[0], data[256], data[510]
        )
        .into());
    }
    owner.restore_state(&initial).map_err(|e| e.to_string())?;
    let mut restored = [1.; 512];
    owner
        .process_offline(&mut restored, &[], TransportPosition::default(), 120.)
        .map_err(|e| e.to_string())?;
    if restored.iter().any(|x| (*x - 0.4).abs() > 1e-5) {
        return Err("opaque state restoration changed output".into());
    }
    let mut processor = owner.start_buffered().map_err(|e| e.to_string())?;
    processor.prepare(spec)?;
    let mut rendered = vec![];
    let mut allocations = 0;
    for block in 0..32 {
        let mut samples = [1.; 512];
        let ctx = ProcessContext {
            events: &[],
            transport: Transport {
                sample_position: block * 256,
                tempo: 120.,
                playing: true,
            },
        };
        let info = allocation_counter::measure(|| {
            processor.process(&mut AudioBuffer::new(&mut samples, 2).unwrap(), &ctx)
        });
        allocations += info.count_total;
        rendered.extend_from_slice(&samples);
        owner.poll().map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(7));
    }
    let stats = owner.stats();
    let onset = rendered
        .as_chunks::<2>()
        .0
        .iter()
        .position(|f| f[0].abs() > 0.01)
        .ok_or("no buffered output")?;
    if allocations != 0
        || onset != vst3::LATENCY
        || stats.process_errors != 0
        || stats.input_drops != 0
        || stats.underruns != 0
    {
        return Err(format!(
            "buffered VST3 check failed: allocations={allocations}, onset={onset}, stats={stats:?}"
        )
        .into());
    }
    if rendered[vst3::LATENCY * 2..]
        .iter()
        .any(|x| (*x - 0.4).abs() > 1e-5)
    {
        return Err("buffered output differs from reference gain".into());
    }
    owner.stop().map_err(|e| e.to_string())?;
    if editor {
        owner.open_editor().map_err(|e| e.to_string())?;
        std::fs::write(path.with_extension("editor-ready"), b"ready")?;
        for _ in 0..160 {
            owner.poll().map_err(|e| e.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !owner.editor_open() {
            return Err("VST3 editor closed unexpectedly".into());
        }
        owner.close_editor();
        owner.open_editor().map_err(|e| e.to_string())?;
        for _ in 0..100 {
            owner.poll().map_err(|e| e.to_string())?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        owner.close_editor();
    }
    drop(owner);
    let mut revoked = [1.; 128];
    processor.process(
        &mut AudioBuffer::new(&mut revoked, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    if revoked.iter().any(|x| *x != 0.) {
        return Err("retained processor not revoked".into());
    }
    let drums = products
        .iter()
        .find(|p| p.name.contains("Multisample Drums"))
        .ok_or("drum VST3 class missing")?;
    let mut instrument = unsafe { vst3::Instance::load(&drums.path, Some(&drums.id), spec) }
        .map_err(|e| e.to_string())?;
    let mut midi_processor = instrument.start_buffered().map_err(|e| e.to_string())?;
    midi_processor.prepare(spec)?;
    let mut midi_energy = 0.0_f64;
    let mut midi_tail_energy = 0.0_f64;
    let mut midi_allocations = 0;
    for block in 0..32 {
        let on = [Event {
            frame: 7,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 36,
                velocity: 110,
            },
        }];
        let off = [Event {
            frame: 0,
            message: MidiMessage::AllNotesOff,
        }];
        let events = if block == 0 {
            &on[..]
        } else if block == 24 {
            &off[..]
        } else {
            &[]
        };
        let ctx = ProcessContext {
            events,
            transport: Transport {
                sample_position: block * 256,
                tempo: 120.,
                playing: true,
            },
        };
        let mut samples = [0.; 512];
        let info = allocation_counter::measure(|| {
            midi_processor.process(&mut AudioBuffer::new(&mut samples, 2).unwrap(), &ctx)
        });
        midi_allocations += info.count_total;
        let energy = samples.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
        midi_energy += energy;
        if (8..24).contains(&block) {
            midi_tail_energy += energy;
        }
        instrument.poll().map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(7));
    }
    let midi_stats = instrument.stats();
    instrument.stop().map_err(|e| e.to_string())?;
    if midi_stats.restart_requests > 2
        || midi_tail_energy < 0.0001
        || midi_energy < 0.01
        || midi_allocations != 0
        || midi_stats.process_errors != 0
        || midi_stats.underruns != 0
    {
        return Err(format!("native VST3 MIDI failed: energy={midi_energy}, allocations={midi_allocations}, stats={midi_stats:?}").into());
    }
    drop(midi_processor);
    drop(instrument);
    std::fs::create_dir(path)?;
    std::fs::write(
        path.join("state.json"),
        initial.to_json().map_err(|e| e.to_string())?,
    )?;
    soniclaw::io::write_wav(path.join("buffered.wav"), 48000, 2, &rendered)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"products":products,"parameters":params,"timed_gain":{"before":data[0],"at128":data[256],"last":data[510]},"restored_gain":restored[0],"stats":stats,"onset_frames":onset,"callback_allocations":allocations,"owner_revocation":true,"editor_open_close_reopen":editor,"midi_tail_energy":midi_tail_energy,"midi_energy":midi_energy,"midi_callback_allocations":midi_allocations,"midi_stats":midi_stats}),
        )?,
    )?;
    println!(
        "PASS native VST3 factory, timed gain, opaque state, buffered callback, owner revocation{}",
        if editor { ", native editor reopen" } else { "" }
    );
    Ok(())
}
#[cfg(feature = "vst3-tools")]
fn vst3_probe(library: &Path, path: &Path) -> Result<()> {
    // SAFETY: explicit trusted path; parent supervisor provides scan timeout/crash containment.
    let report = unsafe { soniclaw::vst3::probe(library) }.map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

#[cfg(feature = "vst3-tools")]
fn vst3_direct_demo(library: &Path, path: &Path) -> Result<()> {
    use soniclaw::vst3;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 512,
    };
    let products = unsafe {
        soniclaw::host::formats::products(soniclaw::host::formats::PluginFormat::Vst3, library)
    }
    .map_err(|e| e.to_string())?;
    let product = products
        .iter()
        .find(|p| p.name.contains("Gain + Filter"))
        .ok_or("reference VST3 expected")?;
    let mut owner = unsafe { vst3::Instance::load(library, Some(&product.id), spec) }
        .map_err(|e| e.to_string())?;
    let params = owner.parameters().map_err(|e| e.to_string())?;
    let mode = params.iter().find(|p| p.name == "Mode").ok_or("Mode")?.id;
    let gain = params.iter().find(|p| p.name == "Gain").ok_or("Gain")?.id;
    owner.set_parameter(mode, 0.5).map_err(|e| e.to_string())?;
    owner.set_parameter(gain, 0.4).map_err(|e| e.to_string())?;
    let mut processor = owner.start_direct().map_err(|e| e.to_string())?;
    processor.prepare(spec)?;
    let latency = processor.latency_frames();
    if latency != 0 || owner.latency_frames().map_err(|e| e.to_string())? != 0 {
        return Err("direct bridge reported added delay".into());
    }
    let mut allocations = 0;
    let mut checked = 0;
    for size in [1usize, 7, 64, 255, 257, 512] {
        let mut samples = vec![1.; size * 2];
        let ctx = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        let a = allocation_counter::measure(|| {
            processor.process(&mut AudioBuffer::new(&mut samples, 2).unwrap(), &ctx)
        });
        allocations += a.count_total;
        checked += size;
        if samples.iter().any(|x| (*x - 0.4).abs() > 1e-5) {
            return Err(format!(
                "direct output mismatch at block size {size}: {}",
                samples[0]
            )
            .into());
        }
    }
    owner.set_parameter(gain, 0.8).map_err(|e| e.to_string())?;
    let mut controls = [1.; 1024];
    let ctx = ProcessContext {
        events: &[],
        transport: Default::default(),
    };
    let a = allocation_counter::measure(|| {
        processor.process(&mut AudioBuffer::new(&mut controls, 2).unwrap(), &ctx)
    });
    allocations += a.count_total;
    if controls[1022] < 0.79 || allocations != 0 || owner.direct_errors() != 0 {
        return Err(format!(
            "direct controls/allocations failed: last={}, alloc={allocations}",
            controls[1022]
        )
        .into());
    }
    drop(owner);
    let mut revoked = [1.; 128];
    processor.process(&mut AudioBuffer::new(&mut revoked, 2)?, &ctx);
    if revoked.iter().any(|x| *x != 0.) {
        return Err("owner-first destruction failed".into());
    }
    std::fs::create_dir(path)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"bridge_frames":latency,"callback_allocations":allocations,"variable_frames_checked":checked,"live_control_final":controls[1022],"owner_first_revocation":true}),
        )?,
    )?;
    println!("PASS native direct VST3, arbitrary callback sizes, zero host allocations, controls and owner-first revocation");
    Ok(())
}

#[cfg(feature = "vst3-tools")]
fn mixed_demo(fixture: &Path, bundle: &Path, path: &Path) -> Result<()> {
    use soniclaw::{
        host::{
            formats::{PluginFormat, TrustedLoader},
            session::{EditorSettings, MissingPolicy, Slot},
        },
        hosted::{self, Source},
        state::{ProcessorKind, ProjectState, RackNode},
    };
    use std::collections::BTreeMap;
    let products = unsafe { soniclaw::host::formats::products(PluginFormat::Vst3, bundle) }
        .map_err(|e| e.to_string())?;
    let product = products
        .into_iter()
        .find(|p| p.name.contains("Gain + Filter"))
        .ok_or("native reference VST3 required")?;
    let v = unsafe {
        soniclaw::vst3::Instance::load(
            bundle,
            Some(&product.id),
            ProcessSpec {
                sample_rate: 48000.,
                channels: 2,
                max_block_frames: 256,
            },
        )
    }
    .map_err(|e| e.to_string())?;
    let parameters = v.parameters().map_err(|e| e.to_string())?;
    let gain = parameters
        .iter()
        .find(|p| p.name == "Gain")
        .ok_or("gain")?
        .id;
    let mode = parameters
        .iter()
        .find(|p| p.name == "Mode")
        .ok_or("mode")?
        .id;
    drop(v);
    let mut project = ProjectState::empty("org.soniclaw.mixed.native", "Native gain");
    let mut gain_node = RackNode::new("gain", ProcessorKind::Gain);
    gain_node.input_gain = 1.;
    gain_node.settings.insert("gain".into(), 0.5);
    project.nodes.push(gain_node);
    project.outputs.insert("gain".into(), 1.);
    let clap = Slot {
        id: "clap".into(),
        entry: soniclaw::host::live::RackEntry {
            path: fixture.to_owned(),
            plugin_id: "org.soniclaw.host-fixture".into(),
        },
        plugin_version: "fixture".into(),
        bypass: false,
        editor: EditorSettings::default(),
        parameters: BTreeMap::new(),
        state_hex: None,
        state_sha256: None,
    };
    let document = hosted::Session {
        schema: 1,
        name: "Three formats".into(),
        nodes: vec![
            hosted::Node {
                id: "native".into(),
                source: Source::Native {
                    project: Box::new(project),
                    root: ".".into(),
                },
                bypass: false,
                saved_latency: 0,
            },
            hosted::Node {
                id: "clap".into(),
                source: Source::Clap {
                    slot: Box::new(clap),
                },
                bypass: false,
                saved_latency: 0,
            },
            hosted::Node {
                id: "vst3".into(),
                source: Source::Vst3 {
                    product: Box::new(product),
                    state: None,
                    parameters: BTreeMap::from([(gain, 0.4), (mode, 0.5)]),
                    editor: Default::default(),
                    musical: None,
                },
                bypass: false,
                saved_latency: 0,
            },
        ],
        connections: vec![
            hosted::Connection {
                from: "native".into(),
                to: "clap".into(),
                gain: 1.,
            },
            hosted::Connection {
                from: "clap".into(),
                to: "vst3".into(),
                gain: 1.,
            },
        ],
        inputs: BTreeMap::from([("native".into(), 1.)]),
        outputs: BTreeMap::from([("vst3".into(), 1.)]),
    };
    let loader = unsafe { TrustedLoader::new(&[fixture.to_owned(), bundle.to_owned()]) }
        .map_err(|e| e.to_string())?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 256,
    };
    let (mut owner, mut processor) = document.build(&loader, spec, MissingPolicy::Fail)?;
    processor.prepare(spec)?;
    let latency = processor.latency_frames();
    fn render_mixed(
        owner: &mut hosted::Owner,
        processor: &mut hosted::Processor,
        blocks: u64,
    ) -> Result<(Vec<f32>, u64)> {
        std::thread::scope(|scope| {
            let handle = scope.spawn(move || {
                let mut allocations = 0;
                let mut output = vec![];
                for block in 0..blocks {
                    let mut data = [1.; 512];
                    let ctx = ProcessContext {
                        events: &[],
                        transport: Transport {
                            sample_position: block * 256,
                            ..Default::default()
                        },
                    };
                    let info = allocation_counter::measure(|| {
                        processor.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx)
                    });
                    allocations += info.count_total;
                    output.extend_from_slice(&data);
                    std::thread::sleep(std::time::Duration::from_millis(7));
                }
                (output, allocations)
            });
            while !handle.is_finished() {
                owner.poll()?;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            handle
                .join()
                .map_err(|_| "mixed audio thread panicked".into())
        })
    }
    let (output, allocations) = render_mixed(&mut owner, &mut processor, 24)?;
    eprintln!("Mixed diagnostics {:?}", owner.diagnostics());
    if allocations != 0
        || latency != 1024
        || output[..2048].iter().any(|x| *x != 0.)
        || output[2048..].iter().any(|x| (*x - 0.1).abs() > 1e-5)
    {
        return Err(format!(
            "mixed graph failed, latency={latency}, alloc={allocations}, final={}",
            output.last().unwrap()
        )
        .into());
    }
    owner.set_bypass("vst3", true)?;
    let captured = owner.capture()?;
    drop(owner);
    drop(processor);
    let text = captured.to_json()?;
    let round = hosted::Session::from_json(&text)?;
    let (mut owner, mut p) = round.build(&loader, spec, MissingPolicy::Fail)?;
    p.prepare(spec)?;
    let (bypassed, _) = render_mixed(&mut owner, &mut p, 8)?;
    if bypassed[2048..].iter().any(|x| (*x - 0.25).abs() > 1e-5) {
        return Err("mixed bypass/state roundtrip failed".into());
    }
    let preserved = owner.capture()?;
    drop(owner);
    drop(p);
    let mut missing = preserved.clone();
    if let Source::Clap { slot } = &mut missing.nodes[1].source {
        slot.entry.path = Path::new("/missing/fixture.clap").into();
    }
    let (mut owner, p) = missing.build(&loader, spec, MissingPolicy::Placeholder)?;
    if owner.warnings.len() != 1 {
        return Err("missing mixed node was not preserved".into());
    }
    let recovered = owner.capture()?;
    if recovered.nodes.len() != 3 {
        return Err("mixed placeholders lost node identity".into());
    }
    drop(owner);
    drop(p);
    std::fs::create_dir(path)?;
    captured.write(&path.join("session.json"))?;
    std::fs::write(path.join("missing.json"), recovered.to_json()?)?;
    soniclaw::io::write_wav(path.join("mixed.wav"), 48000, 2, &output)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"formats":["native","CLAP","VST3"],"expected_gain":0.1,"final_gain":output.last(),"callback_allocations":allocations,"latency_frames":latency,"restored_bypass_gain":0.25,"missing_node_preserved":true}),
        )?,
    )?;
    println!("PASS mixed native/CLAP/VST3 graph, state, aligned bypass, missing node and callback allocations");
    Ok(())
}

#[cfg(feature = "vst3-tools")]
fn offline_host_demo(bundle: &Path, path: &Path) -> Result<()> {
    use soniclaw::{
        rendering::{self, Plan, Stem},
        vst3,
    };
    let products = unsafe {
        soniclaw::host::formats::products(soniclaw::host::formats::PluginFormat::Vst3, bundle)
    }
    .map_err(|e| e.to_string())?;
    let product = products
        .iter()
        .find(|p| p.name.contains("Multisample Drums"))
        .ok_or("reference drums expected")?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 256,
    };
    let mut owner = unsafe { vst3::Instance::load(bundle, Some(&product.id), spec) }
        .map_err(|e| e.to_string())?;
    let initial = owner.save_state().map_err(|e| e.to_string())?;
    let mut plan = Plan {
        frames: 2400,
        tail_frames: 4800,
        block_frames: 64,
        ..Default::default()
    };
    let event = Event {
        frame: 7,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 36,
            velocity: 110,
        },
    };
    let rendered = owner
        .with_offline(|audio| {
            rendering::render(audio, &plan, &[], &[event]).map_err(|e| e.to_string().into())
        })
        .map_err(|e| e.to_string())?;
    owner.restore_state(&initial).map_err(|e| e.to_string())?;
    plan.block_frames = 256;
    let other = owner
        .with_offline(|audio| {
            rendering::render(audio, &plan, &[], &[event]).map_err(|e| e.to_string().into())
        })
        .map_err(|e| e.to_string())?;
    let difference = rendered
        .samples
        .iter()
        .zip(&other.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    let tail_energy: f64 = rendered.samples[4800..]
        .iter()
        .map(|x| (*x as f64).powi(2))
        .sum();
    if rendered.frames != 7200 || difference > 1e-5 || tail_energy < 0.01 {
        return Err(format!(
            "continuous offline mismatch: frames={}, difference={difference}, tail={tail_energy}",
            rendered.frames
        )
        .into());
    }
    std::fs::create_dir(path)?;
    rendered.write_wav(&path.join("drum-tail.wav"))?;
    let parameter = |id| {
        std::sync::Arc::new(
            Parameter::new(soniclaw::core::ParameterInfo {
                id,
                name: id,
                min: 0.,
                max: 1.,
                default: 0.5,
                unit: "",
            })
            .unwrap(),
        )
    };
    let mut a = soniclaw::dsp::Gain::new(parameter("a"));
    let mut b = soniclaw::dsp::Gain::new(parameter("b"));
    let input = vec![1.; 512];
    let mut stems = [
        Stem {
            name: "A",
            processor: &mut a,
            input: &input,
            events: &[],
        },
        Stem {
            name: "B",
            processor: &mut b,
            input: &input,
            events: &[],
        },
    ];
    let paths = rendering::export_stems(
        &mut stems,
        &Plan {
            frames: 256,
            tail_frames: 32,
            ..Default::default()
        },
        &path.join("stems"),
    )?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":rendered.frames,"block_sizes":[64,256],"max_partition_difference":difference,"tail_energy":tail_energy,"latency_trimmed":rendered.trimmed_latency,"stems":paths,"native_state_preserved_across_blocks":true}),
        )?,
    )?;
    println!(
        "PASS continuous native VST3 offline stream, tail, partition invariance and aligned stems"
    );
    Ok(())
}

fn offline_clap_demo(library: &Path, path: &Path) -> Result<()> {
    let lib = unsafe { soniclaw::host::ClapLibrary::load(library) }.map_err(|e| e.to_string())?;
    let mut owner = lib
        .instantiate("org.soniclaw.host-fixture")
        .map_err(|e| e.to_string())?;
    owner
        .activate(ProcessSpec {
            sample_rate: 48000.,
            channels: 2,
            max_block_frames: 257,
        })
        .map_err(|e| e.to_string())?;
    let latency = owner.latency_frames();
    let rendered = owner
        .run_audio(|audio| {
            let mut processor = audio.processor(latency);
            let result = soniclaw::rendering::render(
                &mut processor,
                &soniclaw::rendering::Plan {
                    frames: 1000,
                    tail_frames: 17,
                    block_frames: 257,
                    ..Default::default()
                },
                &vec![1.; 2000],
                &[],
            );
            (result, processor.error())
        })
        .map_err(|e| e.to_string())?;
    if rendered.1.is_some() {
        return Err("native CLAP offline failure".into());
    }
    let rendered = rendered.0?;
    if rendered.samples[..2000]
        .iter()
        .any(|x| (*x - 0.5).abs() > 1e-6)
        || rendered.samples[2000..].iter().any(|x| *x != 0.)
    {
        return Err("continuous CLAP output/tail failed".into());
    }
    owner.deactivate();
    std::fs::create_dir(path)?;
    rendered.write_wav(&path.join("clap.wav"))?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":rendered.frames,"blocks":rendered.blocks,"continuous_native_clap":true}),
        )?,
    )?;
    println!("PASS continuous native CLAP offline renderer");
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().collect();
    let path = |i| -> Result<&Path> { Ok(Path::new(a.get(i).ok_or("missing path")?)) };
    match a.get(1).and_then(|s|s.to_str()) {
        Some("--precision-host-demo")=>precision_host::run(path(2)?, &a[3..]),
        #[cfg(feature="vst3-tools")]
        Some("--offline-host-demo")=>offline_host_demo(path(2)?,path(3)?),
        #[cfg(feature="vst3-tools")]
        Some("--mixed-demo")=>mixed_demo(path(2)?,path(3)?,path(4)?),
        #[cfg(feature="vst3-tools")]
        Some("--vst3-direct-demo")=>vst3_direct_demo(path(2)?,path(3)?),
        #[cfg(feature="vst3-tools")]
        Some("--vst3-demo")=>vst3_demo(path(2)?,path(3)?,false),
        #[cfg(feature="vst3-tools")]
        Some("--vst3-editor-demo")=>vst3_demo(path(2)?,path(3)?,true),
        #[cfg(feature="vst3-tools")]
        Some("--vst3-probe")=>vst3_probe(path(2)?,path(3)?),
        Some("--offline-clap-demo")=>offline_clap_demo(path(2)?,path(3)?),
        Some("--workflow-demo")=>workflow_demo(path(2)?),
        Some("--registry-demo")=>registry_demo(path(2)?),
        Some("--catalog-demo")=>catalog_demo(path(2)?),
        #[cfg(feature="format-tools")]
        Some("--format-demo")=>format_demo(path(2)?),
        Some("--bus-demo")=>bus_demo(path(2)?,path(3)?),
        #[cfg(target_os="linux")]
        Some("--editor-demo")=>editor_demo(path(2)?,path(3)?),
        Some("--transport-demo")=>transport_demo(path(2)?,path(3)?),
        Some("--midi-io-demo")=>midi_io_demo(path(2)?),
        Some("--session-demo")=>session_demo(path(2)?,path(3)?),
        Some("--direct-demo")=>direct_demo(path(2)?,path(3)?),
        Some("--probe-trusted")=>probe_trusted(path(2)?,path(3)?),
        Some("--live-rack-demo")=>live_rack_demo(path(2)?,path(3)?),
        Some("--automation-demo")=>automation_demo(path(2)?),
        Some("--sampler-demo")=>sampler_demo(path(2)?),
        #[cfg(feature="device-tools")]
        Some("--duplex-demo")=>duplex_demo(path(2)?),
        Some("--pack")=>pack(path(2)?,path(3)?),
        Some("--unpack")=>{soniclaw::project_storage::unpack_bundle(path(2)?,path(3)?)?;Ok(())},
        Some("--demo")=>demo(path(2)?),Some("--audit")=>audit(path(2)?),Some("--host")=>host(path(2)?,path(3)?),Some("--host-contract")=>host_contract(path(2)?,path(3)?),
        _=>Err("use --registry-demo NEWDIR, --workflow-demo NEWDIR, --pack PROJECT.json NEW.soniclaw, --unpack FILE.soniclaw NEWDIR, --demo NEWDIR, --audit NEW.json, --host LIB NEWDIR or --host-contract LIB NEW.json".into())
    }
}
