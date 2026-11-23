use super::*;
use soniclaw_dsp::fractional_delay::{DelayLine, Interpolation};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut d = DelayLine::new(1, 1600, 420., 0.15, Interpolation::Cubic)?;
    let mut wav = hound::WavWriter::create(
        out.join("modulated-delay.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let mut pcm = vec![0.; 48000];
    let alloc = allocation_counter::measure(|| {
        for (i, y) in pcm.iter_mut().enumerate() {
            let t = i as f64 / 48000.;
            d.set_delay(420. + 150. * (std::f64::consts::TAU * 0.7 * t).sin())
                .unwrap();
            let x = 0.3 * (std::f64::consts::TAU * 330. * t).sin() as f32;
            d.tick(&[x], std::slice::from_mut(y)).unwrap();
            *y = 0.65 * x + 0.35 * *y;
        }
    })
    .count_total;
    assert_eq!(alloc, 0);
    assert!(pcm.iter().all(|x| x.is_finite()));
    for x in pcm {
        wav.write_sample(x)?;
    }
    wav.finalize()?;
    Ok(serde_json::json!({"frames":48000,"callback_allocations":alloc}))
}
