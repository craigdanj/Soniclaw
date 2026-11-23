use super::*;
use soniclaw_core::{AudioBuffer64, AudioProcessor64};
use soniclaw_dsp::{
    dynamics::{Config, Controls},
    filter_design::{butterworth, Response},
    precision::{Dynamics64, Filter64, Gain64},
};
use std::fmt::Write as _;

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let s = spec();
    let delta = 2f64.powi(-40);
    let mut gain = Gain64::new(2.)?;
    let mut fir = Filter64::fir(s.sample_rate, vec![0.25, 0.5, 0.25])?;
    let mut iir = Filter64::iir(
        s.sample_rate,
        butterworth(s.sample_rate, 4000., 4, Response::LowPass)?,
    )?;
    let mut dynamics = Dynamics64::new(Config {
        lookahead_ms: 1.,
        controls: Controls {
            ratio: 1.,
            ..Default::default()
        },
        ..Default::default()
    })?;
    let mut chain: [&mut dyn AudioProcessor64; 4] = [&mut gain, &mut fir, &mut iir, &mut dynamics];
    let mut csv = String::from("frame,unit_impulse,perturbed_impulse,difference\n");
    let mut renders = [[0.; 512]; 2];
    for (case, buffer) in renders.iter_mut().enumerate() {
        for processor in &mut chain {
            processor.prepare(s)?;
        }
        buffer[0] = 1. + case as f64 * delta;
        for (index, block) in buffer.as_chunks_mut::<128>().0.iter_mut().enumerate() {
            let context = context(index * 128);
            let allocations = allocation_counter::measure(|| {
                let mut block = AudioBuffer64::new(block, 1).unwrap();
                for p in &mut chain {
                    p.process(&mut block, &context);
                }
            });
            assert_eq!(allocations.count_total, 0);
        }
    }
    assert_eq!(renders[0][..48], [0.; 48]);
    assert_eq!(renders[1][..48], [0.; 48]);
    let mut peak_delta = 0f64;
    for (i, (&a, &b)) in renders[0].iter().zip(&renders[1]).enumerate() {
        peak_delta = peak_delta.max((b - a).abs());
        writeln!(csv, "{i},{a:.17e},{b:.17e},{:.17e}", b - a)?;
    }
    assert!(peak_delta > 1e-14);
    assert!((renders[0][48] as f32 - renders[1][48] as f32).abs() == 0.);
    std::fs::write(out.join("precision-impulses.csv"), csv)?;
    Ok(
        serde_json::json!({"input_delta":delta,"maximum_residual":peak_delta,"callback_allocations":0,"lookahead_frames":dynamics.latency_frames()}),
    )
}
