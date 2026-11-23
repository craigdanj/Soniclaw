use super::*;
use soniclaw_dsp::{
    iir_oversampling::IirOversampler,
    oversampling::{Config, Factor, Oversampler, Quality},
};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 128,
        channels: 1,
    };
    let mut iir = IirOversampler::new(spec)?;
    let mut fir = Oversampler::new(
        Config {
            schema: 1,
            factor: Factor::Two,
            quality: Quality::Balanced,
        },
        spec,
    )?;
    let input: Vec<_> = (0..8192)
        .map(|i| 0.8 * (std::f64::consts::TAU * 601. * (i % 4096) as f64 / 4096.).sin() as f32)
        .collect();
    let shape = |b: &mut AudioBuffer<'_>, _: ProcessSpec| {
        for x in b.samples_mut() {
            *x = (*x * 3.).tanh();
        }
    };
    let mut direct = input.clone();
    shape(&mut AudioBuffer::new(&mut direct, 1)?, spec);
    let mut ia = input.clone();
    let mut fa = input;
    let c = allocation_counter::measure(|| {
        for b in ia.chunks_mut(128) {
            iir.process_with(&mut AudioBuffer::new(b, 1).unwrap(), shape)
                .unwrap();
        }
        for b in fa.chunks_mut(128) {
            fir.process_with(&mut AudioBuffer::new(b, 1).unwrap(), shape)
                .unwrap();
        }
    })
    .count_total;
    assert_eq!(c, 0);
    let mut residuals = Vec::new();
    for (label, pcm) in [("direct", direct), ("iir", ia), ("fir", fa)] {
        let fft = soniclaw_quality::Spectrum::coherent(&pcm[4096..])?;
        residuals.push((label, fft.distortion(&[601, 1803], 2)?.thdn_db));
        let mut w = hound::WavWriter::create(
            out.join(format!("{label}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in pcm {
            w.write_sample(x)?;
        }
        w.finalize()?;
    }
    Ok(
        serde_json::json!({"iir_dc_delay_samples":iir.dc_delay_samples(),"fir_delay_frames":fir.latency_frames(),"residual_db_excluding_fundamental_and_third":residuals,"allocations":c}),
    )
}
