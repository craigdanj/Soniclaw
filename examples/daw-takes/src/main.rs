use soniclaw::{
    core::*,
    studio::{
        self,
        project::{Project, ProjectError},
        recording::{self, Arm, Settings, Source},
        workspace::{self, Command, Kind},
    },
};
use std::{
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};
fn run(root: &Path) -> Result<(), ProjectError> {
    std::fs::create_dir(root)?;
    let mut p = workspace::blank("Loop takes".into(), 48000, 128)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "mic-b".into(),
            name: "Mic B".into(),
            kind: Kind::Audio,
        },
    )?;
    p.document.timeline.loop_range = Some([0., 256. / 24000.]);
    p.document.tracks[0].inserts.push(studio::Descriptor {
        id: "mic-fx".into(),
        kind: "gain".into(),
        state: serde_json::json!({"value":0.5}),
    });
    p.document.mixer.strips[0].solo = true;
    p.document
        .parameter_values
        .insert("mix.audio-1.solo".into(), 1.);
    let bus = Arc::new(ChannelBus::new(2, 128)?);
    let settings = Settings {
        arms: vec![
            Arm {
                track: "audio-1".into(),
                source: Source::DryInput,
                monitor_input: false,
                channels: vec![0],
            },
            Arm {
                track: "mic-b".into(),
                source: Source::DryInput,
                monitor_input: false,
                channels: vec![1],
            },
        ],
        ..Default::default()
    };
    let path = root.join("audio-take");
    let mut a = recording::prepare_with_inputs(
        &p,
        &mut studio::Builtins,
        settings,
        &path,
        "grouped",
        0,
        Some(bus.clone()),
    )?;
    std::fs::write(
        path.join("recording-schedule.json"),
        serde_json::to_vec(&a.schedule)?,
    )?;
    let mut pcm = vec![0.; 256];
    let mut input = vec![0.; 256];
    for block in 0..9 {
        let level = 0.1 + 0.1 * (block / 2) as f32;
        for frame in input.as_chunks_mut::<2>().0 {
            *frame = [level, level * 2.];
        }
        bus.write(&input)?;
        a.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    drop(a.processor);
    let take = a.recorder.stop()?;
    assert_eq!(take.missing_frames, 0);
    let mut recipe = soniclaw::record::comp::Recipe::new(&take)?;
    assert_eq!(recipe.passes.len(), 4);
    recipe.choose(&take, 128, 256, 2)?;
    recipe.crossfade_frames = 16;
    p.takes.push(path.clone());
    let mut history = studio::history::History::default();
    history.edit(&mut p, "Grouped comp", |p| {
        studio::takes::apply_group(p, "drums", 0, &recipe)
    })?;
    assert_eq!(p.layout.take_groups["drums"].destinations.len(), 2);
    assert!(p.document.mixer.strips[0].mute);
    assert!(!p.document.mixer.strips[0].solo);
    let comp = p.layout.take_groups["drums"].destinations["audio-1"].clone();
    assert_eq!(
        p.document
            .tracks
            .iter()
            .find(|t| t.id == comp)
            .ok_or("comp missing")?
            .inserts
            .len(),
        1
    );
    studio::takes::set_active(&mut p, "drums", false)?;
    assert!(p.document.mixer.strips[0].solo);
    studio::takes::set_active(&mut p, "drums", true)?;
    studio::takes::apply_group(&mut p, "drums", 0, &recipe)?;
    assert_eq!(
        p.document
            .tracks
            .iter()
            .find(|t| t.id == comp)
            .ok_or("comp missing")?
            .inserts
            .len(),
        1
    );
    p.document
        .compile_streamed(&p.assets, &p.disk, &mut studio::Builtins)?;
    studio::sessions::save(&p, &root.join("song"))?;
    assert_eq!(
        Project::open(&root.join("song"))?.layout.take_groups.len(),
        1
    );
    history.undo(&mut p)?;
    assert!(p.layout.take_groups.is_empty());
    history.redo(&mut p)?;
    assert_eq!(p.layout.take_groups.len(), 1);
    workspace::apply(
        &mut p,
        Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    let settings = studio::midi_recording::Settings {
        track: "keys".into(),
        clip: workspace::identity("midi")?,
        looping: true,
        ..Default::default()
    };
    let mut midi = studio::midi_recording::prepare(&p, &mut studio::Builtins, settings, 0)?;
    for block in 0..8 {
        let events = if block % 2 == 0 {
            vec![
                Event {
                    frame: 15,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60 + (block / 2) as u8,
                        velocity: 100,
                    },
                },
                Event {
                    frame: 95,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        note: 60 + (block / 2) as u8,
                        velocity: 0,
                    },
                },
            ]
        } else {
            vec![]
        };
        midi.processor.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &events,
                transport: Default::default(),
            },
        );
    }
    drop(midi.processor);
    let midi = midi.recorder.finish(0)?;
    let midi_passes = midi.passes()?.len();
    assert_eq!(midi_passes, 4);
    midi.save_new(&root.join("midi-take"))?;
    p.midi_takes.push(root.join("midi-take"));
    midi.apply(&mut p)?;
    midi.apply_pass(&mut p, 1)?;
    let keys = p
        .document
        .tracks
        .iter()
        .find(|t| t.id == "keys")
        .ok_or("keys missing")?;
    assert_eq!(keys.midi[0].notes[0].key, 61);
    std::fs::write(root.join("selected-pass.mid"), midi.pass(1)?.smf(960)?)?;
    studio::sessions::save(&p, &root.join("song"))?;
    let loaded = Project::open(&root.join("song"))?;
    assert_eq!(
        loaded
            .layout
            .midi_selections
            .values()
            .next()
            .ok_or("selection missing")?
            .pass,
        1
    );
    assert_eq!(loaded.midi_takes.len(), 1);
    let interrupted = root.join("interrupted.wav");
    std::fs::copy(path.join("audio-1.wav"), &interrupted)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&interrupted)?;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&[0; 4])?;
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&[0; 4])?;
    file.seek(SeekFrom::End(0))?;
    file.write_all(&[0xaa, 0xbb])?;
    file.sync_all()?;
    drop(file);
    let original = std::fs::read(&interrupted)?;
    let recovered = soniclaw::record::salvage::recover(&interrupted, &root.join("recovered.wav"))?;
    assert_eq!(recovered.recovered_frames, take.frames);
    assert_eq!(recovered.discarded_bytes, 2);
    assert_eq!(std::fs::read(&interrupted)?, original);
    assert!(soniclaw::record::salvage::recover(&interrupted, &root.join("recovered.wav")).is_err());
    let mut poisoned = original.clone();
    poisoned[44..48].copy_from_slice(&f32::NAN.to_le_bytes());
    std::fs::write(root.join("poisoned.wav"), poisoned)?;
    assert!(soniclaw::record::salvage::recover(
        &root.join("poisoned.wav"),
        &root.join("refused.wav")
    )
    .is_err());
    assert!(!root.join("refused.wav").exists());
    assert_eq!(
        hound::WavReader::open(root.join("recovered.wav"))?.duration() as u64,
        take.frames
    );
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"complete_audio_passes":recipe.passes.len(),"complete_midi_passes":midi_passes,"selected_midi_pass":1,"phase_locked_stems":2,"source_solo_restored":true,"dry_comp_inserts_preserved":true,"portable_group_reopen":true,"recovered_audio":recovered,"original_interrupted_bytes_unchanged":true,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
fn main() -> Result<(), ProjectError> {
    run(Path::new(
        &std::env::args()
            .nth(1)
            .ok_or("usage: daw-takes NEW_DIRECTORY")?,
    ))
}
