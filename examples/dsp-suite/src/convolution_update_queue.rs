use super::*;
use soniclaw_dsp::{convolution_update_queue::*, stereo_convolution::*};

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let queue = ConvolutionUpdateQueue::new(8)?;
    let ir = |gain| StereoImpulse::new(vec![gain, 0., -0.05], 1, 48000.);
    let mut cases = Vec::new();
    let (first, first_audio) =
        queue.create(ir(1.)?, ConvolutionMode::ZeroLatency, 0, 256, spec())?;
    let (second, second_audio) =
        queue.create(ir(1.)?, ConvolutionMode::ZeroLatency, 0, 512, spec())?;
    let a = queue.try_replace(&first, ir(0.65)?)?;
    let b = queue.try_replace(&second, ir(0.25)?)?;
    a.wait()??;
    b.wait()??;
    for (name, audio, control) in [
        ("convolver-a", first_audio, first),
        ("convolver-b", second_audio, second),
    ] {
        let input = (0..8192).map(|i| 0.6 * (i as f32 * 0.07).sin()).collect();
        let (mut case, _) = render(out, name, Box::new(audio), input)?;
        let retired = queue.try_collect(&control)?.wait()?;
        assert_eq!(retired, 1);
        case["retired_kernels"] = retired.into();
        cases.push(case);
    }
    Ok(serde_json::json!({ "shared_workers": 1, "cases": cases }))
}
