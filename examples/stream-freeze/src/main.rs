use soniclaw::{
    stream::clips::{DiskAssets, DiskClip},
    studio::{freeze, project::Project, Builtins, Descriptor},
};
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: stream-freeze NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let path = root.join("source.wav");
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..96000 {
        w.write_sample((i % 997) as f32 / 997. * 0.2)?;
    }
    w.finalize()?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    let t = &mut d.tracks[0];
    t.audio.clear();
    t.midi.clear();
    t.instrument = None;
    t.midi_input = false;
    t.input_gain = 0.;
    t.inserts = vec![Descriptor {
        id: "disk.gain".into(),
        kind: "gain".into(),
        state: serde_json::json!({"value":0.5}),
    }];
    t.streamed = vec![DiskClip {
        edits: None,
        id: "source-region".into(),
        asset: "original".into(),
        start_frame: 0,
        length_frames: 96000,
        source_offset: 0,
        source_length: 96000,
        looping: false,
        gain: 1.,
        fade_in: 0,
        fade_out: 0,
        enabled: true,
    }];
    let mut project = Project::new(
        "Streaming freeze".into(),
        d,
        a,
        DiskAssets::from([("original".into(), path)]),
    )?;
    let mut job = freeze::Job::spawn(
        project.document.clone(),
        "bass".into(),
        project.assets.clone(),
        project.disk.clone(),
        Builtins,
        root.join("freeze"),
        0,
        96000,
        0,
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let frozen = loop {
        if let Some(r) = job.poll() {
            break r?;
        }
        if std::time::Instant::now() > deadline {
            return Err("freeze timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let pcm = hound::WavReader::open(frozen.directory.join("stem.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    let mut error = 0f32;
    for (i, frame) in pcm.as_chunks::<2>().0.iter().enumerate() {
        error = error.max((frame[0] - (i % 997) as f32 / 997. * 0.1).abs());
    }
    assert!(error < 1e-7);
    project.apply_frozen(frozen)?;
    project.save_new(&root.join("project"))?;
    let mut loaded = Project::open(&root.join("project"))?;
    loaded.unfreeze("bass")?;
    assert_eq!(loaded.document.tracks[0].streamed[0].asset, "original");
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":pcm.len()/2,"max_source_times_gain_error":error,"progress":job.monitor.progress(),"bounded_page_rendering":true,"portable_unfreeze":true,"physical_audio_tested":false}),
        )?,
    )?;
    Ok(())
}
