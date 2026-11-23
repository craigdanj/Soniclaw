use soniclaw::studio::{
    history::History,
    project::ProjectError,
    workspace::{self, Command, Kind},
    Builtins,
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-workspace NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut p = workspace::blank("New song".into(), 48000, 256)?;
    let mut h = History::default();
    h.edit(&mut p, "Add instrument", |p| {
        workspace::apply(
            p,
            Command::Add {
                id: "keys".into(),
                name: "Keys".into(),
                kind: Kind::Instrument,
            },
        )
    })?;
    workspace::apply(
        &mut p,
        Command::AddMidi {
            track: "keys".into(),
            id: "verse".into(),
            start: 0.,
            length: 4.,
        },
    )?;
    workspace::apply(
        &mut p,
        Command::AddMidi {
            track: "keys".into(),
            id: "chorus".into(),
            start: 4.,
            length: 4.,
        },
    )?;
    workspace::apply(
        &mut p,
        Command::Folder {
            id: "band".into(),
            name: "Band".into(),
        },
    )?;
    workspace::apply(
        &mut p,
        Command::AssignFolder {
            track: "keys".into(),
            folder: Some("band".into()),
        },
    )?;
    h.edit(&mut p, "Duplicate", |p| {
        workspace::apply(
            p,
            Command::Duplicate {
                id: "keys".into(),
                new_id: "keys-copy".into(),
            },
        )
    })?;
    h.undo(&mut p)?;
    assert_eq!(p.document.tracks.len(), 2);
    h.redo(&mut p)?;
    workspace::apply(
        &mut p,
        Command::Move {
            id: "keys-copy".into(),
            index: 0,
        },
    )?;
    assert_eq!(p.document.tracks[0].midi[1].track, 0);
    p.document.compile(&p.assets, &mut Builtins)?;
    p.save_new(&root.join("project"))?;
    let loaded = soniclaw::studio::project::Project::open_with_report(&root.join("project"))?;
    assert_eq!(loaded.source_schema, 3);
    assert_eq!(loaded.project.layout.membership["keys-copy"], "band");
    assert_eq!(loaded.project.document.tracks[0].midi.len(), 2);
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"blank_project":true,"track_lifecycle_and_reindex":true,"multiple_editable_midi_clips":true,"folder_portability":true,"undo_redo":true,"project_schema":3,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
