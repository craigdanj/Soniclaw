#[cfg(target_os = "linux")]
fn main() -> Result<(), soniclaw_pipewire::PipeWireError> {
    use soniclaw_core::*;
    use soniclaw_pipewire::*;
    struct Signal(bool);
    impl AudioProcessor for Signal {
        fn prepare(&mut self, _: ProcessSpec) -> Result<(), Error> {
            Ok(())
        }
        fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
            for s in b.samples_mut() {
                *s = if self.0 { 1. } else { *s * 0.5 };
            }
        }
        fn reset(&mut self) {}
    }
    fn config(name: &str, remote: &str) -> Config {
        Config {
            name: name.into(),
            remote: remote.into(),
            sample_rate: 48000,
            channels: 2,
            max_block_frames: 64,
            requested_quantum: 128,
        }
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: pipewire-probe REMOTE NEW_REPORT_JSON".into());
    }
    let mut source = Host::start(
        config("soniclaw-probe-source", &args[1]),
        Box::new(Signal(true)),
    )?;
    let sink = Host::start(
        config("soniclaw-probe-sink", &args[1]),
        Box::new(Signal(false)),
    )?;
    let source_stats = source.telemetry();
    let sink_stats = sink.telemetry();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let (source_id, sink_id, snapshot) = loop {
        let s = source.snapshot();
        let dest = sink.snapshot();
        if s.node_id != u32::MAX
            && dest.node_id != u32::MAX
            && s.objects
                .iter()
                .filter(|o| o.kind == "port" && (o.node == s.node_id || o.node == dest.node_id))
                .count()
                == 8
        {
            break (s.node_id, dest.node_id, s);
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("PipeWire port registration timed out: {s:?}; {dest:?}").into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let mut handles = Vec::new();
    for channel in 1..=2 {
        let from = snapshot
            .objects
            .iter()
            .find(|o| o.node == source_id && o.name == format!("output_{channel}") && o.output)
            .ok_or("output port absent")?;
        let to = snapshot
            .objects
            .iter()
            .find(|o| o.node == sink_id && o.name == format!("input_{channel}") && !o.output)
            .ok_or("input port absent")?;
        handles.push(source.link(from.id, to.id)?);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let linked = source.snapshot();
    let a = source_stats.snapshot();
    let b = sink_stats.snapshot();
    assert!(a.callbacks > 0 && b.callbacks > 0);
    assert_eq!(b.sample_rate, 48000);
    assert_eq!(b.rate_mismatches, 0);
    assert!((b.input_peak - 1.).abs() < 1e-6);
    assert!((b.output_peak - 0.5).abs() < 1e-6);
    assert!(linked.error.is_empty());
    assert_eq!(linked.registry_overflows, 0);
    let link_count = linked.objects.iter().filter(|o| o.kind == "link").count();
    assert_eq!(link_count, 2);
    source.unlink(handles[0])?;
    std::thread::sleep(std::time::Duration::from_millis(40));
    let unlinked = source.snapshot();
    assert_eq!(
        unlinked.objects.iter().filter(|o| o.kind == "link").count(),
        1
    );
    drop(source);
    std::thread::sleep(std::time::Duration::from_millis(40));
    let removed = sink.snapshot();
    assert!(!removed.objects.iter().any(|o| o.id == source_id));
    let report = serde_json::json!({"source":a,"sink":b,"linked_graph":linked,"links_after_unlink":1,"source_removal_observed":true,"physical_audio_tested":false,"callback_allocation_unit_test":true});
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&args[2])?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("PipeWire probe requires Linux");
    std::process::exit(1);
}
