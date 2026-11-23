use soniclaw_core::*;
use soniclaw_dsp::oversampling::*;
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("NEW_OUTPUT_DIRECTORY")?);
    std::fs::create_dir(&out)?;
    let mut cases = vec![];
    for factor in [Factor::Two, Factor::Four, Factor::Eight] {
        let config = Config {
            schema: 1,
            factor,
            quality: Quality::High,
        };
        let mut e = Oversampler::new(
            config,
            ProcessSpec {
                sample_rate: 48000.,
                max_block_frames: 128,
                channels: 2,
            },
        )?;
        let mut pcm: Vec<f32> = (0..8192)
            .flat_map(|i| {
                let x = 0.3 * (std::f32::consts::TAU * 1000. * i as f32 / 48000.).sin();
                [x, x]
            })
            .collect();
        let mut allocations = 0;
        for data in pcm.chunks_mut(256) {
            allocations += allocation_counter::measure(|| {
                e.process_with(&mut AudioBuffer::new(data, 2).unwrap(), |b, _| {
                    for x in b.samples_mut() {
                        *x = (*x * 8.).tanh();
                    }
                })
                .unwrap()
            })
            .count_total;
        }
        assert_eq!(allocations, 0);
        let file = format!("drive-{}x.wav", factor.multiplier());
        let mut wav = hound::WavWriter::create(
            out.join(&file),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in pcm {
            wav.write_sample(x)?
        }
        wav.finalize()?;
        cases.push(serde_json::json!({"factor":factor.multiplier(),"latency_frames":e.latency_frames(),"callback_allocations":allocations,"audio":file}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"cases":cases}),
        )?,
    )?;
    Ok(())
}
