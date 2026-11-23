use soniclaw::studio::{
    project::{Project, ProjectError},
    tempo_cache::Mode,
    wave_edit::Draft,
    *,
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-media-audit NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let source = root.join("source.wav");
    let mut w = hound::WavWriter::create(
        &source,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for i in 0..48000 {
        w.write_sample((i as f32 * 220. * std::f32::consts::TAU / 48000.).sin() * 0.1)?;
    }
    w.finalize()?;
    let unused = root.join("unused.wav");
    std::fs::copy(&source, &unused)?;
    let original = std::fs::read(&unused)?;
    let (mut d, mut a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    for t in &mut d.tracks {
        t.midi_input = false;
    }
    a.insert("unused-memory".into(), a["bass"].clone());
    let mut p = Project::new(
        "Consolidation".into(),
        d,
        a,
        [
            ("source".into(), source),
            ("unused-disk".into(), unused.clone()),
        ]
        .into(),
    )?;
    let mut draft = Draft::new(&p, "source", "lead", "warp-region", "warp-cache")?;
    draft.recipe.mode = Mode::Repitch;
    draft
        .build(&p, &root.join("warp"), &Default::default())?
        .apply(&mut p)?;
    p.freeze("bass", &mut Builtins, &root.join("freeze"), 0, 48000, 0)?;
    let plan = media::audit(&p)?;
    assert_eq!(
        plan.report()
            .entries
            .iter()
            .filter(|e| !e.referenced)
            .count(),
        2
    );
    assert!(
        plan.report()
            .entries
            .iter()
            .find(|e| e.id == "bass")
            .unwrap()
            .referenced
    );
    assert!(
        plan.report()
            .entries
            .iter()
            .find(|e| e.id == "source")
            .unwrap()
            .referenced
    );
    let mut changed = p.clone();
    changed.document.mixer.strips[0].gain = 0.123;
    assert!(media::consolidate(&changed, &plan, &root.join("stale")).is_err());
    assert!(!root.join("stale").exists());
    let report = media::consolidate(&p, &plan, &root.join("consolidated"))?;
    assert_eq!(report.omitted_assets, 2);
    let mut reopened = Project::open(&root.join("consolidated"))?;
    assert!(!reopened.assets.contains_key("unused-memory"));
    assert!(!reopened.disk.contains_key("unused-disk"));
    assert_eq!(reopened.warps.len(), 1);
    assert_eq!(reopened.freezes.len(), 1);
    reopened.unfreeze("bass")?;
    assert!(!reopened.document.tracks[0].audio.is_empty());
    assert_eq!(std::fs::read(&unused)?, original);
    assert!(p.assets.contains_key("unused-memory"));
    assert!(p.disk.contains_key("unused-disk"));
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"consolidation":report,"stale_plan_rejected":true,"freeze_original_dependency_retained":true,"warp_original_dependency_retained":true,"consolidated_copy_reopened_and_unfrozen":true,"original_files_unchanged":true}),
        )?,
    )?;
    Ok(())
}
