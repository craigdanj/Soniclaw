use super::*;
use soniclaw_dsp::hpss::{separate, Settings};
pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let file = match args {
        [] => {
            let mut samples = Vec::new();
            let mut seed = 74u32;
            for i in 0..48_000 {
                let t = i as f32 / 48_000.;
                let tone = 0.16 * (std::f32::consts::TAU * 375. * t).sin()
                    + 0.08 * (std::f32::consts::TAU * 562.5 * t).sin();
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let transient = if i % 12_000 >= 7000 && i % 12_000 < 7240 {
                    0.65 * (seed as f32 / u32::MAX as f32 - 0.5)
                        * (-0.02 * (i % 12_000 - 7000) as f32).exp()
                } else {
                    0.
                };
                samples.extend_from_slice(&[tone + transient, 0.85 * tone - 0.6 * transient]);
            }
            soniclaw::io::AudioFile {
                sample_rate: 48_000,
                channels: 2,
                samples,
            }
        }
        [file] => soniclaw::io::read_wav_with_limit(file, 64 * 1024 * 1024)?,
        _ => return Err("expected at most one mono or stereo WAV file".into()),
    };
    let stems = separate(
        &file.samples,
        usize::from(file.channels),
        Settings::default(),
    )?;
    let error = file
        .samples
        .iter()
        .zip(stems.harmonic.iter().zip(&stems.percussive))
        .map(|(x, (h, p))| (x - (h + p)).abs())
        .fold(0f32, f32::max);
    assert!(error < 2e-6);
    for (name, data) in [
        ("original", &file.samples),
        ("harmonic", &stems.harmonic),
        ("percussive", &stems.percussive),
    ] {
        let mut w = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                sample_rate: file.sample_rate,
                channels: file.channels,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for &x in data {
            w.write_sample(x)?;
        }
        w.finalize()?;
    }
    Ok(
        serde_json::json!({"channels":file.channels,"frames":file.frames(),"max_recombination_error":error}),
    )
}
