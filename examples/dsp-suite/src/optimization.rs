use super::*;
use soniclaw_dsp::{prepared_resample::PreparedSinc, resample::ResampleQuality};
use std::{hint::black_box, time::Instant};
fn timed(output: &mut [f32], positions: &[f64], mut sample: impl FnMut(f64) -> f32) -> (u128, u64) {
    let start = Instant::now();
    let allocations = allocation_counter::measure(|| {
        for (x, pos) in output.iter_mut().zip(positions) {
            *x = black_box(sample(black_box(*pos)));
        }
    })
    .count_total;
    (start.elapsed().as_micros(), allocations)
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let start = Instant::now();
    let prepared = PreparedSinc::new(Default::default())?;
    let prepare_us = start.elapsed().as_micros();
    let source: Vec<f32> = (0..262144)
        .map(|i| {
            let x = i as f64;
            ((x * 0.021).sin() * 0.2 + (x * 0.13).sin() * 0.15 + (x * 0.3).sin() * 0.1) as f32
        })
        .collect();
    let read = |i: isize| source[(i as usize) % source.len()];
    let mut cases = vec![];
    let mut audio_old = vec![];
    let mut audio_prepared = vec![];
    for rate in [1., 2., 4., 8.] {
        let positions: Vec<f64> = (0..24000)
            .map(|i| 2048. + i as f64 * rate + 0.371)
            .collect();
        let mut expected = vec![0.; positions.len()];
        let mut actual = expected.clone();
        for pos in positions.iter().take(512) {
            black_box(ResampleQuality::SincFast.interpolate(*pos, rate, read));
            black_box(prepared.interpolate(*pos, rate, read));
        }
        let mut old_times = [0u128; 3];
        let mut new_times = [0u128; 3];
        let mut allocations = 0;
        for round in 0..3 {
            let mut old = || {
                let (t, a) = timed(&mut expected, &positions, |pos| {
                    ResampleQuality::SincFast.interpolate(pos, black_box(rate), read)
                });
                old_times[round] = t;
                a
            };
            let mut new = || {
                let (t, a) = timed(&mut actual, &positions, |pos| {
                    prepared.interpolate(pos, black_box(rate), read)
                });
                new_times[round] = t;
                a
            };
            let (old_alloc, new_alloc) = if round % 2 == 0 {
                (old(), new())
            } else {
                let first = new();
                (old(), first)
            };
            allocations += old_alloc + new_alloc;
        }
        let max_error = expected
            .iter()
            .zip(&actual)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max_error < 2e-6);
        assert_eq!(allocations, 0);
        let mut a = old_times;
        let mut b = new_times;
        a.sort();
        b.sort();
        let old_us = a[1];
        let new_us = b[1];
        cases.push(serde_json::json!({"increment":rate,"frames":positions.len(),"exact_sinc_us":old_us,"prepared_us":new_us,"exact_runs_us":old_times,"prepared_runs_us":new_times,"observed_ratio":old_us as f64/new_us.max(1)as f64,"maximum_absolute_error":max_error,"callback_allocations":allocations}));
        audio_old.extend(expected);
        audio_prepared.extend(actual);
    }
    for (name, pcm) in [("exact-sinc", audio_old), ("prepared-sinc", audio_prepared)] {
        let mut wav = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in pcm {
            wav.write_sample(x)?;
        }
        wav.finalize()?;
    }
    Ok(
        serde_json::json!({"prepare_us":prepare_us,"table_bytes":prepared.storage_bytes(),"backend":format!("{:?}",prepared.backend()),"architecture":std::env::consts::ARCH,"release_build":!cfg!(debug_assertions),"statistic":"median of three; alternating order; 512-frame warmup","cases":cases,"scope":"local synthetic exact band rates; not a universal speedup"}),
    )
}
