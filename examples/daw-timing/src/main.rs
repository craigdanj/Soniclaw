use soniclaw::{
    core::*,
    studio::{
        self,
        project::{Project, ProjectError},
        timing,
        workspace::{self, Command, Kind},
    },
};
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: daw-timing NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let mut reference = vec![0.; 1000];
    reference[10] = 1.;
    let mut returned = vec![0.; 2000];
    returned[147] = -0.5;
    let measured = timing::measure_impulse(&reference, &returned, 1000)?;
    assert_eq!(measured.offset_frames, 137);
    let mut p = workspace::blank("Timing".into(), 48000, 256)?;
    workspace::apply(
        &mut p,
        Command::Add {
            id: "keys".into(),
            name: "Keys".into(),
            kind: Kind::Instrument,
        },
    )?;
    let mut calibration = timing::Calibration::new(48000);
    calibration.audio_offset_frames = measured.offset_frames;
    calibration.midi_offset_frames = 64;
    p.layout.calibration = Some(calibration);
    let mut a = studio::midi_recording::prepare(
        &p,
        &mut studio::Builtins,
        studio::midi_recording::Settings {
            track: "keys".into(),
            input_offset_frames: 64,
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
                    frame: 100,
                    message: MidiMessage::NoteOn {
                        channel: 0,
                        note: 60,
                        velocity: 100,
                    },
                },
                Event {
                    frame: 110,
                    message: MidiMessage::PitchBend {
                        channel: 0,
                        value: 9000,
                    },
                },
                Event {
                    frame: 200,
                    message: MidiMessage::NoteOff {
                        channel: 0,
                        note: 60,
                        velocity: 0,
                    },
                },
            ],
            transport: Default::default(),
        },
    );
    drop(a.processor);
    let take = a.recorder.finish(0)?;
    assert_eq!(take.events[0].sample, 100);
    let clip = take.note_clip(1)?;
    let map = soniclaw::transport::TempoMap::compile(&p.document.timeline)?;
    assert_eq!(map.sample_at(clip.notes[0].beat, 48000.)?, 36);
    assert_eq!(
        map.sample_at(clip.notes[0].beat + clip.notes[0].length, 48000.)?,
        136
    );
    take.apply(&mut p)?;
    studio::sessions::save(&p, &root.join("song"))?;
    assert_eq!(
        Project::open(&root.join("song"))?
            .layout
            .calibration
            .as_ref()
            .ok_or("missing calibration")?
            .audio_offset_frames,
        137
    );
    let (position, trim) = timing::placement(100, 137)?;
    assert_eq!((position, trim), (0, 37));
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"measurement":measured,"raw_midi_note_on":100,"compensated_note_on":36,"compensated_note_off":136,"boundary_source_trim":37,"portable_calibration":true,"synthetic_only":true}),
        )?,
    )?;
    Ok(())
}
