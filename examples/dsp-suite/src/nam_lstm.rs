use super::*;
use soniclaw_dsp::nam_lstm::NamLstm;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let n = 4;
    let layers = 2;
    let count = 4 * n * (1 + n) + 6 * n + (layers - 1) * (8 * n * n + 6 * n) + n + 1;
    let weights: Vec<_> = (0..count)
        .map(|i| 0.35 * (i as f32 * 0.756).sin())
        .collect();
    let bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"version":"0.5.4","architecture":"LSTM","config":{"num_layers":layers,"input_size":1,"hidden_size":n},"weights":weights,"sample_rate":48000}),
    )?;
    std::fs::write(out.join("fixture.nam"), &bytes)?;
    let x: Vec<_> = (0..12000)
        .map(|i| 0.6 * (std::f32::consts::TAU * 315. * i as f32 / 48000.).sin())
        .collect();
    std::fs::write(
        out.join("input.f32le"),
        x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
    )?;
    let mut model = NamLstm::from_json(&bytes)?;
    assert!(model
        .prepare(ProcessSpec {
            sample_rate: 44100.,
            ..spec()
        })
        .is_err());
    let (report, pcm) = render(out, "lstm", Box::new(model), x.clone())?;
    std::fs::write(
        out.join("rust.f32le"),
        pcm.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
    )?;
    let mut reference = NamLstm::from_json(&bytes)?;
    for (&a, &b) in x.iter().zip(&pcm) {
        assert_eq!(reference.tick(a), b);
    }
    assert!(NamLstm::from_json(b"{}").is_err());
    Ok(
        serde_json::json!({"layers":layers,"hidden_size":n,"model_rate_hz":reference.sample_rate(),"sample_and_block_agree":true,"fixture":report}),
    )
}
