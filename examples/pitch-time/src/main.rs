use soniclaw::{dsp::streaming::*, prelude::*, quality};
fn shifted(
    input: &[f32],
    block: usize,
    settings: PitchSettings,
) -> Result<(Vec<f32>, PitchStats), Box<dyn std::error::Error>> {
    let mut pitch = PitchShifter::new(7., settings)?;
    pitch.prepare(ProcessSpec {
        sample_rate: 48000.,
        channels: 1,
        max_block_frames: 1024,
    })?;
    let latency = pitch.latency_frames();
    let mut pcm = input.to_vec();
    pcm.resize(pcm.len() + latency, 0.);
    for (i, b) in pcm.chunks_mut(block).enumerate() {
        pitch.process(
            &mut AudioBuffer::new(b, 1)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: (i * block) as u64,
                    ..Default::default()
                },
            },
        );
    }
    Ok((pcm[latency..].to_vec(), pitch.stats()))
}
fn stretch(input: &[f32], ratio: f64) -> Result<(Vec<f32>, u64), Box<dyn std::error::Error>> {
    let mut p = TimeStretcher::new(StretchSettings {
        ratio,
        window_frames: 1024,
        search_frames: 128,
        max_block_frames: 1024,
        ..Default::default()
    })?;
    let mut output = Vec::with_capacity((input.len() as f64 * ratio).ceil() as usize + 2);
    let mut tmp = [0.; 514];
    let mut offset = 0;
    while offset < input.len() / 2 {
        let end = (offset + 511).min(input.len() / 2);
        let consumed = p.push(&input[offset * 2..end * 2])?;
        offset += consumed;
        let produced = p.pull(&mut tmp, 8)?;
        output.extend_from_slice(&tmp[..produced * 2]);
        if consumed == 0 && produced == 0 {
            return Err("stream deadlock".into());
        }
    }
    p.finish();
    while !p.is_drained() {
        let produced = p.pull(&mut tmp, 8)?;
        if produced == 0 {
            return Err("stream failed to drain".into());
        }
        output.extend_from_slice(&tmp[..produced * 2]);
    }
    Ok((output, p.transient_locks()))
}
fn frequency(input: &[f32]) -> f64 {
    let mut crossings = Vec::new();
    for (i, w) in input.windows(2).enumerate() {
        if w[0] <= 0. && w[1] > 0. {
            crossings.push(i as f64 + (-w[0] / (w[1] - w[0])) as f64)
        }
    }
    48000. * (crossings.len() - 1) as f64 / (crossings.last().unwrap() - crossings[0])
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-pitch-time NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&out)?;
    let noise = quality::deterministic_noise(96000, 0.4, 34)?;
    let source: Vec<_> = (0..96000)
        .map(|i| {
            let phase = std::f64::consts::TAU * 440. * i as f64 / 48000.;
            let base = (phase.sin() * 0.2 + (phase * 2.).sin() * 0.025) as f32;
            let transient = if (48000..48240).contains(&i) {
                noise[i]
            } else {
                0.
            };
            base + transient
        })
        .collect();
    soniclaw::io::write_wav(out.join("source.wav"), 48000, 1, &source)?;
    let settings = PitchSettings::default();
    let (locked, stats) = shifted(&source, 257, settings)?;
    let (partitioned, _) = shifted(&source, 64, settings)?;
    assert_eq!(locked, partitioned);
    let independent_settings = PitchSettings {
        phase_locking: false,
        transient_threshold: None,
        ..settings
    };
    let (independent, _) = shifted(&source, 257, independent_settings)?;
    let hz = frequency(&locked[5000..20000]);
    let expected = 440. * 2f64.powf(7. / 12.);
    assert!((hz - expected).abs() < 5.);
    assert!(stats.transient_channel_resets > 0);
    soniclaw::io::write_wav(out.join("pitch-phase-locked.wav"), 48000, 1, &locked)?;
    soniclaw::io::write_wav(out.join("pitch-independent.wav"), 48000, 1, &independent)?;
    let stereo: Vec<_> = source.iter().flat_map(|v| [*v, *v * 0.5]).collect();
    let mut stretches = Vec::new();
    for ratio in [0.75, 1.5] {
        let (pcm, transient_locks) = stretch(&stereo, ratio)?;
        let frames = pcm.len() / 2;
        assert_eq!(frames, (96000f64 * ratio).ceil() as usize);
        soniclaw::io::write_wav(out.join(format!("stretch-{ratio}.wav")), 48000, 2, &pcm)?;
        stretches.push(serde_json::json!({"ratio":ratio,"frames":frames,"transient_locks":transient_locks,"statistics":quality::statistics(&pcm)?}));
    }
    let report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"input_frames":96000,"pitch_settings":settings,"pitch_stats":stats,"pitch_target_hz":expected,"measured_pitch_hz":hz,"partition_error":0.,"pitch_frames":locked.len(),"pitch_latency_frames":settings.window_frames,"pitch_statistics":quality::statistics(&locked)?,"stretches":stretches,"listening_acceptance":"pending; files provided for review"});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("Pitch partition equivalence, target frequency, transient resets and exact stretched durations passed");
    Ok(())
}
