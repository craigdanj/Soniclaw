use super::*;
use soniclaw_dsp::spectral_morph::{Morph, SpectralMorph};

struct Demo {
    controls: Morph,
    engine: Option<SpectralMorph>,
}
impl AudioProcessor for Demo {
    fn prepare(&mut self, spec: ProcessSpec) -> std::result::Result<(), Error> {
        self.engine = Some(SpectralMorph::new(spec, 1024, 12, self.controls)?);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, c: &ProcessContext<'_>) {
        let mut side = [0.; 128];
        for (i, x) in side[..b.frames()].iter_mut().enumerate() {
            let t = (c.transport.sample_position + i as u64) as f64 / 48_000.;
            let a = (std::f64::consts::TAU * t * 880.).sin();
            let amplitude = 0.2 + 0.17 * (std::f64::consts::TAU * t * 5.).sin();
            *x = (amplitude * (a + 0.8 * (std::f64::consts::TAU * t * 1540.).sin())) as f32;
        }
        self.engine
            .as_mut()
            .unwrap()
            .process(b, &side[..b.frames()])
            .unwrap();
    }
    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.reset();
        }
    }
    fn latency_frames(&self) -> usize {
        self.engine
            .as_ref()
            .map_or(0, SpectralMorph::latency_frames)
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<_> = (0..48_000)
        .map(|i| {
            let phase = std::f64::consts::TAU * f64::from(i) * 220. / 48_000.;
            (1..=16)
                .map(|h| (phase * f64::from(h)).sin() * 0.16 / f64::from(h))
                .sum::<f64>() as f32
        })
        .collect();
    let mut reports = Vec::new();
    for (name, env, fine) in [
        ("carrier", 0., 0.),
        ("envelope-transfer", 1., 0.),
        ("partial-blend", 0.65, 0.6),
    ] {
        let (report, _) = render(
            out,
            name,
            Box::new(Demo {
                controls: Morph {
                    envelope: env,
                    fine_structure: fine,
                },
                engine: None,
            }),
            input.clone(),
        )?;
        reports.push(report);
    }
    Ok(serde_json::json!({"cases":reports,"frames_latency":1024}))
}
