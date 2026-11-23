use super::*;
use soniclaw_dsp::{advanced::wavetable::WavetableBank, hybrid_voice::*, AdsrSettings, Sample};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let patch = Patch {
        schema: 1,
        voices: 32,
        wavetable_gain: 0.045,
        sample_gain: 0.02,
        morph: 0.35,
        cutoff_hz: 6500.,
        lfo_hz: 0.4,
        morph_depth: 0.3,
        envelope: AdsrSettings {
            attack: 0.005,
            decay: 0.1,
            sustain: 0.7,
            release: 0.08,
        },
    };
    let round_trip = serde_json::from_value(serde_json::to_value(patch)?)?;
    let asset = Arc::new(Sample::new(
        (0..4000)
            .map(|i| {
                let t = i as f32 / 48000.;
                (std::f32::consts::TAU * 660. * t).sin() * (-t * 85.).exp()
            })
            .collect::<Vec<_>>(),
        1,
        48000.,
        69,
    )?);
    let bank = Arc::new(WavetableBank::basic(1024)?);
    let mut synth = HybridSynth::new(round_trip, bank, asset)?;
    synth.prepare(spec())?;
    let mut data = vec![0.; 48000];
    let events: Vec<_> = (0..32)
        .map(|i| Event {
            frame: 17,
            message: MidiMessage::NoteOn {
                channel: 0,
                note: 44 + i,
                velocity: 65,
            },
        })
        .collect();
    let steal = [Event {
        frame: 64,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 81,
            velocity: 71,
        },
    }];
    let release = [Event {
        frame: 64,
        message: MidiMessage::AllNotesOff,
    }];
    let start = std::time::Instant::now();
    let alloc = allocation_counter::measure(|| {
        for (i, b) in data.chunks_mut(128).enumerate() {
            let e: &[Event] = if i == 0 {
                events.as_slice()
            } else if i == 60 {
                &steal
            } else if i == 240 {
                &release
            } else {
                &[]
            };
            synth.process(
                &mut AudioBuffer::new(b, 1).unwrap(),
                &ProcessContext {
                    events: e,
                    ..context(i * 128)
                },
            );
            if i == 0 || i == 60 {
                assert_eq!(synth.active_voices(), 32);
            }
        }
    })
    .count_total;
    let time = start.elapsed().as_micros();
    assert_eq!(alloc, 0);
    assert!(data[..18].iter().all(|x| *x == 0.));
    assert!(data[49..128].iter().any(|x| x.abs() > 1e-5));
    assert_eq!(synth.active_voices(), 0);
    let mut w = hound::WavWriter::create(
        out.join("hybrid-polyphony.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for x in data {
        assert!(x.is_finite());
        w.write_sample(x)?;
    }
    w.finalize()?;
    Ok(
        serde_json::json!({"max_voices":32,"elapsed_us_for_48000_frames":time,"measured_callback_allocations":alloc,"optimized":!cfg!(debug_assertions),"first_note_frame":17}),
    )
}
