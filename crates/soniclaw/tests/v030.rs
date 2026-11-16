//! Processing paths added in v0.3: count allocations including voice stealing,
//! long-IR partition transitions, reset, and changing callback sizes.
use soniclaw::{dsp::*, prelude::*, sampler::*};
use std::sync::Arc;
#[test]
fn new_effects_and_multisampler_do_not_allocate_in_callbacks() {
    let bank = Arc::new(
        SampleBank::new(
            vec![Sample::new(vec![0.5; 48000], 1, 48000.0, 60).unwrap()],
            vec![Zone::single(0, 60)],
            192000,
        )
        .unwrap(),
    );
    let mut processors: Vec<Box<dyn AudioProcessor>> = vec![
        Box::new(
            ParametricEq::new(vec![EqBand {
                shape: EqShape::Peak,
                frequency: 2000.0,
                q: 0.7,
                gain_db: 3.0,
            }])
            .unwrap(),
        ),
        Box::new(Compressor::new(Default::default()).unwrap()),
        Box::new(PeakLimiter::new(-1.0, 100.0).unwrap()),
        Box::new(NoiseGate::new(-50.0, 1.0, 10.0, 50.0).unwrap()),
        Box::new(Chorus::new(0.5, 4.0, 0.3).unwrap()),
        Box::new(Reverb::new(1.0, 0.3).unwrap()),
        Box::new(Convolution::new(vec![0.001; 4096], 48000.0, 128).unwrap()),
        Box::new(OversampledDrive::new(2.0, 1.0).unwrap()),
        Box::new(MultiSampler::<4>::new(bank)),
    ];
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 257,
        channels: 2,
    };
    let events = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    }];
    let context = ProcessContext {
        events: &events,
        transport: Transport::default(),
    };
    for p in &mut processors {
        p.prepare(spec).unwrap();
        let mut audio = [0.1; 514];
        let allocations = allocation_counter::measure(|| {
            for n in 0..100 {
                let len = if n % 2 == 0 { 514 } else { 126 };
                audio[..len].fill(0.1);
                p.process(
                    &mut AudioBuffer::new(&mut audio[..len], 2).unwrap(),
                    &context,
                );
                assert!(audio[..len].iter().all(|x| x.is_finite()));
            }
            p.reset();
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
    }
}
#[test]
fn optimized_gain_ramp_is_block_partition_independent() {
    fn render(block: usize) -> Vec<f32> {
        let gain = Arc::new(
            Parameter::new(ParameterInfo {
                id: "g",
                name: "Gain",
                min: 0.0,
                max: 1.0,
                default: 0.5,
                unit: "",
            })
            .unwrap(),
        );
        let mut g = Gain::new(gain.clone());
        g.prepare(ProcessSpec {
            sample_rate: 48000.0,
            max_block_frames: block,
            channels: 2,
        })
        .unwrap();
        gain.set(0.9).unwrap();
        let mut data = vec![1.0; 4096];
        for chunk in data.chunks_mut(block * 2) {
            g.process(
                &mut AudioBuffer::new(chunk, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport::default(),
                },
            );
        }
        data
    }
    assert_eq!(render(64), render(257));
}
