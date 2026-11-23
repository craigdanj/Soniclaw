use super::*;
use soniclaw_dsp::first_order_tpt::*;

struct Sweep {
    filter: FirstOrderTpt,
    offset: usize,
    hz: [f32; 128],
}
impl AudioProcessor for Sweep {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.filter.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let frames = b.frames();
        for (i, hz) in self.hz[..frames].iter_mut().enumerate() {
            *hz = 120. + 7000. * ((self.offset + i) as f32 * 0.00071).sin().powi(2);
        }
        self.filter
            .process_modulated(b, &self.hz[..frames])
            .unwrap();
        self.offset += frames;
    }
    fn reset(&mut self) {
        self.filter.reset();
        self.offset = 0;
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<_> = (0..8192)
        .map(|i| 0.35 * (((i as f32 * 0.031).sin()) + (i as f32 * 0.61).sin()))
        .collect();
    let mut cases = Vec::new();
    for (name, mode) in [
        ("modulated-lowpass", Mode::LowPass),
        ("modulated-highpass", Mode::HighPass),
        ("modulated-allpass", Mode::AllPass),
    ] {
        let engine = Sweep {
            filter: FirstOrderTpt::new(mode, 120.)?,
            offset: 0,
            hz: [0.; 128],
        };
        cases.push(render(out, name, Box::new(engine), input.clone())?.0);
    }
    Ok(serde_json::json!({ "cutoff_bounds_hz":[120.,7120.], "cases":cases }))
}
