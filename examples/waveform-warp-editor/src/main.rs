use soniclaw::{
    studio::{
        project::{Project, ProjectError},
        tempo_cache::Mode,
        wave_edit::Draft,
    },
    waveform,
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: waveform-warp-editor NEW_DIRECTORY")?,
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
        w.write_sample((i as f32 * 440. * std::f32::consts::TAU / 48000.).sin() * 0.2)?;
    }
    w.finalize()?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    let mut p = Project::new(
        "Warp editor".into(),
        d,
        a,
        [("source".into(), source.clone())].into(),
    )?;
    let loaded = waveform::load_or_build(
        &source,
        &root.join("peaks"),
        Default::default(),
        || false,
        |_| {},
    )?;
    let mut draft = Draft::new(&p, "source", "bass", "warped-region", "warped")?;
    draft.recipe.mode = Mode::Repitch;
    draft.recipe.markers[1].beat = 4.;
    draft.insert(24000, 1., &p)?;
    let before = draft.recipe.markers.len();
    assert!(draft.insert(24000, 2., &p).is_err());
    assert_eq!(draft.recipe.markers.len(), before);
    let mut editor = soniclaw::ui::wave_edit::Editor::default();
    let ctx = soniclaw::ui::egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        editor.show(ui, &loaded.thumbnail, &mut draft.recipe);
    });
    output.textures_delta.clear();
    let ready = draft.build(&p, &root.join("cache"), &Default::default())?;
    let mut stale = p.clone();
    stale.project_id.push_str("-different");
    assert!(ready.apply(&mut stale).is_err());
    let ready = draft.build(&p, &root.join("cache2"), &Default::default())?;
    let mut history = soniclaw::studio::history::History::default();
    history.edit(&mut p, "Apply warp", |p| ready.apply(p))?;
    assert_eq!(p.warps.len(), 1);
    history.undo(&mut p)?;
    assert!(p.warps.is_empty());
    history.redo(&mut p)?;
    p.save_new(&root.join("project"))?;
    Project::open(&root.join("project"))?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"markers":3,"duplicate_rejected":true,"headless_widget_drawn":true,"stale_project_rejected":true,"cache_applied_undo_redo_reopened":true,"physical_playback_tested":false}),
        )?,
    )?;
    Ok(())
}
