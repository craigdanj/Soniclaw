use super::*;
use soniclaw_dsp::{
    composition::{MixingRule, Parallel, ProcessorChain},
    dynamics::{Config, Controls, Dynamics},
};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let mut cases = Vec::new();
    for (name, mix) in [
        ("parallel-dry", 0.),
        ("parallel-mix", 0.5),
        ("parallel-wet", 1.),
    ] {
        let mut chain = ProcessorChain::default();
        chain.push(Parallel::new(
            Dynamics::new(Config {
                lookahead_ms: 5.,
                controls: Controls {
                    attack_ms: 0.3,
                    ..Default::default()
                },
                ..Default::default()
            })?,
            MixingRule::Linear,
            mix,
            64,
        )?);
        let mut signal = vec![0.; 48_000 * 2];
        for (i, frame) in signal.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let phase = (i / 6000) % 2;
            let amplitude = if phase == 0 { 0.15 } else { 0.85 };
            frame[0] = amplitude * (i as f32 * std::f32::consts::TAU * 220. / 48000.).cos();
            frame[1] = frame[0] * 0.75;
        }
        let (report, pcm) = render_channels(out, name, Box::new(chain), signal, 2)?;
        assert!(pcm[..480].iter().all(|x| *x == 0.));
        assert!((0.01..=0.85).contains(&pcm[480]));
        cases.push(report);
    }
    Ok(serde_json::json!({"cases": cases, "lookahead_ms": 5}))
}
