use super::*;
use soniclaw_dsp::fractional_delay::{DelayLine, DelayMode};
struct Delay(DelayLine);
impl AudioProcessor for Delay {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        for (i, sample) in b.samples_mut().iter_mut().enumerate() {
            let position = c.transport.sample_position + i as u64;
            self.0
                .set_delay(32. + (position as f64 * 0.0004).sin() * 8.)
                .unwrap();
            let mut delayed = [0.];
            self.0.tick(&[*sample], &mut delayed).unwrap();
            *sample = *sample * 0.5 + delayed[0] * 0.5;
        }
    }
    fn reset(&mut self) {
        self.0.reset();
    }
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<_> = (0..48_000)
        .map(|i| {
            let t = f64::from(i) / 48000.;
            (0.4 * (std::f64::consts::TAU * 330. * t).sin()
                + 0.15 * (std::f64::consts::TAU * 3600. * t).sin()) as f32
        })
        .collect();
    let mut cases = Vec::new();
    for (mode, label) in [
        (DelayMode::Integer, "integer"),
        (DelayMode::Linear, "linear"),
        (DelayMode::Cubic, "cubic"),
        (DelayMode::Lagrange3, "lagrange"),
        (DelayMode::Thiran, "thiran"),
    ] {
        let line = DelayLine::with_mode(1, 64, 32., 0.05, mode)?;
        let (report, _) = render(out, label, Box::new(Delay(line)), input.clone())?;
        cases.push(report);
    }
    Ok(serde_json::json!({"mix":0.5,"delay_range_frames":[24.,40.],"cases":cases}))
}
