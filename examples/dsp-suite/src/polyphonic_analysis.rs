use super::*;
use soniclaw_dsp::{
    midi_hz,
    polyphonic_analysis::{transcribe, Settings},
};
use std::fmt::Write as _;

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let (audio, rate, fixture) = match args {
        [] => {
            let mut pcm = vec![0.; 72_000];
            for (keys, start, end) in [
                ([57, 64, 71], 6_000, 30_000),
                ([53, 60, 69], 36_000, 65_000),
            ] {
                for (i, sample) in pcm[start..end].iter_mut().enumerate() {
                    let time = i as f32 / 48_000.;
                    let ramp = (i.min(end - start - i) as f32 / 256.).min(1.);
                    for (j, midi) in keys.into_iter().enumerate() {
                        let phase = std::f32::consts::TAU * midi_hz(midi) * time;
                        *sample += ramp
                            * 0.12
                            * (phase.sin()
                                + 0.25 * (2. * phase).sin()
                                + 0.14 * (3. * phase + j as f32).sin());
                    }
                }
            }
            crate::ir_capture::ir_wav(out.join("transcription-input.wav"), &pcm, 48_000)?;
            (pcm, 48_000, true)
        }
        [wav] => {
            let (audio, rate) = crate::ir_capture::read_mono(wav)?;
            (audio, rate, false)
        }
        _ => return Err("expected at most one mono WAV path".into()),
    };
    let notes = transcribe(
        &audio,
        Settings {
            sample_rate: rate,
            ..Settings::default()
        },
    )?;
    if fixture {
        for key in [57, 64, 71, 53, 60, 69] {
            assert!(
                notes
                    .iter()
                    .any(|n| n.midi == key && n.end_frame - n.start_frame > 15_000),
                "missing {key}"
            );
        }
    }
    let mut csv = String::from("midi,start_frame,end_frame,start_seconds,end_seconds,confidence\n");
    for n in &notes {
        writeln!(
            csv,
            "{},{},{},{:.6},{:.6},{:.4}",
            n.midi,
            n.start_frame,
            n.end_frame,
            n.start_frame as f64 / f64::from(rate),
            n.end_frame as f64 / f64::from(rate),
            n.confidence
        )?;
    }
    std::fs::write(out.join("note-events.csv"), csv)?;
    Ok(serde_json::json!({"notes":notes.len(),"sample_rate":rate,"fixture":fixture}))
}
