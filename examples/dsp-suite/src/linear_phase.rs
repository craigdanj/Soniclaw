use super::*;
use soniclaw_dsp::linear_phase::{Crossover, Equalizer};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<_> = (0..48000)
        .map(|i| {
            let t = i as f32 / 48000.;
            0.15 * [220., 3300., 8800.]
                .iter()
                .map(|f| (std::f32::consts::TAU * t * f).sin())
                .sum::<f32>()
        })
        .collect();
    let mut c = Crossover::new(48000., 1, [1800., 5500.], 1000., 66.)?;
    let mut lo = vec![0.; input.len()];
    let mut mid = lo.clone();
    let mut hi = lo.clone();
    c.split(&input, [&mut lo, &mut mid, &mut hi])?;
    let lag = c.latency_frames();
    let error = (0..input.len())
        .map(|i| (lo[i] + mid[i] + hi[i] - i.checked_sub(lag).map_or(0., |j| input[j])).abs())
        .fold(0., f32::max);
    assert!(error < 1e-6);
    let (details, _) = render(
        out,
        "linear-phase-eq",
        Box::new(Equalizer::new(c, [1.2, 0.4, 0.7])?),
        input,
    )?;
    Ok(serde_json::json!({"recombination_error":error,"latency_frames":lag,"equalizer":details}))
}
