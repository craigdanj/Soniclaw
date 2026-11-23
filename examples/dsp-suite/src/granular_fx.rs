use super::*;
use soniclaw_dsp::granular_fx::{Controls, LiveGranular};

struct Demo(LiveGranular, Controls);
impl AudioProcessor for Demo {
    fn prepare(&mut self, s: ProcessSpec) -> std::result::Result<(), Error> {
        self.0.prepare(s)
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        self.0
            .set_controls(Controls {
                freeze: ctx.transport.sample_position >= 24_576,
                ..self.1
            })
            .unwrap();
        let ctx = ProcessContext {
            events: ctx.events,
            transport: Transport {
                tempo: 120.,
                playing: true,
                ..ctx.transport
            },
        };
        self.0.process(b, &ctx);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input = (0..60_000)
        .map(|i| {
            if i >= 24_576 {
                return 0.;
            }
            let t = i as f32 / 48_000.;
            let a = (std::f32::consts::TAU * 220. * t).sin();
            (0.2 + 0.1 * (43. * t).sin())
                * (a + 0.2 * (std::f32::consts::TAU * (330. + 900. * t) * t).sin())
        })
        .collect();
    let cfg = Controls {
        grain_ms: 63.,
        delay_ms: 210.,
        pitch_semitones: 7.,
        mix: 0.8,
        ..Default::default()
    };
    let (report, pcm) = render(
        out,
        "beat-granular-freeze",
        Box::new(Demo(LiveGranular::new(1000., 32, cfg)?, cfg)),
        input,
    )?;
    assert!(pcm[36_000..].iter().any(|x| x.abs() > 0.02));
    Ok(serde_json::json!({"audio":report,"freeze_frame":24576,"tempo":120,"interval_beats":0.25}))
}
