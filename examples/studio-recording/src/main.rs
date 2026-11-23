use soniclaw::{
    prelude::*,
    studio::{
        project::{Project, ProjectError},
        recording::{self, Arm, Settings, Source},
        Builtins,
    },
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-recording NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    d.timeline.tempo[0].bpm = 400.;
    for t in &mut d.tracks {
        t.audio.clear();
        t.midi.clear();
        t.midi_input = false;
    }
    let mut p = Project::new("Recording controls".into(), d, a, Default::default())?;
    let settings = Settings {
        arms: vec![Arm {
            channels: vec![],
            track: "bass".into(),
            source: Source::DryInput,
            monitor_input: false,
        }],
        master: true,
        count_in_beats: 1,
        pre_roll_seconds: 0.01,
        punch_beats: Some([0.5, 1.5]),
        metronome: true,
        ..Default::default()
    };
    let text = settings.to_json(&p.document)?;
    std::fs::write(root.join("settings.json"), &text)?;
    let mut armed = recording::prepare(
        &p,
        &mut Builtins,
        Settings::from_json(&text, &p.document)?,
        &root.join("take"),
        "take",
        64,
    )?;
    let mut pcm = vec![0.2; 254];
    while !armed.progress.status().punch_complete {
        pcm.fill(0.2);
        armed.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    let take = armed.recorder.stop()?;
    assert_eq!(take.frames, 7744);
    assert_eq!(take.missing_frames, 0);
    let raw = hound::WavReader::open(root.join("take/bass.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    assert!(raw.iter().all(|&x| x == 0.2));
    let master = hound::WavReader::open(root.join("take/master.wav"))?
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()?;
    assert!(master.iter().all(|&x| x == 0.));
    armed.controller.detach_multitrack()?;
    armed.processor.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    armed.controller.collect_retired();
    p.takes.push(root.join("take"));
    p.save_new(&root.join("project"))?;
    Project::open(&root.join("project"))?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"schedule":armed.schedule,"frames":take.frames,"missing_frames":0,"dry_input_exact":true,"monitor_click_excluded_from_master":true,"portable_take_reopened":true,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
