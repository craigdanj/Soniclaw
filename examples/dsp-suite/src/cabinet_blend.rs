use super::*;
use soniclaw_dsp::{
    cabinet_blend::{blend, estimate, Blend},
    composition::ProcessorChain,
    nam_wavenet::NamWaveNet,
};

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let (a, b, rate, fixture) = match args {
        [] => {
            let mut a = vec![0.; 256];
            for (i, x) in a.iter_mut().enumerate().skip(16) {
                let t = (i - 16) as f32;
                *x = (0.25 * (0.31 * t).cos() + 0.14 * (0.76 * t).cos()) * (-0.035 * t).exp();
            }
            let mut b = vec![0.; 256];
            for (x, y) in b[17..].iter_mut().zip(&a) {
                *x = -0.8 * y;
            }
            (a, b, 48_000, true)
        }
        [a, b] => {
            let a = soniclaw::io::read_wav_with_limit(a, 4 * 1024 * 1024)?;
            let b = soniclaw::io::read_wav_with_limit(b, 4 * 1024 * 1024)?;
            if a.channels != 1 || b.channels != 1 || a.sample_rate != b.sample_rate {
                return Err("provide two mono IRs at the same rate".into());
            }
            (a.samples, b.samples, a.sample_rate, false)
        }
        _ => return Err("expected zero or two cabinet WAV paths".into()),
    };
    let alignment = estimate(&a, &b, 2048)?;
    if fixture {
        assert!((alignment.advance_frames - 17.).abs() < 0.01 && alignment.invert_candidate);
    }
    let ir = blend(
        &a,
        &b,
        Blend {
            balance: 0.5,
            reference_gain: 1.,
            candidate_gain: 1.,
            advance_frames: alignment.advance_frames,
            invert_candidate: alignment.invert_candidate,
            frames: a.len().max(b.len()),
        },
    )?;
    crate::ir_capture::ir_wav(out.join("cab-blend.wav"), &ir, rate)?;
    if fixture {
        let mut chain = ProcessorChain::default();
        chain.push(NamWaveNet::from_json(&serde_json::to_vec(
            &crate::nam_wavenet::reference_model(),
        )?)?);
        chain.push(soniclaw_dsp::Convolution::new(ir, f64::from(rate), 128)?);
        let input = (0..32_768)
            .map(|i| {
                let t = i as f32 / 48_000.;
                let phase = std::f32::consts::TAU * 110. * t;
                (1..=6)
                    .map(|k| (k as f32 * phase).sin() * (0.35 / k as f32) * (-6. * t).exp())
                    .sum()
            })
            .collect();
        let (report, _) = render(out, "wavenet-cabinet-chain", Box::new(chain), input)?;
        return Ok(serde_json::json!({"alignment":alignment,"amp_and_cabinet":report}));
    }
    Ok(serde_json::json!({"alignment":alignment,"sample_rate":rate}))
}
