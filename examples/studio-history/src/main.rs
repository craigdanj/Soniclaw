use soniclaw::studio::{
    history::History,
    project::{Project, ProjectError},
    Builtins,
};
fn main() -> Result<(), ProjectError> {
    let path = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-history NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    let mut project = Project::new("Project-wide history".into(), d, a, Default::default())?;
    let original = project.document.to_json()?;
    let mut h = History::default();
    h.begin_group(&project, "Move bass and change mix")?;
    h.edit(&mut project, "Move", |p| {
        p.document.tracks[0].audio[0].start_frame = 4800;
        Ok(())
    })?;
    h.edit(&mut project, "Mix", |p| {
        p.document.mixer.strips[0].gain = 0.4;
        Ok(())
    })?;
    h.finish_group(&mut project)?;
    assert_eq!(h.stats()?.undo, 1);
    let restored = h.undo_prepared(&mut project, |p| {
        Ok(p.document
            .compile_streamed(&p.assets, &p.disk, &mut Builtins)?)
    })?;
    assert!(restored.is_some());
    assert_eq!(project.document.to_json()?, original);
    assert!(h.redo(&mut project)?);
    assert_eq!(project.document.tracks[0].audio[0].start_frame, 4800);
    project.save_new(&path.join("project"))?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"grouped_undo_entries":1,"prepared_undo_restored_document":true,"redo_restored_region_and_mix":true,"estimated_retained_bytes":h.stats()?.estimated_retained_bytes,"files_deleted_by_undo":0,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
