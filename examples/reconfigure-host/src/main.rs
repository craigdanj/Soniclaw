use soniclaw_core::*;
use soniclaw_host::{isolation::*, reconfigure::*};
use std::{path::PathBuf, sync::atomic::Ordering};
struct Dry;
impl AudioProcessor for Dry {
    fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let ch = b.channels();
        for f in b.samples_mut().chunks_mut(ch) {
            if ch > 2 {
                f[2..].fill(0.);
            }
        }
    }
    fn reset(&mut self) {}
}
fn routing() -> Routing {
    let c = |bus: &str, channel, external| Connection {
        port: Channel {
            bus: bus.into(),
            channel,
        },
        external,
    };
    Routing {
        channels: 3,
        inputs: vec![c("0", 0, 0), c("0", 1, 1), c("1", 0, 2)],
        silent_inputs: vec![],
        outputs: vec![c("0", 0, 0), c("0", 1, 1)],
        discarded_outputs: vec![],
    }
}
fn main() -> Result<(), soniclaw_host::HostError> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: reconfigure-host HELPER FIXTURE NEW_DIRECTORY".into());
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
        audio_configuration: Some(0),
    };
    // SAFETY: caller explicitly supplies the independent trusted fixture and matching helper executable.
    let mut current = unsafe { Isolated::load(config.clone()) }?;
    let previous = current.topology();
    assert_eq!(previous.latency, 0);
    let old_pid = current.helper_pid()?;
    let (mut owner, mut stale_audio) = current.start_buffered(256)?;
    owner.set_parameter(9, 0.25)?;
    for i in 0..8 {
        let mut pcm = [1.; 128];
        stale_audio.process(
            &mut AudioBuffer::new(&mut pcm, 2)?,
            &ProcessContext {
                events: &[],
                transport: Transport {
                    sample_position: i * 64,
                    tempo: 120.,
                    playing: true,
                },
            },
        );
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    assert!(owner.requires_reconfiguration());
    let requests = owner.control.requests.load(Ordering::Acquire);
    drop(owner);
    let mut invalid = config.clone();
    invalid.audio_configuration = Some(999);
    assert!(current.stage_reconfiguration(invalid).is_err());
    assert_eq!(current.helper_pid()?, old_pid);
    assert!(!current.faulted());
    let mut next = config;
    next.audio_configuration = Some(1);
    let candidate = current.stage_reconfiguration(next.clone())?;
    assert_eq!(candidate.topology().latency, 16);
    assert_eq!(candidate.topology().layout.inputs.len(), 2);
    assert!(candidate.prepare(routing(), 256, 271).is_err());
    assert_eq!(current.helper_pid()?, old_pid);
    let rejected = current
        .stage_reconfiguration(next.clone())?
        .prepare(routing(), 256, 272)?;
    assert!(rejected
        .build::<soniclaw_graph::Graph>(|_, _| Err(Error::InvalidRouting))
        .is_err());
    assert_eq!(current.helper_pid()?, old_pid);
    let stale = current
        .stage_reconfiguration(next.clone())?
        .prepare(routing(), 256, 272)?;
    current.checkpoint()?;
    assert!(stale.commit(&mut current).is_err());
    assert_eq!(current.helper_pid()?, old_pid);
    let prepared = current
        .stage_reconfiguration(next)?
        .prepare(routing(), 256, 272)?
        .build(|processor, plan| {
            let mut graph = soniclaw_graph::Graph::default();
            let wet = graph.add(processor);
            let dry = graph.add(Dry);
            graph.set_input(wet, 1.)?;
            graph.set_input(dry, 1.)?;
            graph.set_output(wet, 1.)?;
            graph.set_output(dry, 1.)?;
            // PreparedChange::build prepares the complete graph and verifies its total PDC budget.
            assert_eq!(plan.next.latency, 16);
            Ok(graph)
        })?;
    let (owner, mut graph, plan) = prepared.commit(&mut current)?;
    assert_ne!(current.helper_pid()?, old_pid);
    assert_eq!(graph.latency_frames(), 272);
    let mut old = [1.; 128];
    stale_audio.process(
        &mut AudioBuffer::new(&mut old, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    assert!(old.iter().all(|&s| s == 0.));
    let mut output = vec![0.; 32 * 64 * 3];
    let mut allocations = 0;
    for block in 0..32 {
        let slice = &mut output[block * 192..(block + 1) * 192];
        for f in slice.as_chunks_mut::<3>().0 {
            f[2] = 1.;
        }
        if block == 0 {
            slice[0] = 1.;
            slice[1] = 1.;
        }
        let measured = allocation_counter::measure(|| {
            graph.process(
                &mut AudioBuffer::new(slice, 3).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: block as u64 * 64,
                        tempo: 120.,
                        playing: true,
                    },
                },
            )
        });
        allocations += measured.count_total;
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    assert_eq!(allocations, 0);
    for (frame, s) in output.as_chunks::<3>().0.iter().enumerate() {
        let expected = if frame == 272 { 1.125 } else { 0. };
        assert_eq!(s[0], expected, "frame {frame}");
        assert_eq!(s[1], expected);
        assert_eq!(s[2], 0.);
    }
    assert_eq!(owner.telemetry().missing_frames.load(Ordering::Acquire), 0);
    assert!(!current.faulted());
    std::fs::write(root.join("plan.json"), plan.to_json()?)?;
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"native_latency_before":previous.latency,"native_latency_after":16,"pipeline_frames":256,"graph_latency_frames":graph.latency_frames(),"impulse_frame":272,"dry_plus_sidechain_wet_impulse":1.125,"callback_allocations":allocations,"request_bits":requests,"failed_config_preserved_owner":true,"failed_graph_preserved_owner":true,"low_budget_rejected":true,"stale_candidate_rejected":true,"old_audio_silent":true,"latest_gain_preserved":true,"native_gui_opened":false}),
        )?,
    )?;
    drop(owner);
    Ok(())
}
