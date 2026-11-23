use soniclaw::{
    mixer::Tap,
    prelude::*,
    studio::{
        project::{Project, ProjectError},
        routing::{Draft, Edit},
        *,
    },
};
fn rms(p: &Project) -> Result<f64, ProjectError> {
    let mut session = p
        .document
        .compile_offline(&p.assets, &p.disk, &mut Builtins)?;
    let mut data = vec![0.; 512];
    let mut sum = 0.;
    for i in 0..200 {
        session.render_block(
            &mut AudioBuffer::new(&mut data, 2)?,
            &ProcessContext {
                events: &[],
                transport: soniclaw::core::Transport {
                    sample_position: i * 256,
                    playing: true,
                    ..Default::default()
                },
            },
        )?;
        for x in &data {
            sum += (*x as f64).powi(2);
        }
    }
    Ok((sum / (200. * 512.)).sqrt())
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-routing NEW_DIRECTORY")?,
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
        t.midi.clear();
        t.midi_input = false;
    }
    d.mixer.strips[0].gain = 0.5;
    let mut p = Project::new("Routing".into(), d, a, Default::default())?;
    let baseline = rms(&p)?;
    let mut draft = Draft::new(&p)?;
    draft.edit(Edit::AddAux {
        tap: Tap::PostFader,
        return_gain: 1.,
    })?;
    draft.edit(Edit::Send {
        track: "bass".into(),
        aux: 0,
        gain: 1.,
    })?;
    drop(draft.commit(&mut p, &mut Builtins)?);
    let post = rms(&p)?;
    assert!((post / baseline - 2.).abs() < 0.01);
    let stale = Draft::new(&p)?;
    let mut draft = Draft::new(&p)?;
    draft.edit(Edit::Tap {
        index: 0,
        tap: Tap::PreFader,
    })?;
    drop(draft.commit(&mut p, &mut Builtins)?);
    let pre = rms(&p)?;
    assert!((pre / baseline - 3.).abs() < 0.01);
    assert!(stale.commit(&mut p, &mut Builtins).is_err());
    let mut h = history::History::default();
    h.edit(&mut p, "Remove aux", |p| {
        let mut draft = Draft::new(p)?;
        draft.edit(Edit::RemoveAux { index: 0 })?;
        drop(draft.commit(p, &mut Builtins)?);
        Ok(())
    })?;
    assert!(p.document.auxiliaries.is_empty());
    h.undo(&mut p)?;
    p.save_new(&root.join("project"))?;
    let reopened = Project::open(&root.join("project"))?;
    assert_eq!(reopened.document.auxiliaries.len(), 1);
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"baseline_rms":baseline,"post_fader_rms_ratio":post/baseline,"pre_fader_rms_ratio":pre/baseline,"stale_draft_rejected":true,"undo_restored_aux":true,"portable_routes_reopened":true}),
        )?,
    )?;
    Ok(())
}
