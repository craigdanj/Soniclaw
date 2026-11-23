use super::*;
use soniclaw_dsp::wave_digital::{Circuit, DiodeClipper};
use std::fmt::Write as _;

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let rate = spec().sample_rate;
    let input: Vec<_> = (0..4_800)
        .map(|i| (1.5 * (std::f64::consts::TAU * 997. * f64::from(i) / rate).sin()) as f32)
        .collect();
    let (report, rendered) = render(
        out,
        "wave-digital-overdrive",
        Box::new(DiodeClipper::new(Circuit::default())?),
        input.clone(),
    )?;
    let mut csv = String::from("time,input,output\n");
    for (i, (&x, &y)) in input.iter().zip(&rendered).enumerate() {
        writeln!(csv, "{:.12},{x:.9},{y:.9}", i as f64 / rate)?;
    }
    std::fs::write(out.join("diode-response.csv"), csv)?;
    std::fs::write(
        out.join("diode-reference.cir"),
        include_str!("diode-reference.cir"),
    )?;
    Ok(report)
}
