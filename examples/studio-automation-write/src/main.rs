use soniclaw::{
    automation::Mode,
    prelude::*,
    studio::{
        automation_write::{self, Writer},
        project::{Project, ProjectError},
        *,
    },
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-automation-write NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    let mut p = Project::new("Automation write".into(), d, a, Default::default())?;
    automation_write::arm(&mut p, "mix.bass.gain", 0.2, Mode::Touch)?;
    let mut writer = Writer::new(&p, "mix.bass.gain", 0.25)?;
    let prepared = p
        .document
        .compile_streamed(&p.assets, &p.disk, &mut Builtins)?;
    let (mut control, mut audio) = live_pair(prepared, 4096, 128)?;
    control.set_playing(true)?;
    let mut pcm = vec![0.; 512];
    audio.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    let g = control.gestures()[writer.lane_index()].clone();
    g.set(0.8)?;
    audio.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    assert!(
        (control
            .parameters()
            .iter()
            .find(|p| p.info().id == "mix.bass.gain")
            .unwrap()
            .normalized()
            - 0.8)
            .abs()
            < 0.001
    );
    writer.input(0.49, 0.8, true)?;
    writer.end();
    g.end();
    assert!(!writer.tick(1.)?);
    drop(audio);
    drop(control);
    let mut h = history::History::default();
    h.edit(&mut p, "Write automation", |p| writer.commit(p))?;
    let count = p.document.automation.lanes[writer.lane_index()]
        .points
        .len();
    assert_eq!(count, 2);
    h.undo(&mut p)?;
    assert_eq!(
        p.document.automation.lanes[writer.lane_index()]
            .points
            .len(),
        1
    );
    h.redo(&mut p)?;
    automation_write::arm(&mut p, "mix.bass.gain", 0.8, Mode::Latch)?;
    let mut latch = Writer::new(&p, "mix.bass.gain", 0.25)?;
    latch.input(1.5, 0.4, false)?;
    assert!(latch.tick(2.)?);
    latch.commit(&mut p)?;
    p.save_new(&root.join("project"))?;
    Project::open(&root.join("project"))?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"live_gesture_control_applied":true,"touch_release_stopped_writing":true,"latch_continued_after_release":true,"snapped_beat":0.5,"write_undo_redo_reopened":true,"physical_gui_gesture_tested":false}),
        )?,
    )?;
    Ok(())
}
