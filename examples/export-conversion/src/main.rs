use soniclaw::studio::{
    export::{self, Settings},
    jobs::{Priority, Scheduler},
    project::{Project, ProjectError},
    Builtins,
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: export-conversion NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    d.auxiliaries.clear();
    d.mixer.taps.clear();
    for s in &mut d.mixer.strips {
        s.sends.clear();
    }
    for t in &mut d.tracks {
        t.midi_input = false;
        t.midi.clear();
    }
    let p = Project::new("Converted export".into(), d, a, Default::default())?;
    let pool = Scheduler::new(Default::default())?;
    let manager = soniclaw_formats::FormatManager::native_default()?;
    let mut reports = vec![];
    for rate in [44100, 96000, 8000] {
        let p = p.clone();
        let path = root.join(rate.to_string());
        let directory = path.clone();
        let settings = Settings {
            output_rate: Some(rate),
            length_frames: 48000,
            tail_frames: 0,
            ..Default::default()
        };
        let mut job = pool.submit("convert", 0, Priority::Normal, 64 * 1024 * 1024, move |c| {
            export::render(&p, settings, &directory, &mut Builtins, &c).map_err(|e| e.to_string())
        })?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let report = loop {
            if let Some(r) = job.poll(0) {
                break r?;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert_eq!(report.sample_rate, rate);
        let mut reader = manager.open(&path.join("mix.wav"))?;
        assert_eq!(reader.info().sample_rate, rate);
        assert_eq!(reader.info().frames, rate as u64);
        let mut block = vec![0.; 8192];
        let mut crossings = 0;
        let mut previous = 0.;
        loop {
            let n = reader.read_frames(&mut block)?;
            if n == 0 {
                break;
            }
            for frame in block[..n * 2].as_chunks::<2>().0 {
                assert!(frame[0].is_finite());
                if previous < 0. && frame[0] >= 0. {
                    crossings += 1;
                }
                previous = frame[0];
            }
        }
        assert!((105..=115).contains(&crossings));
        reports.push(serde_json::json!({"rate":rate,"frames":rate,"positive_zero_crossings":crossings,"duration_seconds":1.}));
    }
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"exports":reports,"project_rate_unchanged":p.document.sample_rate==48000,"bounded_streaming_conversion":true,"no_pitch_shift":true}),
        )?,
    )?;
    Ok(())
}
