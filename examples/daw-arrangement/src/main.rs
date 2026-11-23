use soniclaw::{
    core::*,
    studio::{
        self,
        arrangement::{self, AudioAnchor, Command, Marker, Section},
        project::{Project, ProjectError},
        workspace::{self, Kind},
    },
};
use std::sync::Arc;
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-arrangement NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut p = workspace::blank("Song sections".into(), 48000, 128)?;
    p.assets.insert(
        "audio".into(),
        soniclaw::clips::AudioAsset {
            sample_rate: 48000,
            channels: 1,
            samples: Arc::from(vec![0.125; 24000]),
        },
    );
    p.document.tracks[0].audio.push(soniclaw::clips::AudioClip {
        id: "audio-region".into(),
        asset: "audio".into(),
        track: 0,
        start_frame: 0,
        length_frames: 96000,
        source_offset: 0,
        source_length: 24000,
        looping: true,
        gain: 1.,
        fade_in: 1000,
        fade_out: 1000,
        enabled: true,
        edits: None,
    });
    workspace::apply(
        &mut p,
        workspace::Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    workspace::apply(
        &mut p,
        workspace::Command::AddMidi {
            track: "keys".into(),
            id: "midi-region".into(),
            start: 0.,
            length: 4.,
        },
    )?;
    p.document.tracks[1].midi[0]
        .notes
        .push(soniclaw::clips::Note {
            id: 0,
            beat: 2.,
            length: 0.5,
            channel: 0,
            key: 64,
            velocity: 0.8,
        });
    studio::midi_edit::upsert(
        &mut p,
        "midi-region",
        studio::midi_edit::ControlEvent {
            id: 0,
            beat: 2.,
            channel: 0,
            control: studio::midi_edit::Control::Bend { value: 10000 },
        },
    )?;
    p.document
        .automation
        .lanes
        .push(soniclaw::automation::Lane {
            parameter: "mix.audio-1.gain".into(),
            enabled: true,
            mode: soniclaw::automation::Mode::Read,
            points: vec![
                soniclaw::automation::Point {
                    beat: 0.,
                    value: 0.,
                    curve: soniclaw::automation::Curve::Smooth,
                },
                soniclaw::automation::Point {
                    beat: 4.,
                    value: 1.,
                    curve: soniclaw::automation::Curve::Step,
                },
            ],
        });
    let source_lane = p.document.automation.lanes[0].clone();
    arrangement::apply(
        &mut p,
        Command::Marker(Marker {
            id: "verse".into(),
            name: "Verse".into(),
            beat: 1.,
        }),
        AudioAnchor::Seconds,
    )?;
    arrangement::apply(
        &mut p,
        Command::Section(Section {
            id: "section".into(),
            name: "Phrase".into(),
            start: 1.,
            end: 3.,
        }),
        AudioAnchor::Seconds,
    )?;
    let mut h = studio::history::History::default();
    let mut added = std::collections::BTreeSet::new();
    h.edit(&mut p, "Copy section", |p| {
        added = arrangement::duplicate_section(p, "section", 8., true)?;
        Ok(())
    })?;
    assert_eq!(added.len(), 2);
    let copied = p.document.tracks[0]
        .audio
        .iter()
        .find(|c| added.contains(&c.id))
        .ok_or("audio copy missing")?;
    assert_eq!(copied.start_frame, 192000);
    assert_eq!(copied.length_frames, 48000);
    assert_eq!(copied.source_phase() % copied.source_length, 0);
    assert!(
        (copied.region_gain(0) - p.document.tracks[0].audio[0].region_gain(24000)).abs() < 1e-6
    );
    let midi = p.document.tracks[1]
        .midi
        .iter()
        .find(|c| added.contains(&c.id))
        .ok_or("MIDI copy missing")?;
    assert_eq!(midi.start_beat, 8.);
    assert_eq!(midi.source_offset, 1.);
    assert_eq!(midi.controls.len(), 1);
    for i in 0..=64 {
        let x = i as f64 / 32.;
        assert!(
            (p.document.automation.lanes[0].value_at(8. + x).unwrap()
                - source_lane.value_at(1. + x).unwrap())
            .abs()
                < 1e-5
        );
    }
    h.undo(&mut p)?;
    assert_eq!(p.layout.arrangement.sections.len(), 1);
    h.redo(&mut p)?;
    assert_eq!(p.layout.arrangement.sections.len(), 2);
    let mut beats = p.clone();
    let mut timeline = p.document.timeline.clone();
    timeline.tempo[0].bpm = 60.;
    arrangement::change_timeline(&mut beats, timeline.clone(), AudioAnchor::Beats)?;
    assert_eq!(beats.document.tracks[0].audio[1].start_frame, 384000);
    arrangement::change_timeline(&mut p, timeline, AudioAnchor::Seconds)?;
    assert_eq!(p.document.tracks[0].audio[1].start_frame, 192000);
    p = beats;
    studio::sessions::save(&p, &root.join("song"))?;
    let p = Project::open(&root.join("song"))?;
    assert_eq!(p.layout.arrangement.markers[0].name, "Verse");
    let map = soniclaw::transport::TempoMap::compile(&p.document.timeline)?;
    let start = map.sample_at(8., 48000.)?;
    let mut prepared =
        p.document
            .compile_streamed_at(&p.assets, &p.disk, &mut studio::Builtins, start)?;
    let mut pcm = vec![0.; 256];
    prepared.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[],
            transport: Transport {
                sample_position: start,
                playing: true,
                ..Default::default()
            },
        },
    );
    assert!(pcm.iter().any(|v| v.abs() > 1e-6));
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"copied_regions":2,"source_curve_preserved":true,"midi_expression_preserved":true,"loop_audio_phase_preserved":true,"audio_seconds_anchor":192000,"audio_beats_anchor":384000,"chosen_playback_sample":start,"markers_and_sections_portable":true,"undo_redo":true,"hardware_opened":false}),
        )?,
    )?;
    Ok(())
}
