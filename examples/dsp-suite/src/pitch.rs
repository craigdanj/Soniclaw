use super::*;
pub fn run(out: &Path) -> Result<serde_json::Value> {
    use soniclaw_dsp::{pitch_quality::Quality, streaming::FormantSettings};
    let source: Vec<f32> = (0..48000)
        .map(|i| {
            let t = i as f32 / 48000.;
            (1..60)
                .map(|h| {
                    let hz = h as f32 * 140.;
                    let envelope = (-((hz - 850.) / 280.).powi(2)).exp()
                        + 0.6 * (-((hz - 2400.) / 420.).powi(2)).exp();
                    0.04 * envelope * (std::f32::consts::TAU * hz * t).sin()
                })
                .sum()
        })
        .collect();
    let mut cases = vec![];
    for (name, formants) in [
        ("pitch-up-plain", None),
        ("pitch-up-envelope", Some(FormantSettings::default())),
        (
            "pitch-up-shifted-envelope",
            Some(FormantSettings {
                shift_semitones: -3.,
                ..Default::default()
            }),
        ),
    ] {
        let mut p = Quality::Music.pitch(5.)?;
        p.set_formants(formants)?;
        cases.push(render(out, name, Box::new(p), source.clone())?.0);
    }
    let mut stretch = Quality::Music.stretch(1.5, 1, 128)?;
    let mut pos = 0;
    let mut output = vec![];
    let mut block = [0.; 128];
    let mut alloc = 0;
    for _ in 0..10000 {
        let mut accepted = 0;
        let mut produced = 0;
        alloc += allocation_counter::measure(|| {
            if pos < source.len() {
                accepted = stretch
                    .push(&source[pos..(pos + 128).min(source.len())])
                    .unwrap();
            } else {
                stretch.finish();
            }
            produced = stretch.pull(&mut block, 4).unwrap();
        })
        .count_total;
        pos += accepted;
        output.extend_from_slice(&block[..produced]);
        if stretch.is_drained() {
            break;
        }
        assert!(accepted + produced > 0 || pos == source.len());
    }
    assert_eq!(alloc, 0);
    assert_eq!(output.len(), 72000);
    assert!(output.iter().all(|x| x.is_finite()));
    let mut wav = hound::WavWriter::create(
        out.join("stretch-music.wav"),
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for x in output {
        wav.write_sample(x)?;
    }
    wav.finalize()?;
    Ok(
        serde_json::json!({"pitch_cases":cases,"stretch":{"ratio":1.5,"input_frames":source.len(),"output_frames":72000,"callback_allocations":alloc},"fixture":"synthetic harmonics; listening quality not certified"}),
    )
}
