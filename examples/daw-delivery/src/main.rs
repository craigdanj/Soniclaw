use soniclaw::{
    automation::{Curve, Lane, Mode, Point},
    core::*,
    studio::{
        self, cues, delivery, export, jobs, mix_graph,
        project::{Project, ProjectError},
        recording::{self, Arm, Source},
        workspace::{self, Command, Kind},
    },
};
use soniclaw_host::{
    formats::{self, PluginFormat, TrustedLoader},
    studio::{self as native, registry},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
fn read(path: &Path) -> Result<Vec<f32>, ProjectError> {
    Ok(hound::WavReader::open(path)?
        .samples::<f32>()
        .collect::<Result<_, _>>()?)
}
fn music(module: &Path) -> Result<(), ProjectError> {
    // SAFETY: explicitly selected owned protocol fixture.
    let mut p = unsafe {
        soniclaw_vst3::Instance::load(
            module,
            None,
            ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 2,
            },
        )
    }?;
    let mut a = p.start_offline_musical(Default::default())?;
    a.capture_output_events(true);
    let mut pcm = [1.; 256];
    let messages = [
        MidiMessage::NoteOnId {
            channel: 0,
            note: 60,
            velocity: 0.7,
            note_id: 77,
        },
        MidiMessage::NoteExpression {
            target: NoteTarget {
                note_id: 77,
                channel: 0,
                key: 60,
            },
            expression: NoteExpression::Pressure,
            value: 0.6,
        },
        MidiMessage::NoteOffId {
            target: NoteTarget {
                note_id: 77,
                channel: 0,
                key: 60,
            },
        },
    ];
    let mut emitted = Vec::new();
    for (i, message) in messages.into_iter().enumerate() {
        let e = [Event { frame: 17, message }];
        a.process_automated(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &e,
                transport: Transport {
                    sample_position: (i * 128) as u64,
                    playing: true,
                    ..Default::default()
                },
            },
            &[],
        )?;
        emitted.extend(a.take_output_events());
    }
    assert!(emitted.iter().any(|e| e.sample == 145
        && matches!(
            e.event.data,
            soniclaw_vst3::musical::PluginEventData::PolyPressure { note_id: 77, .. }
        )));
    assert_eq!(a.latency_frames(), 0);
    let errors = p.offline_errors();
    assert_eq!(errors, 0);
    drop(a);
    p.stop()?;
    // Owner revocation is safe even if an obsolete offline processor survives on another thread.
    let a = p.start_offline_musical(Default::default())?;
    drop(p);
    let obsolete = std::thread::spawn(move || {
        let mut a = a;
        let mut pcm = [1.; 16];
        let r = a.process_automated(
            &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
            &[],
        );
        assert!(r.is_err());
        assert!(pcm.iter().all(|v| *v == 0.));
    })
    .join();
    assert!(obsolete.is_ok());
    Ok(())
}
fn main() -> Result<(), ProjectError> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(args.get(1).ok_or("NEW_FOLDER TRUSTED_PROTOCOL_FIXTURE")?);
    let module = Path::new(args.get(2).ok_or("owned protocol fixture path")?).canonicalize()?;
    std::fs::create_dir(&out)?;
    music(&module)?;
    let mut p = workspace::blank("Integrated session".into(), 48000, 128)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "mic-b".into(),
            name: "Mic B".into(),
            kind: Kind::Audio,
        },
    )?;
    let bus = Arc::new(ChannelBus::new(3, 128)?);
    let mut armed = recording::prepare_with_inputs(
        &p,
        &mut studio::Builtins,
        recording::Settings {
            arms: vec![
                Arm {
                    track: "audio-1".into(),
                    source: Source::DryInput,
                    monitor_input: false,
                    channels: vec![0],
                },
                Arm {
                    track: "mic-b".into(),
                    source: Source::DryInput,
                    monitor_input: false,
                    channels: vec![1, 2],
                },
            ],
            ..Default::default()
        },
        &out.join("raw-audio"),
        "take",
        0,
        Some(bus.clone()),
    )?;
    let mut pcm = [0.; 256];
    let mut input = [0.; 384];
    for frame in input.chunks_mut(3) {
        frame.copy_from_slice(&[0.125, 0.05, -0.05]);
    }
    for _ in 0..16 {
        bus.write(&input)?;
        armed.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    drop(armed.processor);
    let take = armed.recorder.stop()?;
    assert!(take.complete && take.missing_frames == 0);
    p.takes.push(out.join("raw-audio"));
    assert_eq!(
        recording::place_input_take(&mut p, &out.join("raw-audio"), &armed.schedule)?,
        2
    );
    workspace::apply(
        &mut p,
        Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    studio::stock::set_instrument(&mut p, "keys", studio::stock::instrument(1, "keys.stock")?)?;
    let mut m = studio::midi_recording::prepare_low_latency(
        &p,
        &mut studio::Builtins,
        studio::midi_recording::Settings {
            track: "keys".into(),
            clip: "performance".into(),
            ..Default::default()
        },
    )?;
    for i in 0..8 {
        let events = match i {
            0 => vec![Event {
                frame: 17,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            }],
            1 => vec![Event {
                frame: 20,
                message: MidiMessage::PitchBend {
                    channel: 0,
                    value: 9000,
                },
            }],
            7 => vec![Event {
                frame: 64,
                message: MidiMessage::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 0,
                },
            }],
            _ => vec![],
        };
        m.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &events,
                transport: Default::default(),
            },
        );
        if i == 3 {
            let partial = m.recorder.checkpoint(0)?;
            studio::pending::Checkpoint {
                schema: 1,
                project_id: p.project_id.clone(),
                midi: Some(partial),
                automation: None,
            }
            .save(&out.join("pending"))?;
        }
    }
    let (pending, warnings) =
        studio::pending::Checkpoint::load(&out.join("pending"), &p.project_id)?;
    assert!(warnings.is_empty());
    let recovered = pending
        .midi
        .ok_or("pending MIDI missing")?
        .accept_checkpoint()?;
    let mut recovery = p.clone();
    recovered.apply(&mut recovery)?;
    assert!(!recovery.document.tracks[2].midi.is_empty());
    drop(m.processor);
    let midi = m.recorder.finish(0)?;
    midi.save_new(&out.join("raw-midi"))?;
    midi.apply(&mut p)?;
    p.midi_takes.push(out.join("raw-midi"));
    for t in &mut p.document.tracks {
        t.midi_input = false;
        t.input_gain = 0.;
    }
    p.document.timeline.loop_range = None;
    let before = p.clone();
    let mut group = mix_graph::Bus::new("band".into(), "Band".into());
    group.gain = 0.8;
    p.document.mix_routing.buses.push(group);
    for t in &p.document.tracks {
        p.document
            .mix_routing
            .track_outputs
            .insert(t.id.clone(), mix_graph::Destination::Bus("band".into()));
    }
    p.document.auxiliaries.push(studio::Auxiliary {
        inserts: vec![],
        return_gain: 0.5,
    });
    p.document.mixer.taps.push(soniclaw::mixer::Tap::PostFader);
    for s in &mut p.document.mixer.strips {
        s.sends = vec![0.2];
    }
    p.document.tracks[1].inserts.push(studio::Descriptor {
        id: "duck".into(),
        kind: "stock.sidechain".into(),
        state: serde_json::json!({"threshold":-30.0,"ratio":4.0,"attack_ms":0.01,"release_ms":20.0}),
    });
    p.document
        .mix_routing
        .sidechains
        .push(mix_graph::Sidechain {
            source: "audio-1".into(),
            target: "duck".into(),
        });
    let mut cue = cues::Cue::new("performer".into(), "Performer".into(), [2, 3]);
    cue.sends.push(cues::Send {
        id: "band-send".into(),
        source: cues::Source::Bus("band".into()),
        gain: 0.7,
        pan: 0.,
    });
    p.document.cues.push(cue);
    // SAFETY: explicit owned native fixture selection. Metadata is not permission for other modules.
    let product = unsafe { formats::products(PluginFormat::Vst3, &module) }?.remove(0);
    let entry = registry::Entry {
        fingerprint: registry::fingerprint(&module)?,
        product,
        enabled: true,
    };
    let mut native = registry::descriptor(&entry, "native-master", false);
    native.state["automation_ids"] = serde_json::json!([9]);
    native.state["state_mode"] = serde_json::json!("parameters");
    native.state["native_state"] = serde_json::json!(true);
    p.document.mix_routing.master.inserts.push(native);
    p.native_states.insert(
        "native-master".into(),
        studio::native_state::Blob::new(
            studio::native_state::Identity {
                format: "vst3".into(),
                module: module.clone(),
                product: entry.product.id.clone(),
                version: entry.product.version.clone(),
            },
            std::collections::BTreeMap::from([(9, 0.5)]),
            None,
        )?,
    );
    p.document.automation.lanes.push(Lane {
        parameter: "native-master.native.9".into(),
        enabled: true,
        mode: Mode::Read,
        points: vec![Point {
            beat: 0.,
            value: 0.5,
            curve: Curve::Step,
        }],
    });
    let mut history = studio::history::History::default();
    history.record(before, &mut p, "Mix routing and native master")?;
    history.undo(&mut p)?;
    assert!(p.document.mix_routing.buses.is_empty());
    history.redo(&mut p)?;
    studio::sessions::save(&p, &out.join("song"))?;
    p.name = "Integrated portable song".into();
    assert_eq!(studio::sessions::save(&p, &out.join("song"))?.generation, 2);
    let p = Project::open(&out.join("song"))?;
    std::fs::remove_dir_all(out.join("raw-audio"))?;
    std::fs::remove_dir_all(out.join("raw-midi"))?;
    p.validate()?;
    let settings = export::Settings {
        length_frames: 4096,
        tail_frames: 2048,
        routed_stems: vec![
            delivery::Source::TrackPost("audio-1".into()),
            delivery::Source::TrackPost("mic-b".into()),
            delivery::Source::TrackPost("keys".into()),
            delivery::Source::Bus("band".into()),
            delivery::Source::Aux(0),
        ],
        ..Default::default()
    };
    let jobs = jobs::Scheduler::new(Default::default())?;
    let path = out.join("export");
    let pc = p.clone();
    let mc = module.clone();
    let settings_copy = settings.clone();
    let mut job = jobs.submit(
        "delivery",
        0,
        jobs::Priority::Normal,
        64 * 1024 * 1024,
        move |ctx| {
            // SAFETY: frozen explicit trusted module; construct native owners on this offline worker.
            let loader = unsafe { TrustedLoader::new(&[mc]) }.map_err(|e| e.to_string())?;
            native::render_export(&pc, settings_copy, &path, &loader, &ctx)
                .map_err(|e| e.to_string())
        },
    )?;
    let report = loop {
        if let Some(r) = job.poll(0) {
            break r?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(report.files.len(), 6);
    for f in &report.files {
        assert_eq!(f.frames, 6144);
    }
    let mix = read(&out.join("export/mix.wav"))?;
    let band = read(&out.join("export/route-3-bus-band.wav"))?;
    let aux = read(&out.join("export/route-4-aux-0.wav"))?;
    let residual = mix
        .iter()
        .zip(&band)
        .zip(&aux)
        .fold(0f32, |m, ((mix, band), aux)| {
            m.max((mix - (band + aux) * 0.5).abs())
        });
    assert!(residual < 1e-6, "same-pass graph mismatch {residual}");
    assert!(mix.iter().any(|v| v.abs() > 0.001));
    // Missing native metadata can be bypassed while retaining native automation lanes/values.
    let mut bypass = p.clone();
    let d = &mut bypass.document.mix_routing.master.inserts[0];
    d.state["bypass"] = serde_json::json!(true);
    d.state["latency_frames"] = serde_json::json!(23);
    let empty = TrustedLoader::default();
    let mut owners = vec![];
    let mut f = native::NativeFactory {
        helpers: &Default::default(),
        bus_graph: &mut Default::default(),
        spec: bypass.document.spec(),
        mode: native::Mode::Offline,
        timeline: &bypass.document.timeline,
        assets: &bypass.assets,
        states: &bypass.native_states,
        loader: &empty,
        owners: &mut owners,
        last_error: None,
    };
    let b = bypass
        .document
        .compile_offline(&bypass.assets, &bypass.disk, &mut f)?;
    assert_eq!(b.latency_frames(), 23);
    drop(b);
    assert!(owners.is_empty());
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","virtual_audio_tracks":2,"audio_frames":take.frames,"editable_midi":true,"pending_midi_recovered":true,"generation_saves":2,"source_takes_removed_before_export":true,"submix_sidechain_cue":true,"native_midi_identity_across_blocks":true,"owner_revocation":true,"export_files":report.files.len(),"frames_per_file":6144,"same_pass_residual":residual,"native_bridge_latency_offline":0,"missing_native_bypass_retains_automation":true,"physical_hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
