use soniclaw::{
    core::*,
    studio::{
        self,
        midi_edit::{self, Control, ControlEvent},
        project::{Project, ProjectError},
        workspace::{self, Command, Kind},
    },
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-midi NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut p = workspace::blank("Expressive MIDI".into(), 48000, 256)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    let mut a = studio::midi_recording::prepare(
        &p,
        &mut studio::Builtins,
        studio::midi_recording::Settings {
            track: "keys".into(),
            clip: "performance".into(),
            ..Default::default()
        },
        0,
    )?;
    let mut pcm = vec![0.; 512];
    a.processor.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[
                Event {
                    frame: 5,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                },
                Event {
                    frame: 20,
                    message: MidiMessage::ControlChange {
                        channel: 0,
                        controller: 64,
                        value: 127,
                    },
                },
                Event {
                    frame: 40,
                    message: MidiMessage::PitchBend {
                        channel: 0,
                        value: 10000,
                    },
                },
                Event {
                    frame: 60,
                    message: MidiMessage::ChannelPressure {
                        channel: 0,
                        value: 90,
                    },
                },
                Event {
                    frame: 90,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                },
                Event {
                    frame: 190,
                    message: MidiMessage::ControlChange {
                        channel: 0,
                        controller: 64,
                        value: 0,
                    },
                },
            ],
            transport: Default::default(),
        },
    );
    drop(a.processor);
    let take = a.recorder.finish(0)?;
    take.apply(&mut p)?;
    assert_eq!(p.document.tracks[1].midi[0].controls.len(), 4);
    let mut h = studio::history::History::default();
    h.edit(&mut p, "Edit bend", |p| {
        midi_edit::upsert(
            p,
            "performance",
            ControlEvent {
                id: 99,
                beat: 0.003,
                channel: 0,
                control: Control::Bend { value: 12000 },
            },
        )
    })?;
    h.undo(&mut p)?;
    assert_eq!(p.document.tracks[1].midi[0].controls.len(), 4);
    h.redo(&mut p)?;
    assert_eq!(p.document.tracks[1].midi[0].controls.len(), 5);
    studio::sessions::save(&p, &root.join("song"))?;
    let p = Project::open(&root.join("song"))?;
    let d = p.arrangement()?;
    let mut player = d.compile(&p.assets, 256, None)?;
    player.render(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        Transport {
            sample_position: 100,
            playing: true,
            ..Default::default()
        },
    )?;
    let events = player.last_events();
    assert!(events
        .iter()
        .any(|e| matches!(e.message, MidiMessage::PitchBend { value: 12000, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e.message, MidiMessage::NoteOnId { note: 60, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e.message, MidiMessage::NoteOffId { .. })));
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"recorded_controller_points":4,"edited_controller_points":5,"sustain_seek_chase_after_key_release":true,"bend_chased":12000,"portable_reopen":true,"undo_redo":true,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
