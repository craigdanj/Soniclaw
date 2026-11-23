use super::*;
use soniclaw_dsp::nam_wavenet::NamWaveNet;

pub fn run(out: &Path, args: &[String]) -> Result<serde_json::Value> {
    let model = match args {
        [] => serde_json::to_vec_pretty(&reference_model())?,
        [path] => std::fs::read(path)?,
        _ => return Err("expected at most one supported .nam file".into()),
    };
    let mut dsp = NamWaveNet::from_json(&model)?;
    if dsp.sample_rate() != 48_000. {
        return Err("the audio example uses a 48 kHz model".into());
    }
    let size = dsp.receptive_field();
    let input: Vec<_> = (0..48_000)
        .map(|i| {
            let t = i as f32 / 48_000.;
            (0.37 * (std::f32::consts::TAU * 137. * t).sin()
                + 0.13 * (std::f32::consts::TAU * 991. * t).sin())
                * (1. - 0.8 * t)
        })
        .collect();
    let (audio, pcm) = render(
        out,
        "nam-wavenet",
        Box::new(NamWaveNet::from_json(&model)?),
        input.clone(),
    )?;
    for (name, values) in [("input.f32le", &input), ("rust.f32le", &pcm)] {
        let bytes: Vec<_> = values.iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(out.join(name), bytes)?;
    }
    std::fs::write(out.join("fixture.nam"), model)?;
    dsp.prepare(spec())?;
    let mut scratch = [0.; 128];
    let start = std::time::Instant::now();
    for block in input.as_chunks::<128>().0.iter().cycle().take(750) {
        scratch.copy_from_slice(block);
        dsp.process(&mut AudioBuffer::new(&mut scratch, 1)?, &context(0));
        std::hint::black_box(&scratch);
    }
    Ok(
        serde_json::json!({"audio":audio,"receptive_field":size,"benchmark_frames":96000,
        "elapsed_seconds":start.elapsed().as_secs_f64(),"optimized":!cfg!(debug_assertions)}),
    )
}

pub(super) fn reference_model() -> serde_json::Value {
    serde_json::json!({"architecture":"WaveNet","version":"0.5.4","sample_rate":48000.,
        "config":{"layers":[
            {"input_size":1,"condition_size":1,"head_size":2,"channels":3,"kernel_size":3,
            "dilations":[1,2],"activation":"Tanh","gated":false,"head_bias":false},
            {"input_size":3,"condition_size":1,"head_size":1,"channels":2,"kernel_size":3,
            "dilations":[8],"activation":"Tanh","gated":false,"head_bias":true}],
            "head":null,"head_scale":0.25},
        "weights":(0..131).map(|i| if i==130 {0.25} else {((i*37%97) as f32-48.)/180.}).collect::<Vec<_>>()})
}
