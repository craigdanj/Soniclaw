use soniclaw_core::*;
use soniclaw_host::isolation::*;
use std::{path::PathBuf, sync::atomic::Ordering};
fn main() -> Result<(), soniclaw_host::HostError> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err(
            "usage: isolated-clap-host HELPER MODULE PLUGIN_ID NEW_REPORT_DIRECTORY".into(),
        );
    }
    let path = PathBuf::from(&args[4]);
    std::fs::create_dir(&path)?;
    let config = Config {
        helper: PathBuf::from(&args[1]).canonicalize()?,
        module: PathBuf::from(&args[2]).canonicalize()?,
        plugin_id: args[3].clone(),
        sample_rate: 48000,
        max_frames: 64,
        timeout_ms: 500,
        recovery_budget: 2,
        audio_configuration: None,
    };
    // SAFETY: executable example explicitly selects the caller-supplied native fixture/helper.
    let mut isolated = unsafe { Isolated::load(config) }?;
    let pid = isolated.helper_pid()?;
    let checkpoint = isolated.checkpoint()?;
    std::fs::write(
        path.join("checkpoint.json"),
        serde_json::to_vec_pretty(&checkpoint)?,
    )?;
    let layout = isolated.layout().clone();
    let channels = layout
        .inputs
        .iter()
        .map(|b| b.channels)
        .sum::<usize>()
        .max(layout.outputs.iter().map(|b| b.channels).sum());
    let (owner, mut audio) = isolated.start_buffered(256)?;
    let mut block = vec![0.; 64 * channels];
    let mut peak = 0f32;
    let mut allocations = 0;
    for i in 0..80 {
        for frame in block.chunks_mut(channels) {
            frame.fill(1.);
            if channels > 2 {
                frame[2] = 1.;
            }
        }
        let measure = allocation_counter::measure(|| {
            audio.process(
                &mut AudioBuffer::new(&mut block, channels).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: i * 64,
                        tempo: 120.,
                        playing: true,
                    },
                },
            )
        });
        allocations += measure.count_total;
        peak = peak.max(block.iter().map(|s| s.abs()).fold(0., f32::max));
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    let faulted = isolated.faulted();
    let expected_fault = std::env::var_os("SONICLAW_FIXTURE_PROCESS_FAULT").is_some();
    if !expected_fault {
        assert!(!faulted);
        assert!((peak - if channels > 2 { 0.25 } else { 0.5 }).abs() < 1e-6);
        assert_eq!(owner.telemetry().missing_frames.load(Ordering::Relaxed), 0);
    }
    assert_eq!(allocations, 0);
    let missing = owner.telemetry().missing_frames.load(Ordering::Relaxed);
    drop(owner);
    drop(audio);
    assert_eq!(faulted, expected_fault);
    isolated.recover()?;
    let recovered_pid = isolated.helper_pid()?;
    assert_ne!(pid, recovered_pid);
    isolated.restore_checkpoint(checkpoint)?;
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"layout":layout,"peak":peak,"callback_allocations":allocations,"missing_frames":missing,"faulted":faulted,"pid":pid,"recovered_pid":recovered_pid,"checkpoint_restored":true}),
        )?,
    )?;
    Ok(())
}
