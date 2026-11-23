use soniclaw::{prelude::*, record::multitrack::*, studio::*};
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: soniclaw-multitrack-session NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let (document, assets) = soniclaw_studio_demo::fixture();
    let (mut control, mut audio) = live_pair(document.compile(&assets, &mut Builtins)?, 0, 64)?;
    let mut block = [0.; 512];
    let context = ProcessContext {
        events: &[],
        transport: Default::default(),
    };
    audio.process(&mut AudioBuffer::new(&mut block, 2)?, &context);
    let sources = vec![
        ("bass".into(), RecordingSource::Track("bass".into())),
        ("lead".into(), RecordingSource::Track("lead".into())),
        ("master".into(), RecordingSource::Master),
    ];
    let (mut recorder, tap) = pair(Config {
        sample_rate: 48000,
        tracks: control.recording_specs(&sources)?,
        pre_roll_frames: 512,
        queue_packets: 512,
        max_block_frames: 256,
    })?;
    control.attach_multitrack(
        tap,
        &sources.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>(),
    )?;
    control.set_playing(true)?;
    for _ in 0..4 {
        audio.process(&mut AudioBuffer::new(&mut block, 2)?, &context);
    }
    recorder.start(
        path.join("take-a"),
        TakePlan {
            id: "a".into(),
            label: "Full ensemble".into(),
            punch: None,
        },
    )?;
    let allocations = allocation_counter::measure(|| {
        for _ in 0..32 {
            audio.process(&mut AudioBuffer::new(&mut block, 2).unwrap(), &context);
        }
    });
    assert_eq!(allocations.count_total, 0);
    let first = recorder.stop()?;
    assert!(first.complete && first.missing_frames == 0);
    recorder.set_armed(1, false)?;
    let begin = recorder.stats().latest_sample + 256;
    recorder.start(
        path.join("take-b"),
        TakePlan {
            id: "b".into(),
            label: "Punched bass and master".into(),
            punch: Some([begin, begin + 1024]),
        },
    )?;
    for _ in 0..8 {
        audio.process(&mut AudioBuffer::new(&mut block, 2)?, &context);
    }
    let second = recorder.stop()?;
    assert!(second.complete && second.missing_frames == 0);
    assert_eq!(second.frames, 1536);
    let silent = hound::WavReader::open(path.join("take-b/lead.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    // Pre-roll retains earlier armed audio; the punch section is unarmed silence.
    assert!(silent[1024..].iter().all(|s| *s == 0.));
    let selection = TakeSelection {
        schema: 1,
        directories: vec![path.join("take-a"), path.join("take-b")],
        selected: "b".into(),
    };
    std::fs::write(path.join("takes.json"), selection.to_json()?)?;
    assert_eq!(
        TakeSelection::from_json(&selection.to_json()?)?
            .resolve()?
            .1
            .id,
        "b"
    );
    control.detach_multitrack()?;
    audio.process(&mut AudioBuffer::new(&mut block, 2)?, &context);
    assert_eq!(control.collect_recorders(), 1);
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"first":first,"second":second,"stats":recorder.stats(),"callback_allocations":allocations.count_total,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
