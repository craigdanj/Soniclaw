use super::*;
use soniclaw_dsp::restoration::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut n = Denoiser::new(1024, 1.7, 0.08)?;
    n.prepare(spec())?;
    let mut rng = 91u32;
    let mut noise = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        0.04 * (rng as f32 / u32::MAX as f32 * 2. - 1.)
    };
    for i in 0..192 {
        let mut x = [0.; 128];
        x.fill_with(&mut noise);
        n.process(&mut AudioBuffer::new(&mut x, 1)?, &context(i * 128));
    }
    n.set_mode(ProfileMode::Frozen);
    n.reset();
    let mut raw: Vec<_> = (0..48000)
        .map(|i| {
            if i < 24000 {
                noise()
            } else {
                noise() + 0.1 * (std::f32::consts::TAU * 440. * i as f32 / 48000.).sin()
            }
        })
        .collect();
    let mut wav = hound::WavWriter::create(
        out.join("denoised.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    let energy = |s: &[f32]| s.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
    let before = energy(&raw[3072..20000]);
    let allocation = allocation_counter::measure(|| {
        for (i, b) in raw.chunks_mut(128).enumerate() {
            n.process(&mut AudioBuffer::new(b, 1).unwrap(), &context(i * 128));
        }
    })
    .count_total;
    let after = energy(&raw[4096..21024]);
    assert!(after < before * 0.5);
    assert_eq!(allocation, 0);
    for x in raw {
        wav.write_sample(x)?;
    }
    wav.finalize()?;
    Ok(
        serde_json::json!({"noise_only_reduction_db":10.*(before/after).log10(),"callback_allocations":allocation,"latency_frames":n.latency_frames()}),
    )
}
