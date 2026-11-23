use super::*;
use soniclaw_dsp::lookup_transform::*;

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let reference = |x: f32| (3. * x).tanh();
    let table = LookupTransform::prepare(-2., 2., 257, reference)?;
    let error = table.measure_error(20_003, 1e-3, reference)?;
    assert!(error.maximum_absolute < 0.00025);
    let input = (0..8192).map(|i| 0.8 * (i as f32 * 0.081).sin()).collect();
    let (render, _) = render(
        out,
        "lookup-soft-clip",
        Box::new(LookupWaveshaper::new(table)),
        input,
    )?;
    Ok(serde_json::json!({
        "render": render, "table_points": 257, "grid_points": 20003, "relative_floor": 1e-3,
        "max_absolute_error": error.maximum_absolute, "rms_absolute_error": error.rms_absolute,
        "max_relative_error": error.maximum_relative
    }))
}
