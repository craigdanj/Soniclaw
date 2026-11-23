use soniclaw::studio::{
    export::{self, Depth, Format, Normalize, Settings},
    jobs::{Priority, Scheduler},
    project::{Project, ProjectError},
    Builtins,
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-export NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    for t in &mut d.tracks {
        t.midi_input = false;
    }
    let p = Project::new("Export demo".into(), d, a, Default::default())?;
    let pool = Scheduler::new(Default::default())?;
    let manager = soniclaw_formats::FormatManager::native_default()?;
    let mut reports = vec![];
    for (i, (format, depth)) in [
        (Format::Wav, Depth::Float32),
        (Format::Aiff, Depth::Pcm16),
        (Format::Flac, Depth::Pcm24),
    ]
    .into_iter()
    .enumerate()
    {
        let project = p.clone();
        let path = root.join(format!("export-{i}"));
        let output = path.clone();
        let settings = Settings {
            format,
            depth,
            stems: vec!["bass".into()],
            length_frames: 48000,
            tail_frames: 2400,
            normalize: Normalize::Peak(0.6),
            dither: depth != Depth::Float32,
            ..Default::default()
        };
        let settings = Settings::from_json(&settings.to_json(&project)?, &project)?;
        let mut ticket = pool.submit(
            "export",
            0,
            Priority::Normal,
            64 * 1024 * 1024,
            move |ctx| {
                export::render(&project, settings, &output, &mut Builtins, &ctx)
                    .map_err(|e| e.to_string())
            },
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let report = loop {
            if let Some(r) = ticket.poll(0) {
                break r?;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        for f in &report.files {
            let mut reader = manager.open(&path.join(&f.file))?;
            assert_eq!(reader.info().frames, 50400);
            let mut block = vec![0.; 8192];
            let mut peak = 0f32;
            loop {
                let n = reader.read_frames(&mut block)?;
                if n == 0 {
                    break;
                }
                for x in &block[..n * 2] {
                    peak = peak.max(x.abs());
                }
            }
            assert!((peak - 0.6).abs() < 0.001);
        }
        reports.push(report);
    }
    let project = p.clone();
    let path = root.join("cancelled");
    let mut ticket = pool.submit(
        "cancelled",
        0,
        Priority::Normal,
        64 * 1024 * 1024,
        move |ctx| {
            export::render(&project, Default::default(), &path, &mut Builtins, &ctx)
                .map_err(|e| e.to_string())
        },
    )?;
    ticket.cancel();
    loop {
        if let Some(r) = ticket.poll(0) {
            assert!(r.is_err());
            break;
        }
        std::thread::yield_now();
    }
    assert!(!root.join("cancelled/export.json").exists());
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"exports":reports,"three_codecs_decoded_and_frame_count_verified":true,"cancelled_without_committed_manifest":true,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
