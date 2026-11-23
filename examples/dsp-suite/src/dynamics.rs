use super::*;
use soniclaw_dsp::dynamics::*;
struct Keyed {
    processor: Dynamics,
    key: Vec<f32>,
    position: usize,
}
impl AudioProcessor for Keyed {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.position = 0;
        self.processor.prepare(s)
    }
    fn reset(&mut self) {
        self.position = 0;
        self.processor.reset();
    }
    fn latency_frames(&self) -> usize {
        self.processor.latency_frames()
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let n = b.frames();
        self.processor
            .process_keyed(b, &self.key[self.position..self.position + n], 1)
            .unwrap();
        self.position += n;
    }
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<f32> = (0..48000)
        .map(|i| {
            let t = i as f32 / 48000.;
            0.6 * (std::f32::consts::TAU * 220. * t).sin() * (-8. * (t % 0.25)).exp()
        })
        .collect();
    let mut cases = vec![];
    for (name, mode) in [
        ("compress", Mode::Compress),
        ("expand", Mode::Expand),
        ("upward", Mode::Upward),
    ] {
        cases.push(
            render(
                out,
                name,
                Box::new(Dynamics::new(Config {
                    mode,
                    detector: Detector::Rms { window_frames: 64 },
                    ..Default::default()
                })?),
                input.clone(),
            )?
            .0,
        );
    }
    let key: Vec<f32> = (0..48000)
        .map(|i| if i % 12000 < 2000 { 0.8 } else { 0. })
        .collect();
    let p = Dynamics::new(Config {
        controls: Controls {
            hold_ms: 20.,
            ..Default::default()
        },
        ..Default::default()
    })?;
    cases.push(
        render(
            out,
            "keyed-duck",
            Box::new(Keyed {
                processor: p,
                key,
                position: 0,
            }),
            input,
        )?
        .0,
    );
    Ok(serde_json::json!({"cases":cases}))
}
