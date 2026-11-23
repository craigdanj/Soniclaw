use super::*;
use soniclaw_dsp::filter_design::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let taps = kaiser(48000., 6000., 2400., 75., Response::LowPass)?;
    let sections = butterworth(48000., 6000., 8, Response::LowPass)?;
    let mut csv = String::from(
        "hz,fir_db,fir_phase_rad,fir_group_samples,iir_db,iir_phase_rad,iir_group_samples\n",
    );
    use std::fmt::Write;
    for i in 0..=480 {
        let hz = 50. * f64::from(i);
        let f = hz / 48000.;
        let a = fir_at(&taps, f);
        let b = iir_at(&sections, f);
        writeln!(
            csv,
            "{hz},{},{},{},{},{},{}",
            20. * a.norm().max(1e-15).log10(),
            a.arg(),
            group_delay(|x| fir_at(&taps, x), f).unwrap_or(f64::NAN),
            20. * b.norm().max(1e-15).log10(),
            b.arg(),
            group_delay(|x| iir_at(&sections, x), f).unwrap_or(f64::NAN)
        )?;
    }
    std::fs::write(out.join("response.csv"), csv)?;
    let pcm: Vec<f32> = (0..24000)
        .map(|i| {
            let t = i as f32 / 48000.;
            0.24 * ((std::f32::consts::TAU * 440. * t).sin()
                + (std::f32::consts::TAU * 9000. * t).sin())
        })
        .collect();
    let (fir, _) = render(
        out,
        "fir",
        Box::new(Filter::fir(48000., taps.clone())?),
        pcm.clone(),
    )?;
    let (iir, _) = render(
        out,
        "iir",
        Box::new(Filter::iir(48000., sections.clone())?),
        pcm,
    )?;
    Ok(
        serde_json::json!({"fir":fir,"iir":iir,"taps":taps.len(),"iir_sections":sections.len(),"response":"response.csv"}),
    )
}
