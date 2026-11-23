use super::*;
use soniclaw_dsp::{
    coupled_instrument::*,
    physical::{ModalConfig, Mode, StringConfig},
};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let cfg = Settings {
        schema: 1,
        strings: [220., 221., 330.]
            .into_iter()
            .map(|frequency| StringConfig {
                frequency,
                decay_seconds: 1.8,
                ..Default::default()
            })
            .collect(),
        body: ModalConfig {
            schema: 1,
            modes: vec![
                Mode {
                    frequency: 220.,
                    decay_seconds: 0.6,
                    gain: 0.012,
                },
                Mode {
                    frequency: 440.,
                    decay_seconds: 0.3,
                    gain: 0.03,
                },
            ],
        },
        coupling: 0.026,
        body_mix: 0.45,
        hammer_ms: 1.4,
    };
    let preset = serde_json::to_vec_pretty(&cfg)?;
    std::fs::write(out.join("instrument.json"), &preset)?;
    let mut results = Vec::new();
    for (name, coupling) in [("uncoupled", 0.), ("sympathetic", cfg.coupling)] {
        let mut loaded: Settings = serde_json::from_slice(&preset)?;
        loaded.coupling = coupling;
        let mut i = CoupledInstrument::new(loaded)?;
        i.prepare(spec())?;
        let mut audio = vec![0.; 48000];
        let mut other = 0.;
        let allocations = allocation_counter::measure(|| {
            i.strike(0, 0.85).unwrap();
            for (index, x) in audio.iter_mut().enumerate() {
                if index == 20000 {
                    i.pluck(2, 0.48, 0.23, 18).unwrap();
                }
                *x = i.tick();
                other += f64::from(i.string_values()[1]).powi(2);
            }
        })
        .count_total;
        assert_eq!(allocations, 0);
        assert!(audio.iter().all(|x| x.is_finite() && x.abs() <= 2.));
        if coupling == 0. {
            assert_eq!(other, 0.);
        } else {
            assert!(other > 1e-10);
        }
        let energy = |samples: &[f32]| samples.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        let middle = energy(&audio[24000..30000]);
        let tail = energy(&audio[42000..48000]);
        assert!(
            middle > 0. && tail < middle * 0.8,
            "{name} ring-down: {middle} -> {tail}"
        );
        i.reset();
        assert_eq!(i.tick(), 0.);
        let mut w = hound::WavWriter::create(
            out.join(format!("{name}.wav")),
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for x in audio {
            w.write_sample(x)?;
        }
        w.finalize()?;
        results.push(serde_json::json!({"render":name,"idle_string_energy":other,"middle_energy":middle,"late_tail_energy":tail,"callback_allocations":allocations}));
    }
    Ok(serde_json::json!({"renders":results}))
}
