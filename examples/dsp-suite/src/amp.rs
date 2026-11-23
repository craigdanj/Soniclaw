use super::*;
use soniclaw_dsp::amp_models::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let input: Vec<f32> = (0..48000)
        .map(|i| {
            let t = i as f32 / 48000.;
            let local = t % 0.25;
            let wave = (1..=10)
                .map(|h| 0.15 / h as f32 * (std::f32::consts::TAU * (110. * h as f32) * t).sin())
                .sum::<f32>();
            wave * (-local * 10.).exp()
        })
        .collect();
    let mut cases = vec![];
    for (name, cab, controls) in [
        (
            "amp-clean",
            false,
            Controls {
                drive_db: 3.,
                mix: 0.8,
                ..Default::default()
            },
        ),
        (
            "pedal-asymmetric",
            false,
            Controls {
                asymmetry: 1.7,
                drive_db: 24.,
                ..Default::default()
            },
        ),
        (
            "amp-sag-cabinet",
            true,
            Controls {
                drive_db: 30.,
                sag: 0.9,
                ..Default::default()
            },
        ),
    ] {
        let config = Config {
            controls,
            ..Default::default()
        };
        let amp = if cab {
            let ir = (0..512)
                .map(|i| {
                    let t = i as f32;
                    0.06 * (-t / 80.).exp() * (t * 0.17).cos()
                })
                .collect();
            Amp::with_cabinet(config, ir, 48000., 64)?
        } else {
            Amp::new(config)?
        };
        cases.push(render(out, name, Box::new(amp), input.clone())?.0);
    }
    Ok(
        serde_json::json!({"cases":cases,"input":"owned synthetic plucked harmonics","cabinet":"synthetic decaying impulse"}),
    )
}
