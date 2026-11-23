use soniclaw::{prelude::*, sampler::*, tuning::Tuning};
use std::sync::Arc;
fn table() -> Arc<Tuning> {
    let mut hz = [None; 128];
    hz[69] = Some(432.);
    Arc::new(Tuning::new("A432 only", hz).unwrap())
}
fn frequency(data: &[f32]) -> f64 {
    let mut crossings = Vec::new();
    for (i, w) in data.windows(2).enumerate().skip(1000) {
        if w[0] <= 0. && w[1] > 0. {
            crossings.push(i as f64 + (-w[0] / (w[1] - w[0])) as f64);
        }
    }
    assert!(crossings.len() > 20);
    48000. * (crossings.len() - 1) as f64 / (crossings.last().unwrap() - crossings[0])
}
fn render(processor: &mut dyn AudioProcessor) -> Vec<f32> {
    processor
        .prepare(ProcessSpec {
            sample_rate: 48000.,
            channels: 1,
            max_block_frames: 256,
        })
        .unwrap();
    let note = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 69,
            velocity: 127,
        },
    }];
    let mut out = vec![0.; 24000];
    let alloc = allocation_counter::measure(|| {
        for (i, s) in out.chunks_mut(256).enumerate() {
            processor.process(
                &mut AudioBuffer::new(s, 1).unwrap(),
                &ProcessContext {
                    events: if i == 0 { &note } else { &[] },
                    transport: Transport {
                        sample_position: (i * 256) as u64,
                        ..Default::default()
                    },
                },
            );
        }
    });
    assert_eq!(alloc.count_total, 0);
    out
}
#[test]
fn actual_synth_and_sampler_use_absolute_tuning_without_allocations() {
    let mut synth = ExpressiveSynth::<8>::new(
        Waveform::Sine,
        AdsrSettings {
            attack: 0.,
            decay: 0.,
            sustain: 1.,
            release: 0.01,
        },
    )
    .unwrap();
    synth.timbre_depth = 0.;
    synth.set_tuning(Some(table())).unwrap();
    let pcm = render(&mut synth);
    assert!((frequency(&pcm) - 432.).abs() < 0.05);
    let sample = soniclaw::dsp::Sample::new(
        (0..48000)
            .map(|i| (i as f32 * 440. * std::f32::consts::TAU / 48000.).sin())
            .collect::<Vec<_>>(),
        1,
        48000.,
        69,
    )
    .unwrap();
    let bank = SampleBank::new(vec![sample], vec![Zone::single(0, 69)], 1024 * 1024).unwrap();
    let mut sampler = MultiSampler::<8>::new(Arc::new(bank));
    sampler.set_tuning(Some(table())).unwrap();
    let pcm = render(&mut sampler);
    assert!((frequency(&pcm) - 432.).abs() < 0.05);
    synth.reset();
    let mut pcm = [0.; 256];
    synth.process(
        &mut AudioBuffer::new(&mut pcm, 1).unwrap(),
        &ProcessContext {
            events: &[Event {
                frame: 0,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 127,
                },
            }],
            transport: Default::default(),
        },
    );
    assert_eq!(synth.active_voices(), 0);
    assert!(pcm.iter().all(|v| *v == 0.));
}
#[test]
fn project_persists_tuning_and_default_projects_omit_it() {
    let mut p = soniclaw::state::ProjectState::empty("test", "tuning");
    assert!(
        serde_json::from_str::<serde_json::Value>(&p.to_json().unwrap())
            .unwrap()
            .get("tuning")
            .is_none()
    );
    p.tuning = Some((*table()).clone());
    let p = soniclaw::state::ProjectState::from_json(&p.to_json().unwrap()).unwrap();
    assert_eq!(p.tuning.unwrap().frequency(69), Some(432.));
}
