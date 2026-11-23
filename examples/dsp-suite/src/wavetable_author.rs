use super::*;
use soniclaw_dsp::{
    advanced::wavetable::WavetableOscillator,
    wavetable_author::{compile, Settings},
};
pub fn run(out: &Path) -> Result<serde_json::Value> {
    let a: Vec<_> = (0..512)
        .map(|i| (std::f32::consts::TAU * i as f32 / 512.).sin())
        .collect();
    let b: Vec<_> = (0..512)
        .map(|i| {
            let p = std::f32::consts::TAU * (i + 93) as f32 / 512.;
            0.7 * p.sin() + 0.2 * (3. * p).sin() + 0.1 * (5. * p).sin()
        })
        .collect();
    let source = out.join("selected-cycles.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&source, spec)?;
    for x in a.into_iter().chain(b) {
        writer.write_sample(x)?;
    }
    writer.finalize()?;
    let pcm = hound::WavReader::open(&source)?
        .samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let t = compile(
        &pcm,
        &[0..512, 512..1024],
        Settings {
            size: 512,
            align_phase: true,
            repair_seams: true,
            normalize: true,
        },
    )?;
    let mut osc = WavetableOscillator::new(Arc::new(t.bank), 48000.)?;
    let mut writer = hound::WavWriter::create(out.join("authored-morph.wav"), spec)?;
    for i in 0..48000 {
        writer.write_sample(0.5 * osc.next_sample(220., i as f32 / 48000., 0., 0.5, 1.))?;
    }
    writer.finalize()?;
    Ok(
        serde_json::json!({"phase_shifts":t.phase_shifts,"table_frames":t.cycles.len(),"cycle_samples":512}),
    )
}
