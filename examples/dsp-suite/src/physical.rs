use super::*;
use soniclaw_dsp::physical::*;
struct Picked(WaveguideString);
impl AudioProcessor for Picked {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.0.prepare(s)?;
        self.0.pluck(0.6, 0.23, 42)
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        self.0.process(b, c);
    }
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut cases = vec![];
    for (name, pressure) in [("picked-string", 0.), ("friction-string", 0.7)] {
        cases.push(
            render(
                out,
                name,
                Box::new(Picked(WaveguideString::new(StringConfig {
                    bow_pressure: pressure,
                    ..Default::default()
                })?)),
                vec![0.; 96000],
            )?
            .0,
        );
    }
    let modes = [330., 539., 911., 1447., 2131.]
        .iter()
        .enumerate()
        .map(|(i, hz)| Mode {
            frequency: *hz,
            decay_seconds: 1.4 / (1. + i as f32 * 0.3),
            gain: 0.5 / (1. + i as f32 * 0.2),
        })
        .collect();
    let mut impulse = vec![0.; 96000];
    impulse[0] = 1.;
    cases.push(
        render(
            out,
            "modal-metal",
            Box::new(ModalBank::new(ModalConfig { schema: 1, modes })?),
            impulse,
        )?
        .0,
    );
    Ok(serde_json::json!({"cases":cases}))
}
