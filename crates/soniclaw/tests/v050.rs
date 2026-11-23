use soniclaw::{
    dsp::{
        advanced::{analysis::*, spectral::*, wavetable::*},
        AutomatedEffect, EffectKind,
    },
    plugin::parameter_id,
    plugin::PluginDescriptor,
    prelude::*,
    state::*,
};
use std::{collections::BTreeMap, sync::Arc};
fn sp() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 128,
        channels: 2,
    }
}
fn kinds() -> [EffectKind; 16] {
    [
        EffectKind::StateVariable,
        EffectKind::Ladder,
        EffectKind::CrossoverLow,
        EffectKind::CrossoverMid,
        EffectKind::CrossoverHigh,
        EffectKind::RmsCompressor,
        EffectKind::LookaheadLimiter,
        EffectKind::TransientShaper,
        EffectKind::SpectralGate,
        EffectKind::FdnReverb,
        EffectKind::Phaser,
        EffectKind::Flanger,
        EffectKind::Tremolo,
        EffectKind::AutoPan,
        EffectKind::MultiTapDelay,
        EffectKind::StereoWidth,
    ]
}
#[test]
fn all_new_effect_paths_are_prepared_allocation_free_and_finite() {
    let mut effects = vec![];
    for kind in kinds() {
        let mut p = AutomatedEffect::new(kind, "effect").unwrap();
        p.prepare(sp()).unwrap();
        effects.push(p);
    }
    let mut audio = [0.1; 256];
    let ctx = ProcessContext {
        events: &[],
        transport: Transport::default(),
    };
    let allocation = allocation_counter::measure(|| {
        for step in 0..12 {
            for effect in &mut effects {
                for p in effect.parameters() {
                    let i = p.info();
                    p.set(i.min + (i.max - i.min) * step as f32 / 12.0).unwrap();
                }
                audio.fill(0.1);
                effect.process(&mut AudioBuffer::new(&mut audio, 2).unwrap(), &ctx);
                assert!(audio.iter().all(|x| x.is_finite()));
            }
            for effect in &mut effects {
                effect.reset();
            }
        }
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.count_current, 0);
}
#[test]
fn wavetable_synthesis_and_analysis_do_not_allocate() {
    let bank = Arc::new(WavetableBank::basic(2048).unwrap());
    let mut synth = WavetableSynth::<8>::new(bank, "osc", 4).unwrap();
    synth.prepare(sp()).unwrap();
    let mut analyzer = SpectrumAnalyzer::new(1024, WindowKind::Hann).unwrap();
    let mut loudness = LoudnessMeter::new(48000, 2, 64).unwrap();
    let mut pitch = PitchDetector::new(48000.0, 1024, 100.0, 1000.0).unwrap();
    let mut tp = TruePeakDetector::new(2).unwrap();
    let mut stereo = StereoMeter::default();
    let mut d = [0.0; 256];
    let input = [0.1; 1024];
    let events = [
        Event {
            frame: 12,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id: 7,
            },
        },
        Event {
            frame: 30,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    note_id: 7,
                    channel: -1,
                    key: -1,
                },
                expression: NoteExpression::Tuning,
                value: 7.0,
            },
        },
    ];
    let allocation = allocation_counter::measure(|| {
        for i in 0..12 {
            synth.process(
                &mut AudioBuffer::new(&mut d, 2).unwrap(),
                &ProcessContext {
                    events: if i == 0 { &events } else { &[] },
                    transport: Transport::default(),
                },
            );
            loudness.process(&d).unwrap();
            for f in d.as_chunks::<2>().0 {
                tp.push_frame(f);
                stereo.push(f[0], f[1]);
            }
            analyzer.analyze(&input).unwrap();
            pitch.estimate(&input);
            assert!(d.iter().all(|x| x.is_finite()));
        }
        synth.reset();
    });
    assert_eq!(allocation.count_total, 0);
    assert_eq!(allocation.count_current, 0);
    assert!(tp.peak_linear() > 0.001);
}
struct Tone {
    phase: f64,
    sr: f64,
}
impl AudioProcessor for Tone {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        self.sr = s.sample_rate;
        self.phase = 0.0;
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_exact_mut(ch) {
            f.fill(self.phase.sin() as f32 * 0.1);
            self.phase = (self.phase + std::f64::consts::TAU * 997.0 / self.sr)
                .rem_euclid(std::f64::consts::TAU);
        }
    }
    fn reset(&mut self) {
        self.phase = 0.0;
    }
}
#[test]
fn timed_automation_and_latency_are_partition_independent() {
    for kind in [
        EffectKind::StateVariable,
        EffectKind::Ladder,
        EffectKind::FdnReverb,
        EffectKind::Flanger,
        EffectKind::LookaheadLimiter,
        EffectKind::SpectralGate,
    ] {
        let render = |block| {
            let effect = AutomatedEffect::new(kind, "e").unwrap();
            let p = effect.parameters()[0].clone();
            let id = parameter_id(p.info().id);
            let info = p.info();
            let mut e = Engine::default();
            e.push(Tone {
                phase: 0.0,
                sr: 48000.0,
            });
            e.bind_parameter(id, p.clone()).unwrap();
            e.push(effect);
            e.prepare(ProcessSpec {
                max_block_frames: block,
                ..sp()
            })
            .unwrap();
            let audio = soniclaw::engine::render_automated(
                &mut e,
                8192,
                &[],
                &[
                    AutomationEvent {
                        frame: 173,
                        parameter: id,
                        value: info.min + (info.max - info.min) * 0.3,
                    },
                    AutomationEvent {
                        frame: 2831,
                        parameter: id,
                        value: info.default,
                    },
                ],
            )
            .unwrap();
            (audio, e.latency_frames())
        };
        let (a, latency) = render(128);
        let (b, other) = render(73);
        assert_eq!(latency, other);
        assert!(
            a.iter().zip(&b).all(|(x, y)| (*x - *y).abs() < 1e-6),
            "{kind:?}"
        );
        assert!(a.iter().any(|x| x.abs() > 1e-5));
        if matches!(kind, EffectKind::LookaheadLimiter) {
            assert_eq!(latency, 252);
        }
        if matches!(kind, EffectKind::SpectralGate) {
            assert_eq!(latency, 1024);
        }
    }
}
#[test]
fn new_project_nodes_roundtrip_and_preserve_parameter_identities() {
    let descriptor = PluginDescriptor {
        id: "org.soniclaw.test-dsp",
        name: "DSP",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: false,
        accepts_midi: false,
    };
    let mut project = ProjectState::empty(descriptor.id, "DSP");
    let types = [
        ProcessorKind::StateVariable,
        ProcessorKind::Ladder,
        ProcessorKind::RmsCompressor,
        ProcessorKind::Phaser,
        ProcessorKind::Flanger,
        ProcessorKind::FdnReverb,
        ProcessorKind::SpectralGate,
        ProcessorKind::LookaheadLimiter,
        ProcessorKind::StereoWidth,
    ];
    for (i, kind) in types.into_iter().enumerate() {
        let id = format!("n{i}");
        project.nodes.push(RackNode {
            custom: None,
            voice_modulation: None,
            asset: None,
            id: id.clone(),
            kind,
            settings: BTreeMap::new(),
            input_gain: if i == 0 { 1.0 } else { 0.0 },
        });
        if i > 0 {
            project.connections.push(Connection {
                from: format!("n{}", i - 1),
                to: id,
                gain: 1.0,
            });
        }
    }
    project.outputs.insert("n8".into(), 1.0);
    let mut plugin = soniclaw::project::create_plugin(descriptor, project, ".").unwrap();
    let ids: Vec<_> = plugin.parameters().iter().map(Arc::as_ptr).collect();
    let saved = plugin.save_state().unwrap();
    plugin.load_state(&saved).unwrap();
    assert_eq!(
        ids,
        plugin
            .parameters()
            .iter()
            .map(Arc::as_ptr)
            .collect::<Vec<_>>()
    );
    plugin.activate(sp()).unwrap();
    assert_eq!(plugin.latency_frames(), 1276);
    let mut audio = [0.1; 256];
    for _ in 0..24 {
        plugin.process(&mut audio, &[]).unwrap();
        assert!(audio.iter().all(|x| x.is_finite()));
    }
    plugin.deactivate();
}
#[test]
fn spectral_gate_suppresses_low_noise_and_width_preserves_mono_sum() {
    let mut gate = AutomatedEffect::new(EffectKind::SpectralGate, "gate").unwrap();
    gate.parameters()[0].set(-40.0).unwrap();
    gate.prepare(sp()).unwrap();
    let mut output = [1e-5; 256];
    for _ in 0..32 {
        output.fill(1e-5);
        gate.process(
            &mut AudioBuffer::new(&mut output, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    assert!(output.iter().all(|x| x.abs() < 1e-8));
    for width in [0.0, 0.5, 1.0, 2.0] {
        let a = stereo_width(0.7, -0.2, width);
        assert!((a[0] + a[1] - 0.5).abs() < 1e-6);
    }
}

#[test]
fn advanced_effects_accept_supported_rates_and_reject_invalid_preparation() {
    for rate in [8000.0, 44100.0, 48000.0, 96000.0, 192000.0] {
        for kind in kinds() {
            let mut effect = AutomatedEffect::new(kind, "edge").unwrap();
            effect
                .prepare(ProcessSpec {
                    sample_rate: rate,
                    max_block_frames: 73,
                    channels: 2,
                })
                .unwrap();
            for p in effect.parameters() {
                p.set(p.info().max).unwrap();
            }
            let mut seed = 12345_u32;
            let mut data = [0.0; 146];
            for _ in 0..64 {
                for x in &mut data {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    *x = (seed as f32 / u32::MAX as f32 - 0.5) * 0.1;
                }
                effect.process(
                    &mut AudioBuffer::new(&mut data, 2).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                );
                assert!(data.iter().all(|x| x.is_finite()), "{kind:?} at {rate}");
            }
            effect.reset();
        }
    }
    for rate in [7999.0, 192001.0] {
        let mut p = AutomatedEffect::new(EffectKind::LookaheadLimiter, "bad").unwrap();
        assert!(p
            .prepare(ProcessSpec {
                sample_rate: rate,
                ..sp()
            })
            .is_err());
        let mut data = [1.0; 8];
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
        assert_eq!(data, [0.0; 8]);
    }
    for (rate, delay) in [
        (44100.0, 233),
        (48000.0, 252),
        (96000.0, 492),
        (192000.0, 972),
    ] {
        let mut p = AutomatedEffect::new(EffectKind::LookaheadLimiter, "delay").unwrap();
        p.prepare(ProcessSpec {
            sample_rate: rate,
            ..sp()
        })
        .unwrap();
        assert_eq!(p.latency_frames(), delay);
    }
    assert!(LoudnessMeter::new(44100, 2, 64).is_err());
    assert!(LoudnessMeter::new(48000, 6, 64).is_err());
}
