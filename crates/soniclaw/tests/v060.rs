use soniclaw::{
    dsp::{
        granular::GranularSynth, modulation::*, spectral_fx::*, stereo_convolution::*,
        AutomatedEffect, EffectKind,
    },
    plugin::PluginDescriptor,
    prelude::*,
    state::*,
};
use std::sync::Arc;
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 128,
        channels: 2,
    }
}
fn ctx(events: &[Event]) -> ProcessContext<'_> {
    ProcessContext {
        events,
        transport: Transport {
            playing: true,
            ..Transport::default()
        },
    }
}
#[test]
fn prepared_effects_granular_modulation_and_ir_swaps_do_not_allocate_or_free() {
    let mut effects = vec![];
    for kind in [
        EffectKind::Multiband,
        EffectKind::DynamicEq,
        EffectKind::SpectralFreeze,
        EffectKind::Robotisation,
        EffectKind::PitchShift,
        EffectKind::FrequencyShift,
    ] {
        let mut effect = AutomatedEffect::new(kind, "fx").unwrap();
        effect.prepare(spec()).unwrap();
        effects.push(effect);
    }
    let sample = Sample::new(
        (0..4800)
            .map(|i| (i as f32 * 0.03).sin() * 0.2)
            .collect::<Vec<_>>(),
        1,
        48000.0,
        60,
    )
    .unwrap();
    let mut grain = GranularSynth::<8>::new(sample, "grain").unwrap();
    grain.parameters()[0].set(80.0).unwrap();
    grain.parameters()[1].set(250.0).unwrap();
    grain.prepare(spec()).unwrap();
    let config = ModulationConfig {
        sources: vec![SourceConfig::Lfo {
            shape: LfoShape::Sine,
            rate_hz: 3.0,
            beats: Some(0.5),
        }],
        routes: vec![ModulationRoute {
            source: 0,
            destination: "grain.position".into(),
            amount: 0.3,
        }],
        macros: vec![],
    };
    let mut matrix = ModulationMatrix::new(config, grain.parameters(), &[]).unwrap();
    matrix.prepare(48000.0).unwrap();
    let (mut control, mut ir) = convolution_pair(
        StereoImpulse::new(vec![1.0], 1, 48000.0).unwrap(),
        ConvolutionMode::ZeroLatency,
        0,
        64,
        spec(),
    )
    .unwrap();
    let mut d = [0.1; 256];
    let events = [Event {
        frame: 17,
        message: MidiMessage::NoteOnId {
            channel: 1,
            note: 60,
            velocity: 0.8,
            note_id: 4,
        },
    }];
    for round in 0..6 {
        control
            .replace(StereoImpulse::new(vec![0.5; 4096], 1, 48000.0).unwrap())
            .unwrap();
        let allocations = allocation_counter::measure(|| {
            for block in 0..96 {
                matrix.tick(120.0, true);
                grain.process(
                    &mut AudioBuffer::new(&mut d, 2).unwrap(),
                    &ctx(if round == 0 && block == 0 {
                        &events
                    } else {
                        &[]
                    }),
                );
                assert!(d.iter().all(|x| x.is_finite()));
                for effect in &mut effects {
                    effect.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&[]));
                    assert!(d.iter().all(|x| x.is_finite()));
                }
                ir.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&[]));
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(control.collect_retired(), 1);
    }
    assert!(grain.dropped_grains > 0);
}
#[test]
fn granular_sustain_then_release_reaches_silence() {
    let mut g =
        GranularSynth::<32>::new(Sample::new(vec![0.2; 4800], 1, 48000.0, 60).unwrap(), "g")
            .unwrap();
    g.prepare(spec()).unwrap();
    let mut data = [0.0; 256];
    let on = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        },
        Event {
            frame: 1,
            message: MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 127,
            },
        },
    ];
    g.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx(&on));
    let off = [Event {
        frame: 0,
        message: MidiMessage::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0,
        },
    }];
    g.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx(&off));
    for _ in 0..200 {
        g.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx(&[]));
    }
    assert!(data.iter().any(|x| x.abs() > 0.001));
    let release = [Event {
        frame: 0,
        message: MidiMessage::ControlChange {
            channel: 0,
            controller: 64,
            value: 0,
        },
    }];
    g.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx(&release));
    for _ in 0..200 {
        g.process(&mut AudioBuffer::new(&mut data, 2).unwrap(), &ctx(&[]));
    }
    assert!(data.iter().all(|x| x.abs() < 1e-8));
    assert_eq!(g.active_grains(), 0);
}
#[test]
fn realtime_pitch_shift_transposes_a_tone_and_declares_delay() {
    let mut effect = SpectralEffect::new(SpectralMode::PitchShift);
    effect.set(12.0, 1.0).unwrap();
    effect
        .prepare(ProcessSpec {
            channels: 1,
            max_block_frames: 48000,
            ..spec()
        })
        .unwrap();
    let mut data: Vec<f32> = (0..48000)
        .map(|i| 0.1 * (std::f32::consts::TAU * 440.0 * i as f32 / 48000.0).sin())
        .collect();
    effect.process(&mut AudioBuffer::new(&mut data, 1).unwrap(), &ctx(&[]));
    assert_eq!(effect.latency_frames(), 1024);
    let crossings = data[4800..43200]
        .windows(2)
        .filter(|x| x[0] <= 0.0 && x[1] > 0.0)
        .count();
    assert!((crossings as f32 - 704.0).abs() < 6.0, "{crossings}");
}
fn modulated_state() -> (PluginDescriptor, ProjectState) {
    let desc = PluginDescriptor {
        id: "org.soniclaw.v060-test",
        name: "Test",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: false,
        accepts_midi: true,
    };
    let mut state = ProjectState::empty(desc.id, "Modulation");
    let mut n = RackNode::new("gain", ProcessorKind::Gain);
    n.input_gain = 1.0;
    n.settings.insert("gain".into(), 0.5);
    state.nodes.push(n);
    state.outputs.insert("gain".into(), 1.0);
    state.modulation = Some(ModulationConfig {
        sources: vec![SourceConfig::Macro { index: 0 }],
        routes: vec![ModulationRoute {
            source: 0,
            destination: "gain.gain".into(),
            amount: 0.1,
        }],
        macros: vec![0.5],
    });
    (desc, state)
}
#[test]
fn project_saves_base_macro_identity_and_clears_removed_routes_transactionally() {
    let (desc, state) = modulated_state();
    let mut plugin = soniclaw::project::create_plugin(desc, state, ".").unwrap();
    let pointers: Vec<_> = plugin.parameters().iter().map(Arc::as_ptr).collect();
    plugin.activate(spec()).unwrap();
    plugin
        .set_transport(Transport {
            playing: true,
            ..Transport::default()
        })
        .unwrap();
    let mut audio = [1.0; 256];
    plugin.process(&mut audio, &[]).unwrap();
    let gain = plugin
        .parameters()
        .iter()
        .find(|p| p.info().id == "gain.gain")
        .unwrap()
        .clone();
    assert!(gain.modulated() > gain.get());
    let saved = plugin.save_state().unwrap();
    let mut state = ProjectState::from_json(&saved).unwrap();
    assert_eq!(state.parameters["gain.gain"], 0.5);
    plugin.deactivate();
    gain.set_modulation(0.1).unwrap();
    state.modulation.as_mut().unwrap().routes[0].destination = "missing.gain".into();
    assert!(plugin.load_state(&state.to_json().unwrap()).is_err());
    assert!(gain.modulated() > gain.get());
    state.modulation.as_mut().unwrap().routes.clear();
    plugin.load_state(&state.to_json().unwrap()).unwrap();
    assert_eq!(gain.modulated(), gain.get());
    assert_eq!(
        pointers,
        plugin
            .parameters()
            .iter()
            .map(Arc::as_ptr)
            .collect::<Vec<_>>()
    );
}
#[test]
fn synced_lfo_and_midi_sources_are_deterministic_and_bounded() {
    let p = Arc::new(
        Parameter::new(ParameterInfo {
            id: "p",
            name: "p",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: "",
        })
        .unwrap(),
    );
    let cfg = ModulationConfig {
        sources: vec![
            SourceConfig::Lfo {
                shape: LfoShape::Saw,
                rate_hz: 1.0,
                beats: Some(1.0),
            },
            SourceConfig::MidiCc {
                channel: 2,
                controller: 74,
            },
        ],
        routes: vec![
            ModulationRoute {
                source: 0,
                destination: "p".into(),
                amount: 0.2,
            },
            ModulationRoute {
                source: 1,
                destination: "p".into(),
                amount: 0.1,
            },
        ],
        macros: vec![],
    };
    let mut m = ModulationMatrix::new(cfg, std::slice::from_ref(&p), &[]).unwrap();
    m.prepare(48000.0).unwrap();
    m.message(MidiMessage::ControlChange {
        channel: 2,
        controller: 74,
        value: 127,
    });
    for _ in 0..12001 {
        m.tick(120.0, true);
    }
    assert!(m.source_values()[0].abs() < 0.001);
    assert_eq!(m.source_values()[1], 1.0);
    assert!((p.modulated() - 0.6).abs() < 0.001);
    assert_eq!(p.get(), 0.5);
    let value = m.source_values()[0];
    for _ in 0..100 {
        m.tick(120.0, false);
    }
    assert!((m.source_values()[0] - value).abs() < 0.001);
    m.reset();
    assert_eq!(p.modulated(), 0.5);
}
