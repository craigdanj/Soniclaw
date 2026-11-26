//! Hardware-independent end-to-end instrument rendering and preset example.
use soniclaw::prelude::*;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "soniclaw-demo.wav".into());
    let gain = Arc::new(Parameter::new(ParameterInfo {
        id: "master.gain",
        name: "Master",
        min: 0.0,
        max: 1.0,
        default: 0.6,
        unit: "",
    })?);
    let cutoff = Arc::new(Parameter::new(ParameterInfo {
        id: "filter.cutoff",
        name: "Cutoff",
        min: 20.0,
        max: 20000.0,
        default: 2600.0,
        unit: " Hz",
    })?);
    let mut engine = Engine::default();
    engine.push(PolySynth::<16>::default());
    engine.push(LowPass::new(cutoff.clone()));
    engine.push(Delay::new(0.3, 0.32, 0.24)?);
    engine.push(Gain::new(gain.clone()));
    engine.prepare(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    })?;
    let mut events = Vec::new();
    for (beat, notes) in [[60, 64, 67], [57, 60, 64], [53, 57, 60], [55, 59, 62]]
        .iter()
        .enumerate()
    {
        for &note in notes {
            events.push(Event {
                frame: beat * 48000,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note,
                    velocity: 96,
                },
            });
            events.push(Event {
                frame: beat * 48000 + 36000,
                message: MidiMessage::NoteOff {
                    channel: 0,
                    note,
                    velocity: 0,
                },
            });
        }
    }
    events.sort_by_key(|e| e.frame);
    let audio = soniclaw::engine::render(&mut engine, 6 * 48000, &events)?;
    soniclaw::io::write_wav(&output, 48000, 2, &audio)?;
    let preset = soniclaw::state::Preset::capture("org.soniclaw.demo", "Warm Room", &[gain, cutoff])?;
    let preset_path = std::path::Path::new(&output).with_extension("preset.json");
    std::fs::write(&preset_path, preset.to_json()?)?;
    let peak = audio.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
    println!(
        "Rendered {} stereo frames (6 s, 48 kHz), peak {peak:.3}\nWAV: {output}\nPreset: {}",
        audio.len() / 2,
        preset_path.display()
    );
    Ok(())
}
