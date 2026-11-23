use soniclaw::{
    prelude::*,
    studio::{
        midi_recording::{self, Settings, Take},
        project::{Project, ProjectError},
        *,
    },
};
fn capture(p: &Project, block: usize) -> Result<Take, ProjectError> {
    let settings = Settings {
        track: "lead".into(),
        clip: "recorded-performance".into(),
        grid_beats: 0.25,
        quantize_ends: true,
        ..Default::default()
    };
    let settings = Settings::from_json(&settings.to_json(p)?, p)?;
    let mut a = midi_recording::prepare(p, &mut Builtins, settings, 64)?;
    let input = [
        (
            500,
            MidiMessage::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        ),
        (
            600,
            MidiMessage::ControlChange {
                channel: 0,
                controller: 64,
                value: 127,
            },
        ),
        (
            1000,
            MidiMessage::PitchBend {
                channel: 0,
                value: 9000,
            },
        ),
        (
            9000,
            MidiMessage::NoteOff {
                channel: 0,
                note: 60,
                velocity: 40,
            },
        ),
        (
            12000,
            MidiMessage::NoteOn {
                channel: 0,
                note: 64,
                velocity: 90,
            },
        ),
        (
            20000,
            MidiMessage::NoteOff {
                channel: 0,
                note: 64,
                velocity: 0,
            },
        ),
        (
            25000,
            MidiMessage::NoteOn {
                channel: 1,
                note: 67,
                velocity: 110,
            },
        ),
    ];
    let blocks: Vec<_> = (0..48000)
        .step_by(block)
        .map(|offset| {
            let n = block.min(48000 - offset);
            let events = input
                .iter()
                .filter(|(at, _)| *at >= offset && *at < offset + n)
                .map(|(at, message)| Event {
                    frame: at - offset,
                    message: *message,
                })
                .collect::<Vec<_>>();
            (n, events)
        })
        .collect();
    let mut pcm = vec![0.; 512];
    let allocations = allocation_counter::measure(|| {
        for (n, events) in &blocks {
            pcm[..n * 2].fill(0.);
            a.processor.process(
                &mut AudioBuffer::new(&mut pcm[..n * 2], 2).unwrap(),
                &ProcessContext {
                    events,
                    transport: Default::default(),
                },
            );
        }
    });
    assert_eq!(allocations.count_total, 0);
    drop(a.processor);
    a.recorder.finish(0)
}
fn main() -> Result<(), ProjectError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-midi-recording NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (mut d, a) = soniclaw_studio_demo::fixture();
    d.timeline.loop_range = None;
    let mut p = Project::new("MIDI performance".into(), d, a, Default::default())?;
    let a = capture(&p, 127)?;
    let b = capture(&p, 64)?;
    assert_eq!(
        a.events.iter().map(|e| e.sample).collect::<Vec<_>>(),
        b.events.iter().map(|e| e.sample).collect::<Vec<_>>()
    );
    assert!(a.complete());
    a.save_new(&root.join("take"))?;
    std::fs::write(root.join("take/performance.mid"), a.smf(960)?)?;
    let restored = Take::from_json(&std::fs::read_to_string(root.join("take/midi-take.json"))?)?;
    let smf =
        soniclaw::midi::file::MidiClip::read(&std::fs::read(root.join("take/performance.mid"))?)?;
    assert!(smf
        .events
        .iter()
        .any(|e| matches!(e.message, MidiMessage::PitchBend { value: 9000, .. })));
    assert!(smf.events.iter().any(|e| matches!(
        e.message,
        MidiMessage::ControlChange {
            controller: 64,
            value: 127,
            ..
        }
    )));
    let old = p.document.tracks[1].midi.len();
    let mut history = history::History::default();
    history.edit(&mut p, "Record MIDI performance", |p| {
        drop(restored.apply_prepared(p, &mut Builtins)?);
        Ok(())
    })?;
    assert_eq!(p.document.tracks[1].midi.len(), old + 1);
    let notes = p.document.tracks[1].midi.last().unwrap().notes.len();
    assert_eq!(notes, 3);
    history.undo(&mut p)?;
    assert_eq!(p.document.tracks[1].midi.len(), old);
    history.redo(&mut p)?;
    p.save_new(&root.join("project"))?;
    let p = Project::open(&root.join("project"))?;
    assert_eq!(p.document.tracks[1].midi.len(), old + 1);
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"captured_input_events":a.events.len(),"recorded_notes":notes,"block_partitions":[64,127],"exact_input_frame_offsets":true,"zero_callback_allocations":true,"controllers_and_bend_in_smf":true,"quantized_clip_prepared_undo_redo_reopened":true,"raw_json_roundtrip":true,"physical_midi_tested":false,"native_audio_tested":false}),
        )?,
    )?;
    Ok(())
}
