use soniclaw::{prelude::*, tuning::*};
use std::sync::Arc;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-microtuning NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let scale = Scale::from_scl("Just major triad\n3\n5/4\n3/2\n2/1")?;
    let tuning = Tuning::from_scala(&scale, &KeyboardMapping::linear(60, 256.)?)?;
    assert!((tuning.frequency(61).unwrap() - 320.).abs() < 1e-9);
    std::fs::write(path.join("tuning.json"), tuning.to_json()?)?;
    let mut synth = ExpressiveSynth::<8>::new(
        Waveform::Sine,
        AdsrSettings {
            attack: 0.005,
            decay: 0.05,
            sustain: 0.7,
            release: 0.2,
        },
    )?;
    synth.timbre_depth = 0.;
    synth.set_tuning(Some(Arc::new(tuning.clone())))?;
    synth.prepare(ProcessSpec {
        sample_rate: 48000.,
        channels: 2,
        max_block_frames: 256,
    })?;
    let notes: Vec<_> = [60, 61, 62]
        .into_iter()
        .map(|note| Event {
            frame: 0,
            message: MidiMessage::NoteOn {
                channel: 0,
                note,
                velocity: 70,
            },
        })
        .collect();
    let mut pcm = vec![0.; 48000 * 2];
    for (i, b) in pcm.chunks_mut(512).enumerate() {
        synth.process(
            &mut AudioBuffer::new(b, 2)?,
            &ProcessContext {
                events: if i == 0 { &notes } else { &[] },
                transport: Default::default(),
            },
        );
    }
    assert!(pcm.iter().any(|v| v.abs() > 0.01));
    soniclaw::io::write_wav(path.join("just-triad.wav"), 48000, 2, &pcm)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"notes":[60,61,62],"frequencies_hz":[tuning.frequency(60),tuning.frequency(61),tuning.frequency(62)],"frames":48000}),
        )?,
    )?;
    Ok(())
}
