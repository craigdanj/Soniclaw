use super::*;
use soniclaw_dsp::stereo_panner::*;

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut reports = Vec::new();
    for (name, law) in [
        ("linear", PanLaw::Linear),
        ("balanced", PanLaw::Balanced),
        ("sine-3", PanLaw::Sine3Db),
        ("sine-4p5", PanLaw::Sine4_5Db),
        ("sine-6", PanLaw::Sine6Db),
        ("square-root-3", PanLaw::SquareRoot3Db),
        ("square-root-4p5", PanLaw::SquareRoot4_5Db),
    ] {
        let mut processor = StereoPanner::new(law, -1., 8192)?;
        processor.set_pan(1.)?;
        let input: Vec<_> = (0..8192).map(|i| 0.55 * (i as f32 * 0.072).sin()).collect();
        let mut stereo = vec![0.; 16384];
        let mut allocations = 0;
        for (mono, stereo) in input.chunks(128).zip(stereo.chunks_mut(256)) {
            let mut audio = AudioBuffer::new(stereo, 2)?;
            allocations +=
                allocation_counter::measure(|| processor.process_mono(mono, &mut audio).unwrap())
                    .count_total;
        }
        assert_eq!(allocations, 0);
        assert_eq!(processor.pan(), 1.);
        let file = format!("{name}.wav");
        let mut wav = hound::WavWriter::create(
            out.join(&file),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for sample in stereo {
            wav.write_sample(sample)?;
        }
        wav.finalize()?;
        reports.push(serde_json::json!({
            "audio":file, "law":name, "callback_allocations":allocations,
            "center_gain":law.gains(0.)?, "center_db":20. * law.gains(0.)?[0].log10()
        }));
    }
    Ok(serde_json::json!({"cases":reports}))
}
