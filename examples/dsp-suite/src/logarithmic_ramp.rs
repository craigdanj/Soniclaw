use super::*;
use soniclaw_dsp::logarithmic_ramp::*;

struct Envelope {
    ramp: LogarithmicRamp,
    sample: usize,
}
impl AudioProcessor for Envelope {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        self.ramp.set_target(1., 8192)
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if self.sample == 8192 {
            self.ramp.set_target(0., 8192).unwrap();
        }
        self.sample += audio.frames();
        self.ramp.apply_gain(audio);
    }
    fn reset(&mut self) {
        self.sample = 0;
        self.ramp.set_target(0., 0).unwrap();
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let rise = Curve::Logarithmic { strength: 5. };
    let fall = Curve::Exponential { strength: 4. };
    let mut measurement = LogarithmicRamp::new(0., rise, fall)?;
    measurement.set_target(1., 8192)?;
    let half_rise = measurement.advance(4096);
    assert!(half_rise > 0.5);
    assert_eq!(measurement.advance(4096), 1.);
    measurement.set_target(0., 8192)?;
    let half_fall = measurement.advance(4096);
    assert!(half_fall > 0.5);
    assert_eq!(measurement.advance(4096), 0.);
    let envelope = Envelope {
        ramp: LogarithmicRamp::new(0., rise, fall)?,
        sample: 0,
    };
    let input = (0..16384)
        .map(|i| 0.55 * (i as f32 * 0.071).sin())
        .collect();
    let (case, _) = render(out, "logarithmic-envelope", Box::new(envelope), input)?;
    Ok(serde_json::json!({ "render":case, "half_rise":half_rise, "half_fall":half_fall }))
}
