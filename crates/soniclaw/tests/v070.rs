use soniclaw::{
    dsp::{modulation::LfoShape, voice_mod::*},
    graph::live::*,
    midi::performance::*,
    prelude::*,
    sampler::*,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
fn spec(n: usize) -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: n,
        channels: 2,
    }
}
fn ctx(e: &[Event]) -> ProcessContext<'_> {
    ProcessContext {
        events: e,
        transport: Transport {
            playing: true,
            ..Default::default()
        },
    }
}
#[test]
fn voice_mpe_render_is_partition_independent_and_allocation_free() {
    let cfg = VoiceModulationConfig {
        lfos: [
            VoiceLfo {
                shape: LfoShape::Sine,
                rate_hz: 4.0,
            },
            VoiceLfo {
                shape: LfoShape::Triangle,
                rate_hz: 0.5,
            },
        ],
        routes: vec![
            VoiceRoute {
                source: VoiceSource::Lfo1,
                destination: VoiceDestination::Pitch,
                amount: 0.01,
            },
            VoiceRoute {
                source: VoiceSource::Pressure,
                destination: VoiceDestination::Cutoff,
                amount: 0.3,
            },
        ],
        ..Default::default()
    };
    let e = [
        Event {
            frame: 3,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id: 7,
            },
        },
        Event {
            frame: 173,
            message: MidiMessage::NoteOnId {
                channel: 2,
                note: 67,
                velocity: 0.6,
                note_id: 8,
            },
        },
        Event {
            frame: 817,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    note_id: 7,
                    channel: -1,
                    key: -1,
                },
                expression: NoteExpression::Pressure,
                value: 0.9,
            },
        },
        Event {
            frame: 2047,
            message: MidiMessage::NoteOffId {
                target: NoteTarget {
                    note_id: 8,
                    channel: -1,
                    key: -1,
                },
            },
        },
    ];
    let render = |n| {
        let mut engine = Engine::default();
        let mut s = PolyModSynth::<8>::new("poly", cfg.clone()).unwrap();
        s.synth
            .set_mpe_zone(Some(MpeZone {
                master: 0,
                first_member: 1,
                last_member: 15,
                member_bend: 48.0,
                master_bend: 2.0,
            }))
            .unwrap();
        engine.push(s);
        engine.prepare(spec(n)).unwrap();
        soniclaw::engine::render(&mut engine, 8192, &e).unwrap()
    };
    let a = render(64);
    let b = render(257);
    assert!(a.iter().any(|x| x.abs() > 0.001));
    assert!(a.iter().zip(b).all(|(a, b)| (*a - b).abs() < 1e-6));
    let mut p = PolyModSynth::<8>::new("poly", cfg).unwrap();
    p.prepare(spec(128)).unwrap();
    let on = [Event {
        frame: 0,
        message: e[0].message,
    }];
    let mut data = [0.0; 256];
    let alloc = allocation_counter::measure(|| {
        for i in 0..64 {
            p.process(
                &mut AudioBuffer::new(&mut data, 2).unwrap(),
                &ctx(if i == 0 { &on } else { &[] }),
            );
        }
        p.reset();
    });
    assert_eq!(alloc.count_total, 0);
    assert_eq!(alloc.count_current, 0);
}
struct Gain {
    value: f32,
    drops: Arc<AtomicUsize>,
}
impl Drop for Gain {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}
impl AudioProcessor for Gain {
    fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        for x in b.samples_mut() {
            *x *= self.value;
        }
    }
    fn reset(&mut self) {}
}
#[test]
fn graph_swap_warms_and_defers_retirement_with_queue_backpressure() {
    let drops = Arc::new(AtomicUsize::new(0));
    let make = |value| {
        Box::new(Gain {
            value,
            drops: drops.clone(),
        }) as Box<dyn AudioProcessor>
    };
    let (mut ctl, mut p) = graph_pair(make(1.0), spec(64), 64, 64).unwrap();
    let mut d = [0.2; 128];
    p.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&[]));
    ctl.replace(make(0.5)).unwrap();
    let a = allocation_counter::measure(|| {
        for i in 0..4 {
            d.fill(0.2);
            p.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&[]));
            assert!(d.iter().all(|x| *x >= 0.09999 && *x <= 0.20001));
            if i == 0 {
                assert!(d.iter().all(|x| (*x - 0.2).abs() < 1e-6));
            }
        }
    });
    assert_eq!(a.count_total, 0);
    assert_eq!(a.count_current, 0);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert_eq!(ctl.collect_retired(), 1);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(p.latency_frames(), 64);
    ctl.replace(make(0.7)).unwrap();
    ctl.replace(make(0.8)).unwrap();
    assert!(ctl.replace(make(0.9)).is_err());
}
#[test]
fn sampler_regions_bound_reads_and_loop_fades_reduce_the_wrap_jump() {
    let sample = Sample::new(
        (0..1024)
            .map(|i| if i < 512 { 0.25 } else { -0.5 })
            .collect::<Vec<_>>(),
        1,
        48000.0,
        60,
    )
    .unwrap();
    let mut z = Zone::single(0, 60);
    z.start_frame = 512;
    z.end_frame = Some(1024);
    let mut p = MultiSampler::<4>::new(Arc::new(
        SampleBank::new(vec![sample], vec![z], 100000).unwrap(),
    ));
    p.set_quality(soniclaw::dsp::resample::ResampleQuality::Linear);
    p.prepare(spec(1024)).unwrap();
    let on = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 127,
        },
    }];
    let mut d = [0.0; 2048];
    p.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&on));
    assert!(d[..1024].iter().all(|x| (*x + 0.5).abs() < 1e-6));
    assert!(d[1024..].iter().all(|x| *x == 0.0));
    let render = |fade| {
        let sample = Sample::new(
            (0..120)
                .map(|i| if i < 60 { 0.25 } else { -0.25 })
                .collect::<Vec<_>>(),
            1,
            48000.0,
            60,
        )
        .unwrap();
        let mut z = Zone::single(0, 60);
        z.one_shot = false;
        z.loop_frames = Some((0, 120));
        z.loop_crossfade = fade;
        let mut p = MultiSampler::<2>::new(Arc::new(
            SampleBank::new(vec![sample], vec![z], 10000).unwrap(),
        ));
        p.set_quality(soniclaw::dsp::resample::ResampleQuality::Linear);
        p.prepare(spec(256)).unwrap();
        let mut d = [0.0; 512];
        p.process(&mut AudioBuffer::new(&mut d, 2).unwrap(), &ctx(&on));
        (d[240] - d[238]).abs()
    };
    assert!(render(20) < render(0) * 0.2);
}
#[test]
fn short_humanized_notes_release_and_capacity_overflow_panics_safely() {
    let mut p = MidiPerformance::new(
        PerformanceConfig {
            chord: vec![0, 4, 7, 11],
            humanize_ms: 50.0,
            seed: 123,
            ..Default::default()
        },
        48000.0,
    )
    .unwrap();
    let e = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id: 7,
            },
        },
        Event {
            frame: 1,
            message: MidiMessage::NoteOffId {
                target: NoteTarget {
                    note_id: 7,
                    channel: 1,
                    key: 60,
                },
            },
        },
    ];
    let mut out = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; 128];
    let mut all = vec![];
    for at in (0..4096).step_by(128) {
        let n = p
            .process(
                128,
                if at == 0 { &e } else { &[] },
                Transport {
                    sample_position: at as u64,
                    playing: true,
                    ..Default::default()
                },
                &mut out,
            )
            .unwrap();
        all.extend_from_slice(&out[..n]);
    }
    assert_eq!(all.len(), 8);
    for i in 0..4 {
        assert!(matches!(all[i].message, MidiMessage::NoteOnId { .. }));
        assert!(matches!(all[i + 4].message, MidiMessage::NoteOffId { .. }));
    }
    p.reset();
    let mut out = [Event {
        frame: 0,
        message: MidiMessage::AllNotesOff,
    }; 1];
    assert_eq!(
        p.process(
            4096,
            &e,
            Transport {
                playing: true,
                ..Default::default()
            },
            &mut out
        )
        .unwrap(),
        1
    );
    assert_eq!(out[0].message, MidiMessage::AllNotesOff);
    assert!(p.overflows > 0);
}
