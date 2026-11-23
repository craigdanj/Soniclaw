use super::*;
use soniclaw_dsp::oversampling::{
    FilterMode, OversamplingConfig, Quality, Ratio, UnifiedOversampler,
};

struct Drive(UnifiedOversampler);
impl AudioProcessor for Drive {
    fn prepare(&mut self, _: ProcessSpec) -> std::result::Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        self.0
            .process_with(b, |high, _| {
                for x in high.samples_mut() {
                    *x = (5. * *x).tanh();
                }
            })
            .expect("valid render");
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_frames(&self) -> usize {
        self.0
            .latency()
            .integer_reference_frames
            .unwrap_or_default()
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut cases = Vec::new();
    let input: Vec<_> = (0..8192)
        .map(|i| (i as f32 * std::f32::consts::TAU * 9000. / 48000.).sin())
        .collect();
    for (mode, prefix) in [(FilterMode::Fir, "fir"), (FilterMode::Iir, "iir")] {
        for ratio in [
            Ratio::One,
            Ratio::Two,
            Ratio::Four,
            Ratio::Eight,
            Ratio::Sixteen,
        ] {
            if mode == FilterMode::Iir && ratio == Ratio::One {
                continue;
            }
            let converter = UnifiedOversampler::new(
                OversamplingConfig {
                    ratio,
                    mode,
                    quality: Quality::High,
                    integer_latency: true,
                },
                spec(),
            )?;
            let latency = converter.latency();
            let (mut case, _) = render(
                out,
                &format!("{prefix}-{}x-drive", ratio.multiplier()),
                Box::new(Drive(converter)),
                input.clone(),
            )?;
            case["dc_latency_frames"] = serde_json::json!(latency.dc_frames);
            case["linear_phase"] = serde_json::json!(latency.linear_phase);
            cases.push(case);
        }
    }
    Ok(serde_json::json!({"cases":cases}))
}
