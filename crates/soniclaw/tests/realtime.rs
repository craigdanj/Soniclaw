//! Behavioral regression: exercise an actual voice/filter/delay/gain chain,
//! changing parameters, events, and block lengths under an allocation counter.
use soniclaw::prelude::*;
use std::sync::Arc;

#[test]
fn prepared_chain_does_not_allocate_while_processing() {
    let gain = Arc::new(
        Parameter::new(ParameterInfo {
            id: "gain",
            name: "Gain",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: "",
        })
        .unwrap(),
    );
    let cutoff = Arc::new(
        Parameter::new(ParameterInfo {
            id: "cutoff",
            name: "Cutoff",
            min: 20.0,
            max: 20000.0,
            default: 3000.0,
            unit: "Hz",
        })
        .unwrap(),
    );
    let mut engine = Engine::default();
    engine.push(PolySynth::<16>::default());
    engine.push(LowPass::new(cutoff.clone()));
    engine.push(Delay::new(0.1, 0.3, 0.2).unwrap());
    engine.push(Gain::new(gain.clone()));
    engine
        .prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 256,
            channels: 2,
        })
        .unwrap();
    let mut audio = [0.0; 512];
    let events = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    }];
    let result = allocation_counter::measure(|| {
        for block in 0..200 {
            let length = if block % 2 == 0 { 512 } else { 126 };
            gain.set(0.3 + (block % 3) as f32 * 0.1).unwrap();
            cutoff.set(1000.0 + block as f32 * 10.0).unwrap();
            engine
                .process(
                    &mut audio[..length],
                    if block % 10 == 0 { &events } else { &[] },
                )
                .unwrap();
            assert!(audio[..length].iter().all(|x| x.is_finite()));
        }
        engine.reset();
    });
    assert_eq!(
        result.count_total, 0,
        "allocations during real-time processing"
    );
    assert_eq!(result.count_current, 0);
}

#[test]
fn offline_audio_is_independent_of_block_partition() {
    fn make(size: usize) -> Engine {
        let mut e = Engine::default();
        e.push(PolySynth::<8>::default());
        e.push(Delay::new(0.01, 0.2, 0.3).unwrap());
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: size,
            channels: 2,
        })
        .unwrap();
        e
    }
    let events = [
        Event {
            frame: 31,
            message: MidiMessage::NoteOn {
                channel: 1,
                note: 69,
                velocity: 100,
            },
        },
        Event {
            frame: 999,
            message: MidiMessage::NoteOff {
                channel: 1,
                note: 69,
                velocity: 0,
            },
        },
    ];
    let a = soniclaw::engine::render(&mut make(64), 15000, &events).unwrap();
    let b = soniclaw::engine::render(&mut make(257), 15000, &events).unwrap();
    assert_eq!(a, b);
}

#[test]
fn prepared_sampler_and_parallel_graph_do_not_allocate() {
    let sample = Sample::new(vec![0.5; 48000], 1, 48000.0, 60).unwrap();
    let mut graph = soniclaw::graph::Graph::default();
    let instrument = graph.add(Sampler::<8>::new(sample));
    let delay = graph.add(Delay::new(0.05, 0.2, 0.5).unwrap());
    graph.connect(instrument, delay, 0.5).unwrap();
    graph.set_output(instrument, 0.5).unwrap();
    graph.set_output(delay, 0.5).unwrap();
    let mut engine = Engine::default();
    engine.push(graph);
    engine
        .prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: 256,
            channels: 2,
        })
        .unwrap();
    let events = [Event {
        frame: 31,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 64,
            velocity: 100,
        },
    }];
    let mut audio = [0.0; 512];
    let result = allocation_counter::measure(|| {
        engine.process(&mut audio, &events).unwrap();
        for _ in 0..100 {
            engine.process(&mut audio, &[]).unwrap();
        }
    });
    assert_eq!(result.count_total, 0);
    assert!(audio.iter().all(|s| s.is_finite()));
}

#[test]
fn automation_and_record_enqueue_do_not_allocate() {
    let parameter = Arc::new(
        Parameter::new(ParameterInfo {
            id: "gain",
            name: "Gain",
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: "",
        })
        .unwrap(),
    );
    let mut engine = Engine::default();
    engine.push(Gain::new(parameter.clone()));
    engine.bind_parameter(9, parameter).unwrap();
    engine
        .prepare(ProcessSpec {
            sample_rate: 48000.0,
            channels: 2,
            max_block_frames: 256,
        })
        .unwrap();
    let (mut recorder, mut tap) = soniclaw::record::Recorder::new(32);
    let path = std::env::temp_dir().join(format!(
        "soniclaw-allocation-record-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    recorder
        .start(&path, 48000, soniclaw::record::RecordSource::Processed)
        .unwrap();
    let automation = [
        AutomationEvent {
            frame: 31,
            parameter: 9,
            value: 0.25,
        },
        AutomationEvent {
            frame: 128,
            parameter: 9,
            value: 0.75,
        },
    ];
    let mut audio = [0.1; 512];
    let dry = audio;
    let measured = allocation_counter::measure(|| {
        for _ in 0..100 {
            engine
                .process_automated(&mut audio, &[], &automation)
                .unwrap();
            tap.capture_pair(&dry, &audio);
        }
    });
    recorder.stop().unwrap();
    assert_eq!(measured.count_total, 0);
    assert_eq!(
        recorder
            .stats()
            .captured_frames
            .load(std::sync::atomic::Ordering::Relaxed),
        25600
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn offline_automation_is_partition_independent() {
    fn make(block: usize) -> Engine {
        let p = Arc::new(
            Parameter::new(ParameterInfo {
                id: "gain",
                name: "Gain",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                unit: "",
            })
            .unwrap(),
        );
        let mut e = Engine::default();
        e.push(PolySynth::<4>::default());
        e.push(Gain::new(p.clone()));
        e.bind_parameter(42, p).unwrap();
        e.prepare(ProcessSpec {
            sample_rate: 48000.0,
            channels: 2,
            max_block_frames: block,
        })
        .unwrap();
        e
    }
    let midi = [Event {
        frame: 7,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    }];
    let automation = [
        AutomationEvent {
            frame: 31,
            parameter: 42,
            value: 0.8,
        },
        AutomationEvent {
            frame: 511,
            parameter: 42,
            value: 0.2,
        },
        AutomationEvent {
            frame: 1023,
            parameter: 42,
            value: 0.6,
        },
    ];
    let a = soniclaw::engine::render_automated(&mut make(64), 1500, &midi, &automation).unwrap();
    let b = soniclaw::engine::render_automated(&mut make(257), 1500, &midi, &automation).unwrap();
    assert_eq!(a, b);
}
