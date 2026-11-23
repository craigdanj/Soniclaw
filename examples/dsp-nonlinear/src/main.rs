use soniclaw_core::*;
use soniclaw_dsp::{
    nonlinear::{self, Shape, Waveshaper},
    oversampling::{self, Factor, Oversampled, Quality},
};
use soniclaw_quality::Spectrum;
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("NEW_DIRECTORY")?);
    std::fs::create_dir(&out)?;
    let n = 4096;
    let bins = [701, 851];
    let mut reports = vec![];
    for bin in bins {
        let spec = ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        };
        let input: Vec<f32> = (0..n * 3)
            .map(|i| (std::f64::consts::TAU * bin as f64 * i as f64 / n as f64).sin() as f32)
            .collect();
        for shape in [Shape::HardClip, Shape::Tanh, Shape::Cubic] {
            let mut naive: Vec<f32> = input
                .iter()
                .map(|x| shape.sample(*x as f64 * 4.) as f32)
                .collect();
            let (mut a, mut b) = (0., 0.);
            for x in &mut naive {
                let y = *x;
                *x = 0.25 * y + 0.5 * a + 0.25 * b;
                b = a;
                a = y;
            }
            let baseline = Spectrum::coherent(&naive[2 * n..])?
                .distortion(&[bin], 2)?
                .thdn_db;
            for oversampled in [false, true] {
                let config = nonlinear::Config {
                    shape,
                    drive: 4.,
                    smoothing_ms: 0.,
                    latency_multiple: if oversampled { 4 } else { 1 },
                    ..Default::default()
                };
                let mut p: Box<dyn AudioProcessor> = if oversampled {
                    Box::new(Oversampled::new(
                        Waveshaper::new(config)?,
                        oversampling::Config {
                            schema: 1,
                            factor: Factor::Four,
                            quality: Quality::High,
                        },
                    ))
                } else {
                    Box::new(Waveshaper::new(config)?)
                };
                p.prepare(spec)?;
                let mut pcm = input.clone();
                let mut allocations = 0;
                for block in pcm.chunks_mut(128) {
                    allocations += allocation_counter::measure(|| {
                        p.process(
                            &mut AudioBuffer::new(block, 1).unwrap(),
                            &ProcessContext {
                                events: &[],
                                transport: Transport::default(),
                            },
                        )
                    })
                    .count_total;
                }
                assert_eq!(allocations, 0);
                let residual = Spectrum::coherent(&pcm[2 * n..])?
                    .distortion(&[bin], 2)?
                    .thdn_db;
                let improvement = baseline - residual;
                eprintln!("{shape:?} oversampled={oversampled} baseline={baseline:.2} residual={residual:.2} improvement={improvement:.2}");
                assert!(improvement > 3., "coherent fixture alias regression");
                let file = format!(
                    "{shape:?}-{bin}-{}.wav",
                    if oversampled { "4x" } else { "base" }
                );
                let mut w = hound::WavWriter::create(
                    out.join(&file),
                    hound::WavSpec {
                        channels: 1,
                        sample_rate: 48000,
                        bits_per_sample: 32,
                        sample_format: hound::SampleFormat::Float,
                    },
                )?;
                for x in &pcm[2 * n..] {
                    w.write_sample(*x)?
                }
                w.finalize()?;
                reports.push(serde_json::json!({"fundamental_bin":bin,"shape":shape,"oversampled_4x":oversampled,"matched_filter_naive_residual_db":baseline,"processed_residual_db":residual,"improvement_db":improvement,"callback_allocations":allocations,"latency_frames":p.latency_frames(),"audio":file}));
            }
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"fft_size":n,"fundamental_bins":bins,"sample_rate":48000,"drive":4,"warmup_frames":2*n,"cases":reports}),
        )?,
    )?;
    Ok(())
}
