use soniclaw_core::MidiMessage;
use soniclaw_device::{event_channel, OutputHost, Telemetry};
use soniclaw_dsp::{
    advanced::wavetable::WavetableBank,
    hybrid_voice::{HybridSynth, Patch},
    AdsrSettings, Sample,
};
use soniclaw_engine::Engine;
use std::{io, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--help") {
        println!("Play a wavetable and pluck patch through the default audio output. Enter on NOTE VELOCITY, off NOTE, panic, or quit.");
        return Ok(());
    }
    let patch = Patch {
        schema: 1,
        voices: 16,
        wavetable_gain: 0.055,
        sample_gain: 0.02,
        morph: 0.36,
        cutoff_hz: 6400.,
        lfo_hz: 0.6,
        morph_depth: 0.15,
        envelope: AdsrSettings {
            attack: 0.006,
            decay: 0.13,
            sustain: 0.7,
            release: 0.15,
        },
    };
    let samples = (0..4000)
        .map(|i| {
            let t = i as f32 / 48000.;
            (std::f32::consts::TAU * 660. * t).sin() * (-t * 85.).exp()
        })
        .collect::<Vec<_>>();
    let asset = Arc::new(Sample::new(samples, 1, 48000., 69)?);
    let bank = Arc::new(WavetableBank::basic(1024)?);
    let mut engine = Engine::default();
    engine.push(HybridSynth::new(patch, bank, asset)?);
    let telemetry = Arc::new(Telemetry::default());
    let (mut sender, receiver) = event_channel(256, telemetry.clone());
    let output = OutputHost::start(engine, None, 256, vec![receiver], telemetry)?;
    println!(
        "Playing through {} at {} Hz.",
        output.device_name, output.spec.sample_rate
    );
    println!("Enter on 60 100, off 60, panic, or quit.");
    for line in io::stdin().lines() {
        let line = line?;
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        let note = |s: &str| s.parse::<u8>().ok().filter(|n| *n <= 127);
        let message = match tokens.as_slice() {
            ["on", key, velocity] => {
                note(key)
                    .zip(note(velocity))
                    .map(|(note, velocity)| MidiMessage::NoteOn {
                        channel: 0,
                        note,
                        velocity,
                    })
            }
            ["off", key] => note(key).map(|note| MidiMessage::NoteOff {
                channel: 0,
                note,
                velocity: 0,
            }),
            ["panic"] => Some(MidiMessage::AllNotesOff),
            ["quit"] => break,
            [] => continue,
            _ => None,
        };
        if let Some(message) = message {
            if !sender.send(message) {
                eprintln!("Event queue full.");
            }
        } else {
            eprintln!("Use note and velocity values from 0 to 127.");
        }
    }
    Ok(())
}
