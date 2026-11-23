use super::*;
use soniclaw_dsp::spatial::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let impulses = || {
        let mut a: [Vec<f32>; 4] = std::array::from_fn(|_| vec![0.; 2048]);
        a[0][0] = 0.7;
        a[0][701] = 0.25;
        a[1][431] = 0.3;
        a[2][823] = 0.35;
        a[3][0] = 0.65;
        a[3][977] = 0.3;
        a
    };
    let mut source = vec![0.; 48000 * 2];
    source[0] = 0.8;
    source[12000 * 2 + 1] = 0.8;
    let matrix = render_channels(
        out,
        "true-stereo",
        Box::new(MatrixConvolution::new(impulses(), 48000., 64, 1.)?),
        source.clone(),
        2,
    )?
    .0;
    source.resize(144000 * 2, 0.);
    let hybrid = render_channels(
        out,
        "hybrid-room",
        Box::new(HybridReverb::new(
            HybridConfig {
                mix: 1.,
                ..Default::default()
            },
            impulses(),
            48000.,
            64,
        )?),
        source,
        2,
    )?
    .0;
    Ok(serde_json::json!({"cases":[matrix,hybrid],"path_order":["LL","LR","RL","RR"]}))
}
