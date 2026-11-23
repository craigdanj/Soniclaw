//! Hardware-free walkthrough. Writes only inside the selected new directory.
use soniclaw::{
    dsp::{resample::ResampleQuality, Waveform},
    prelude::*,
    project::ProjectSession,
    state::ProjectState,
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "soniclaw-v040-demo".into()),
    );
    std::fs::create_dir(&out)?;
    let mut rack = soniclaw_project_plugin::create();
    let spec = ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 256,
        channels: 2,
    };
    rack.activate(spec)?;
    let mut source: Vec<f32> = (0..96000)
        .flat_map(|i| {
            let x = (std::f32::consts::TAU * 220.0 * i as f32 / 48000.0).sin() * 0.2;
            [x, x]
        })
        .collect();
    for (block, data) in source.chunks_mut(512).enumerate() {
        if block == 100 {
            let p = rack
                .parameters()
                .iter()
                .find(|p| p.info().id == "eq.gain_db")
                .unwrap();
            p.set(9.0)?;
        }
        rack.process(data, &[])?;
    }
    soniclaw::io::write_wav(out.join("automated-rack.wav"), 48000, 2, &source)?;
    let state = rack.save_state()?;
    std::fs::write(out.join("project.json"), &state)?;
    rack.deactivate();
    rack.load_state(&state)?;
    let project = ProjectState::from_json(&state)?;
    let descriptor = rack.descriptor;
    let mut session = ProjectSession::new(project, &out, 16)?;
    let _edited = session.edit(descriptor, "Cut EQ", |p| {
        p.parameters.insert("eq.gain_db".into(), -6.0);
    })?;
    assert!(session.undo(descriptor)?.is_some());
    assert!(session.redo(descriptor)?.is_some());
    let file = soniclaw::io::read_wav(out.join("automated-rack.wav"))?;
    let converted = file.resampled(44100, ResampleQuality::SincBest)?;
    soniclaw::io::write_wav(out.join("rack-44100.wav"), 44100, 2, &converted.samples)?;
    let mut synth = ExpressiveSynth::<16>::new(Waveform::Saw, Default::default())?;
    synth.set_mpe_zone(Some(MpeZone {
        master: 0,
        first_member: 1,
        last_member: 15,
        member_bend: 48.0,
        master_bend: 2.0,
    }))?;
    let mut engine = Engine::default();
    engine.push(synth);
    engine.prepare(spec)?;
    let events = [
        Event {
            frame: 0,
            message: MidiMessage::NoteOnId {
                channel: 1,
                note: 60,
                velocity: 0.8,
                note_id: 42,
            },
        },
        Event {
            frame: 24000,
            message: MidiMessage::NoteExpression {
                target: NoteTarget {
                    note_id: 42,
                    channel: -1,
                    key: -1,
                },
                expression: NoteExpression::Tuning,
                value: 7.0,
            },
        },
        Event {
            frame: 36000,
            message: MidiMessage::ChannelPressure {
                channel: 1,
                value: 100,
            },
        },
        Event {
            frame: 48000,
            message: MidiMessage::ControlChange {
                channel: 1,
                controller: 74,
                value: 25,
            },
        },
        Event {
            frame: 72000,
            message: MidiMessage::NoteOffId {
                target: NoteTarget {
                    note_id: 42,
                    channel: -1,
                    key: -1,
                },
            },
        },
    ];
    let audio = soniclaw::engine::render(&mut engine, 96000, &events)?;
    soniclaw::io::write_wav(out.join("expressive-synth.wav"), 48000, 2, &audio)?;
    let (mut handle, mut player) =
        soniclaw::stream::open_seekable_wav(out.join("automated-rack.wav"), 1024 * 1024)?;
    player.prepare(spec)?;
    player.seek(48000)?;
    for _ in 0..4 {
        let _ = handle.wait_ready(1, Duration::from_millis(250));
        player.process(
            &mut AudioBuffer::new(&mut [], 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport::default(),
            },
        );
    }
    let mut seek = vec![0.0; 512];
    player.process(
        &mut AudioBuffer::new(&mut seek, 2)?,
        &ProcessContext {
            events: &[],
            transport: Transport::default(),
        },
    );
    soniclaw::io::write_wav(out.join("seek-preview.wav"), 48000, 2, &seek)?;
    println!("Project state/undo/redo, live rack automation, sinc conversion, expressive synth and seekable disk playback completed in {}",out.display());
    println!(
        "Disk cache: {} bytes; underruns: {}; I/O errors: {}",
        handle.pcm_cache_bytes,
        handle.stats.underrun_frames.load(Ordering::Relaxed),
        handle.stats.io_errors.load(Ordering::Relaxed)
    );
    handle.stop();
    Ok(())
}
