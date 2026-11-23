use super::*;
use soniclaw_dsp::feedback_patch::{Edge, FeedbackPatch, SampleNode, Sum};

struct SoftClip(f32);
impl SampleNode for SoftClip {
    fn tick(&mut self, x: f32) -> f32 {
        (x * self.0).tanh() / self.0
    }
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let direct = Edge {
        from: 0,
        to: 1,
        delay_samples: 0,
        gain: 1.,
    };
    let mut cases = Vec::new();
    for (name, delay, gain, drive) in [
        ("short-resonator", 144, 0.93, 1.1),
        ("saturated-tape-echo", 7200, 0.68, 2.1),
    ] {
        let graph = FeedbackPatch::new(
            vec![Box::new(Sum), Box::new(SoftClip(drive))],
            &[
                direct,
                Edge {
                    from: 1,
                    to: 0,
                    delay_samples: delay,
                    gain,
                },
            ],
            0,
            1,
        )?;
        let mut input = vec![0.; 48_000];
        if name == "short-resonator" {
            input[0] = 0.65;
        } else {
            for (i, value) in input[..2000].iter_mut().enumerate() {
                let t = i as f32 / 48_000.;
                *value = 0.43 * (std::f32::consts::TAU * 330. * t).sin() * (-42. * t).exp();
            }
        }
        let (audio, wave) = render(out, name, Box::new(graph), input)?;
        assert!(wave[delay..delay + 145].iter().any(|x| x.abs() > 0.01));
        cases.push(serde_json::json!({"audio":audio,"feedback_samples":delay}));
    }
    Ok(serde_json::json!({"cases":cases,"sample_rate":48000}))
}
