use soniclaw_core::*;
use soniclaw_dsp::{
    creative_filters::*,
    oversampling::{Config, Factor, Oversampled, Quality},
};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("NEW_DIRECTORY")?);
    std::fs::create_dir(&out)?;
    let cfg = NonlinearConfig {
        resonance: 0.8,
        drive: 3.,
        ..Default::default()
    };
    let processors: Vec<(&str, Box<dyn AudioProcessor>)> = vec![
        ("nonlinear-svf", Box::new(NonlinearSvf::new(cfg)?)),
        ("comb", Box::new(CombFilter::new(Default::default())?)),
        (
            "formants",
            Box::new(FormantFilter::new(Default::default())?),
        ),
        (
            "oversampled-svf",
            Box::new(Oversampled::new(
                NonlinearSvf::new(cfg)?,
                Config {
                    schema: 1,
                    factor: Factor::Four,
                    quality: Quality::Balanced,
                },
            )),
        ),
    ];
    let mut reports = vec![];
    for (name, mut p) in processors {
        p.prepare(ProcessSpec {
            sample_rate: 48000.,
            max_block_frames: 128,
            channels: 1,
        })?;
        let mut seed = 12345u32;
        let mut pcm: Vec<f32> = (0..24000)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                0.15 * (std::f64::consts::TAU * 110. * i as f64 / 48000.).sin() as f32
                    + 0.15 * (seed as f64 / u32::MAX as f64 * 2. - 1.) as f32
            })
            .collect();
        let mut allocations = 0;
        for b in pcm.chunks_mut(128) {
            allocations += allocation_counter::measure(|| {
                p.process(
                    &mut AudioBuffer::new(b, 1).unwrap(),
                    &ProcessContext {
                        events: &[],
                        transport: Transport::default(),
                    },
                )
            })
            .count_total;
        }
        assert_eq!(allocations, 0);
        assert!(pcm.iter().all(|x| x.is_finite()));
        let peak = pcm.iter().map(|x| x.abs()).fold(0f32, f32::max);
        assert!(peak > 1e-5);
        let file = format!("{name}.wav");
        let mut w = hound::WavWriter::create(
            out.join(&file),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in pcm {
            w.write_sample(x)?
        }
        w.finalize()?;
        reports.push(serde_json::json!({"filter":name,"peak":peak,"latency_frames":p.latency_frames(),"callback_allocations":allocations,"audio":file}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"cases":reports}),
        )?,
    )?;
    Ok(())
}
