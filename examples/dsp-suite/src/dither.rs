use super::*;
use soniclaw_dsp::dither::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let x: Vec<_> = (0..48000)
        .map(|i| {
            0.015
                * (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin()
                * (1. - i as f32 / 48000.).powi(3)
        })
        .collect();
    let mut reports = Vec::new();
    for (name, depth, shaping) in [
        ("tpdf16", Depth::Pcm16, Shaping::Flat),
        ("shaped16", Depth::Pcm16, Shaping::FirstOrder),
        ("tpdf24", Depth::Pcm24, Shaping::Flat),
    ] {
        let mut q = Quantizer::new(
            Settings {
                depth,
                shaping,
                seed: 2026,
            },
            1,
        )?;
        let mut y = vec![0; x.len()];
        let a = allocation_counter::measure(|| q.process(&x, &mut y).unwrap()).count_total;
        assert_eq!(a, 0);
        let bits = q.bits();
        let mut w = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: bits,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        let mut error = 0.;
        let scale = f64::from(1i32 << (bits - 1));
        for (&i, s) in x.iter().zip(y) {
            w.write_sample(s)?;
            error += (f64::from(i) - f64::from(s) / scale).powi(2);
        }
        w.finalize()?;
        reports.push((name, 10. * (error / x.len() as f64).log10()));
    }
    Ok(serde_json::json!({"quantization_error_dbfs":reports,"frames":x.len()}))
}
