use soniclaw_core::*;
use soniclaw_host::{isolation::*, OutputEvent, ParameterEvent};
use std::{path::PathBuf, sync::atomic::Ordering};
fn main() -> Result<(), soniclaw_host::HostError> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: isolated-control HELPER FIXTURE NEW_DIRECTORY".into());
    }
    let root = PathBuf::from(&args[3]);
    std::fs::create_dir(&root)?;
    let config = Config {
        helper: PathBuf::from(&args[1]).canonicalize()?,
        module: PathBuf::from(&args[2]).canonicalize()?,
        plugin_id: "org.soniclaw.host-fixture".into(),
        sample_rate: 48000,
        max_frames: 64,
        timeout_ms: 1000,
        recovery_budget: 2,
        audio_configuration: None,
    };
    // SAFETY: the caller explicitly supplies the independently compiled native test fixture/helper.
    let mut isolated = unsafe { Isolated::load(config) }?;
    let (mut owner, mut audio) = isolated.start_buffered(256)?;
    assert_eq!(owner.parameters()[0].id, 9);
    let mut pcm = [1.; 128];
    let mut allocations = 0;
    let mut events = Vec::new();
    let mut split = false;
    for i in 0..64 {
        if i == 16 {
            owner.set_parameter(9, 0.25)?;
        }
        pcm.fill(1.);
        let params = if i == 32 {
            vec![ParameterEvent {
                frame: 32,
                id: 9,
                value: 0.75,
            }]
        } else if i == 33 {
            vec![ParameterEvent {
                frame: 0,
                id: 9,
                value: 0.5,
            }]
        } else {
            vec![]
        };
        let midi = if i == 24 {
            vec![Event {
                frame: 7,
                message: MidiMessage::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100,
                },
            }]
        } else {
            vec![]
        };
        let m = allocation_counter::measure(|| {
            audio
                .process_automated(
                    &mut AudioBuffer::new(&mut pcm, 2).unwrap(),
                    &ProcessContext {
                        events: &midi,
                        transport: Transport {
                            sample_position: (i % 4) * 64,
                            tempo: 120.,
                            playing: true,
                        },
                    },
                    &params,
                )
                .unwrap()
        });
        allocations += m.count_total;
        if i == 36 {
            assert!(pcm[..64].iter().all(|&x| x == 0.25));
            assert!(pcm[64..].iter().all(|&x| x == 0.75));
            split = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
        while let Some(e) = owner.pop_event() {
            events.push(e);
        }
    }
    assert!(split);
    assert_eq!(allocations, 0);
    assert!(!isolated.faulted());
    assert!(events.iter().any(|e| e.render_frame == 32 * 64 + 256 + 32
        && e.source_frame == 32
        && matches!(
            e.event,
            OutputEvent::Parameter(ParameterEvent {
                frame: 32,
                id: 9,
                value: 0.75
            })
        )));
    assert!(events.iter().any(|e| e.render_frame == 24 * 64 + 256 + 7
        && e.source_frame == 7
        && matches!(e.event, OutputEvent::Midi(_))));
    let timers = owner.control.timer_dispatches.load(Ordering::Acquire);
    let fds = owner.control.fd_dispatches.load(Ordering::Acquire);
    assert!(timers > 0);
    assert!(fds > 0);
    assert_eq!(owner.control.mismatched_packets.load(Ordering::Acquire), 0);
    assert_eq!(owner.control.event_drops.load(Ordering::Acquire), 0);
    assert_eq!(owner.control.metadata_drops.load(Ordering::Acquire), 0);
    assert_eq!(owner.telemetry().missing_frames.load(Ordering::Acquire), 0);
    drop(owner);
    pcm.fill(1.);
    audio.process(
        &mut AudioBuffer::new(&mut pcm, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    assert!(pcm.iter().all(|&s| s == 0.));
    let cp = isolated.checkpoint()?;
    assert!(!cp.state.is_empty());
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"sample_accurate_gain_split":split,"callback_allocations":allocations,"output_events":events.len(),"looped_source_clock_preserved":true,"timers_during_playback":timers,"fds_during_playback":fds,"stale_processor_silent":true,"checkpoint_saved":true,"native_gui_opened":false}),
        )?,
    )?;
    Ok(())
}
