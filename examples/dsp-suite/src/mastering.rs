use super::*;
use soniclaw_dsp::mastering::{render_master, Options, MAX_FRAMES};

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let mut options = Options::default();
    let (audio, fixture) = match args {
        [] => {
            let mut samples = Vec::with_capacity(288_000);
            for i in 0..144_000 {
                let phase = std::f64::consts::TAU * 1000. * f64::from(i) / 48_000.;
                let left = (0.06 * phase.sin()) as f32;
                samples.extend_from_slice(&[left, 0.75 * left]);
            }
            samples[2 * 96_010] = 0.7;
            (
                soniclaw::io::AudioFile {
                    sample_rate: 48_000,
                    channels: 2,
                    samples,
                },
                true,
            )
        }
        [file, target, ceiling] => {
            options.target_lufs = target.parse()?;
            options.peak_ceiling_dbtp = ceiling.parse()?;
            (
                soniclaw::io::read_wav_with_limit(file, MAX_FRAMES * 8)?,
                false,
            )
        }
        _ => return Err("expected INPUT.wav TARGET_LUFS CEILING_DBTP".into()),
    };
    let output = render_master(
        &audio.samples,
        audio.sample_rate,
        usize::from(audio.channels),
        options,
    )?;
    if fixture {
        assert!(output.report.met_target, "{:?}", output.report);
    }
    assert_eq!(output.samples.len(), audio.samples.len());
    let mut writer = hound::WavWriter::create(
        out.join("mastered.wav"),
        hound::WavSpec {
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for &sample in &output.samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(
        serde_json::json!({"channels":audio.channels,"sample_rate":audio.sample_rate,
        "frames":audio.frames(),"measurements":output.report}),
    )
}
