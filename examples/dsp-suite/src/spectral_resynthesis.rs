use super::*;
use soniclaw_dsp::spectral_resynthesis::*;
fn tone(i: usize, hz: f32) -> f32 {
    (std::f32::consts::TAU * hz * i as f32 / 48000.).sin()
}
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let src: Vec<_> = (0..24576)
        .map(|i| 0.22 * tone(i, 375.) + 0.1 * tone(i, 750.) + 0.06 * tone(i, 2740.))
        .collect();
    let control = Controls {
        fundamental_hz: 375.,
        harmonic_gain: 1.,
        residual_gain: 0.,
        width_bins: 3.,
    };
    let mut results = Vec::new();
    let mut stems = Vec::new();
    for (name, c) in [
        ("harmonic", control),
        (
            "residual",
            Controls {
                harmonic_gain: 0.,
                residual_gain: 1.,
                ..control
            },
        ),
    ] {
        let (report, pcm) = render(out, name, Box::new(Resynth::new(1024, 6, c)?), src.clone())?;
        results.push(report);
        stems.push(pcm);
    }
    let max_error = (2048..src.len())
        .map(|i| (stems[0][i] + stems[1][i] - src[i - 1024]).abs())
        .fold(0f32, f32::max);
    assert!(max_error < 1e-5);
    let mut analyzer = Resynth::new(1024, 6, control)?;
    analyzer.prepare(spec())?;
    let mut scratch = src;
    for (i, b) in scratch.chunks_mut(128).enumerate() {
        analyzer.process(&mut AudioBuffer::new(b, 1)?, &context(i * 128));
    }
    let partial = analyzer.partials(0).unwrap()[0].frequency_hz;
    assert!((partial - 375.).abs() < 3.);
    Ok(serde_json::json!({"stems":results,"complement_error":max_error,"first_partial_hz":partial}))
}
