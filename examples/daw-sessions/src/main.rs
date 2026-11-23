use soniclaw::{
    core::*,
    studio::{
        self,
        project::{Project, ProjectError},
        workspace::{self, Command, Kind},
    },
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-sessions NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut p = workspace::blank("Sessions".into(), 48000, 128)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    studio::sessions::save(&p, &root.join("song"))?;
    let mut a = studio::midi_recording::prepare(
        &p,
        &mut studio::Builtins,
        studio::midi_recording::Settings {
            track: "keys".into(),
            ..Default::default()
        },
        64,
    )?;
    let mut pcm = vec![0.; 256];
    a.processor.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[
                Event {
                    frame: 17,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                },
                Event {
                    frame: 20,
                    message: MidiMessage::PitchBend {
                        channel: 0,
                        value: 9000,
                    },
                },
            ],
            transport: Default::default(),
        },
    );
    let take = a.recorder.checkpoint(0)?;
    assert!(!take.complete());
    studio::pending::Checkpoint {
        schema: 1,
        project_id: p.project_id.clone(),
        midi: Some(take),
        automation: None,
    }
    .save(&root.join("pending"))?;
    drop(a.processor);
    let (partial, warnings) =
        studio::pending::Checkpoint::load(&root.join("pending"), &p.project_id)?;
    assert!(warnings.is_empty());
    let take = partial
        .midi
        .ok_or("missing checkpoint")?
        .accept_checkpoint()?;
    take.save_new(&root.join("take"))?;
    take.apply(&mut p)?;
    p.midi_takes.push(root.join("take"));
    p.name = "Recorded song".into();
    assert_eq!(
        studio::sessions::save(&p, &root.join("song"))?.generation,
        2
    );
    std::fs::remove_dir_all(root.join("take"))?;
    let loaded = Project::open(&root.join("song"))?;
    assert_eq!(loaded.name, "Recorded song");
    assert_eq!(loaded.document.tracks[1].midi[0].notes.len(), 1);
    let raw = std::fs::read_to_string(loaded.midi_takes[0].join("midi-take.json"))?;
    assert!(studio::midi_recording::Take::from_json(&raw)?
        .events
        .iter()
        .any(|e| matches!(
            e.message,
            studio::midi_recording::Message::Bend { value: 9000, .. }
        )));
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"save_generations":2,"pending_midi_recovered":true,"portable_raw_controllers":true,"source_take_removed_before_reopen":true,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
