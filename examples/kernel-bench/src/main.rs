use soniclaw_kernels::{Backend, Kernel};
use std::{hint::black_box, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-kernel-bench NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let n = 4096;
    let repeats = 1000;
    let input: Vec<_> = (0..n).map(|i| (i as f32 * 0.013).sin() * 0.1).collect();
    let other: Vec<_> = (0..n).map(|i| (i as f32 * 0.017).cos() * 0.01).collect();
    let mut reports = Vec::new();
    for backend in [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Neon] {
        let Ok(k) = Kernel::new(backend) else {
            continue;
        };
        let mut rows = serde_json::Map::new();
        for operation in ["gain", "mix_add", "dot", "interleave"] {
            let mut times = Vec::new();
            for round in 0..8 {
                let mut data = input.clone();
                let mut stereo = vec![0.; n * 2];
                let start = Instant::now();
                for _ in 0..repeats {
                    match operation {
                        "gain" => k.gain(black_box(&mut data), black_box(0.999999))?,
                        "mix_add" => {
                            k.mix_add(black_box(&mut data), black_box(&other), black_box(0.001))?
                        }
                        "dot" => {
                            black_box(k.dot(black_box(&data), black_box(&other))?);
                        }
                        _ => k.interleave(
                            black_box(&mut stereo),
                            black_box(&data),
                            black_box(&other),
                        )?,
                    }
                }
                black_box((&data, &stereo));
                let elapsed = start.elapsed().as_nanos() as u64;
                if round > 0 {
                    times.push(elapsed);
                }
            }
            times.sort_unstable();
            rows.insert(operation.into(),serde_json::json!({"median_ns":times[times.len()/2],"repeats":repeats,"samples_per_call":n,"timings_ns":times}));
        }
        reports.push(serde_json::json!({"backend":format!("{backend:?}"),"measurements":rows}));
    }
    let report = serde_json::json!({"schema":1,"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"selected":format!("{:?}",Kernel::best().backend()),"results":reports,"scope":"Microbenchmarks on this machine; no application or universal speedup claim."});
    std::fs::write(
        path.join("benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
