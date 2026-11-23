use soniclaw::{
    dsp::{resample::ResampleQuality, AutomatedEffect, EffectKind},
    plugin::PluginDescriptor,
    prelude::*,
    project::{create_plugin, ProjectSession},
    sampler::{MultiSampler, SampleBank, Zone},
    state::*,
};
use std::{collections::BTreeMap, sync::Arc};
fn descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "org.soniclaw.test-project",
        name: "Test",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: false,
        accepts_midi: true,
    }
}
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    }
}
fn doc() -> ProjectState {
    let mut p = ProjectState::empty(descriptor().id, "Test");
    p.nodes.push(RackNode {
        custom: None,
        voice_modulation: None,
        asset: None,
        id: "amp".into(),
        kind: ProcessorKind::Gain,
        settings: BTreeMap::from([("gain".into(), 0.5)]),
        input_gain: 1.0,
    });
    p.outputs.insert("amp".into(), 1.0);
    p
}
#[test]
fn complete_state_rebuilds_graph_and_rejects_failure_without_mutation() {
    let mut plugin = create_plugin(descriptor(), doc(), ".").unwrap();
    let first = plugin.save_state().unwrap();
    let mut changed = ProjectState::from_json(&first).unwrap();
    changed.nodes[0].input_gain = 0.25;
    let state = changed.to_json().unwrap();
    plugin.load_state(&state).unwrap();
    plugin.activate(spec()).unwrap();
    let mut out = [1.0; 512];
    plugin.process(&mut out, &[]).unwrap();
    assert!(out.iter().all(|x| (*x - 0.125).abs() < 1e-6));
    assert!(plugin.load_state(&first).is_err());
    plugin.deactivate();
    let before = plugin.save_state().unwrap();
    changed.assets.push(Asset {
        id: "missing".into(),
        path: "no-such-file.wav".into(),
        root_note: 60,
    });
    changed.nodes[0].kind = ProcessorKind::Sampler;
    changed.nodes[0].settings.clear();
    changed.zones.push(SampleZone {
        release_trigger: false,
        velocity_fade_in: None,
        velocity_fade_out: None,
        velocity_power: false,
        start_frame: 0,
        end_frame: None,
        loop_crossfade: 0,
        asset: "missing".into(),
        keys: [60, 60],
        velocities: [1, 127],
        group: 0,
        gain: 1.0,
        one_shot: true,
        loop_frames: None,
    });
    assert!(plugin.load_state(&changed.to_json().unwrap()).is_err());
    assert_eq!(plugin.save_state().unwrap(), before);
}
#[test]
fn undo_redo_rebuild_and_failed_edit_are_transactional() {
    let mut s = ProjectSession::new(doc(), ".", 8).unwrap();
    let _new = s
        .edit(descriptor(), "Quiet", |p| {
            p.parameters.insert("amp.gain".into(), 0.2);
        })
        .unwrap();
    assert_eq!(s.history().current().parameters["amp.gain"], 0.2);
    let before = s.history().current().to_json().unwrap();
    assert!(s
        .edit(descriptor(), "Bad setting", |p| {
            p.nodes[0].settings.insert("typo".into(), 1.0);
        })
        .is_err());
    assert_eq!(s.history().current().to_json().unwrap(), before);
    assert!(s.undo(descriptor()).unwrap().is_some());
    assert!(s.redo(descriptor()).unwrap().is_some());
    assert_eq!(s.history().current().parameters["amp.gain"], 0.2);
}
#[test]
fn automated_effects_and_expressive_sampler_do_not_allocate() {
    let mut effects = vec![];
    for (i, kind) in [
        EffectKind::Gain,
        EffectKind::LowPass,
        EffectKind::Eq,
        EffectKind::Compressor,
        EffectKind::Gate,
        EffectKind::Chorus,
        EffectKind::Reverb,
        EffectKind::Limiter,
        EffectKind::Drive,
        EffectKind::Delay,
    ]
    .into_iter()
    .enumerate()
    {
        let mut e = AutomatedEffect::new(kind, &format!("effect{i}")).unwrap();
        e.prepare(spec()).unwrap();
        effects.push(e);
    }
    let mut z = Zone::single(0, 60);
    z.one_shot = false;
    z.loop_frames = Some((0, 2048));
    let bank = Arc::new(
        SampleBank::new(
            vec![Sample::new(vec![0.25; 4096], 1, 48000.0, 60).unwrap()],
            vec![z],
            16384,
        )
        .unwrap(),
    );
    let mut sampler = MultiSampler::<8>::new(bank);
    sampler.set_quality(ResampleQuality::SincFast);
    sampler
        .set_mpe_zone(Some(MpeZone {
            master: 0,
            first_member: 1,
            last_member: 15,
            member_bend: 48.0,
            master_bend: 2.0,
        }))
        .unwrap();
    sampler.prepare(spec()).unwrap();
    let events = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 1.0,
                note_id: 5,
            },
        },
        Event {
            frame: 50,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    note_id: 5,
                    channel: -1,
                    key: -1,
                },
                expression: NoteExpression::Pressure,
                value: 0.5,
            },
        },
    ];
    let mut audio = [0.1; 512];
    let result = allocation_counter::measure(|| {
        for step in 0..8 {
            sampler.process(
                &mut AudioBuffer::new(&mut audio, 2).unwrap(),
                &ProcessContext {
                    events: if step == 0 { &events } else { &[] },
                    transport: Transport::default(),
                },
            );
            for e in &mut effects {
                for p in e.parameters() {
                    let i = p.info();
                    p.set(i.min + (i.max - i.min) * (step as f32 / 8.0))
                        .unwrap();
                }
                e.process(
                    &mut AudioBuffer::new(&mut audio, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
            }
            assert!(audio.iter().all(|v| v.is_finite()));
        }
    });
    assert_eq!(result.count_total, 0);
    assert_eq!(result.count_current, 0);
}
#[test]
fn sample_map_and_loop_roundtrip_render_without_external_pcm_state() {
    let directory = std::env::temp_dir().join(format!("soniclaw-project-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    soniclaw::io::write_wav(directory.join("sample.wav"), 48000, 1, &vec![0.25; 4096]).unwrap();
    let mut state = doc();
    state.nodes[0].kind = ProcessorKind::Sampler;
    state.nodes[0].input_gain = 0.0;
    state.nodes[0].settings =
        BTreeMap::from([("quality".into(), 2.0), ("release_ms".into(), 200.0)]);
    state.assets.push(Asset {
        id: "s".into(),
        path: "sample.wav".into(),
        root_note: 60,
    });
    state.zones.push(SampleZone {
        release_trigger: false,
        velocity_fade_in: None,
        velocity_fade_out: None,
        velocity_power: false,
        start_frame: 0,
        end_frame: None,
        loop_crossfade: 0,
        asset: "s".into(),
        keys: [60, 72],
        velocities: [1, 127],
        group: 0,
        gain: 1.0,
        one_shot: false,
        loop_frames: Some([0, 4096]),
    });
    let mut plugin = create_plugin(
        descriptor(),
        ProjectState::from_json(&state.to_json().unwrap()).unwrap(),
        &directory,
    )
    .unwrap();
    let saved = plugin.save_state().unwrap();
    plugin.load_state(&saved).unwrap();
    plugin.activate(spec()).unwrap();
    let mut out = [0.0; 512];
    plugin
        .process(
            &mut out,
            &[Event {
                frame: 0,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 127,
                },
            }],
        )
        .unwrap();
    assert!(out[128..].iter().all(|v| (*v - 0.25).abs() < 0.0001));
    plugin.deactivate();
    drop(plugin);
    state.nodes[0].kind = ProcessorKind::DiskSampler;
    state.nodes[0].settings =
        std::collections::BTreeMap::from([("quality".into(), 0.0), ("attack_ms".into(), 0.0)]);
    let mut disk = create_plugin(descriptor(), state, &directory).unwrap();
    let saved = disk.save_state().unwrap();
    disk.load_state(&saved).unwrap();
    disk.activate(spec()).unwrap();
    disk.process(
        &mut out,
        &[Event {
            frame: 0,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 127,
            },
        }],
    )
    .unwrap();
    assert!(out.iter().all(|v| (*v - 0.25).abs() < 0.0001));
    disk.deactivate();
    drop(disk);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn disk_callback_and_seek_do_not_allocate() {
    use std::time::Duration;
    let path = std::env::temp_dir().join(format!("soniclaw-disk-rt-{}.wav", std::process::id()));
    soniclaw::io::write_wav(&path, 48000, 1, &vec![0.25; 8192]).unwrap();
    let (mut h, mut player) = soniclaw::stream::open_seekable_wav(&path, 1024 * 1024).unwrap();
    player.prepare(spec()).unwrap();
    for _ in 0..4 {
        let _ = h.wait_ready(1, Duration::from_millis(100));
        player.process(
            &mut AudioBuffer::new(&mut [], 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    let mut audio = [0.0; 512];
    let allocation = allocation_counter::measure(|| {
        player.process(
            &mut AudioBuffer::new(&mut audio, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        player.seek(5000).unwrap();
        player.process(
            &mut AudioBuffer::new(&mut audio, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        player.reset();
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.count_current, 0);
    h.stop();
    std::fs::remove_file(path).unwrap();
}
#[test]
fn mpe_pitch_changes_only_the_addressed_channel_at_event_time() {
    let mut synth = ExpressiveSynth::<4>::new(
        Waveform::Sine,
        AdsrSettings {
            attack: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 0.1,
        },
    )
    .unwrap();
    synth.timbre_depth = 0.0;
    synth.pressure_gain = 0.0;
    synth
        .set_mpe_zone(Some(MpeZone {
            master: 0,
            first_member: 1,
            last_member: 15,
            member_bend: 12.0,
            master_bend: 2.0,
        }))
        .unwrap();
    synth
        .prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 48000,
            channels: 1,
        })
        .unwrap();
    let events = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 69,
                velocity: 1.0,
                note_id: 1,
            },
        },
        Event {
            frame: 24000,
            message: MidiMessage::PitchBend {
                channel: 1,
                value: 16383,
            },
        },
    ];
    let mut audio = vec![0.0; 48000];
    synth.process(
        &mut AudioBuffer::new(&mut audio, 1).unwrap(),
        &ProcessContext {
            events: &events,
            transport: Transport::default(),
        },
    );
    let crossings = |a: &[f32]| a.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    assert!((crossings(&audio[..24000]) as isize - 220).abs() <= 1);
    assert!((crossings(&audio[24000..]) as isize - 440).abs() <= 1);
}
