use soniclaw_core::*;
use soniclaw_vst3::{isolation::*, musical::ParameterPoint, Processor};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn render(
    owner: &mut Isolated,
    kill: bool,
    fault: Option<f64>,
) -> Result<(Vec<f32>, u64, bool, Processor)> {
    let mut processor = owner.start_buffered()?;
    let pid = owner.helper_pid().ok_or("no isolated PID")?;
    let progress = Arc::new(AtomicUsize::new(0));
    let worker_progress = progress.clone();
    std::thread::scope(|scope| {
        let audio = scope.spawn(move || {
            let mut pcm = vec![0.; 64 * 512];
            let mut allocations = 0;
            for block in 0..64 {
                let samples = &mut pcm[block * 512..(block + 1) * 512];
                samples.fill(1.);
                let ctx = ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: block as u64 * 256,
                        ..Default::default()
                    },
                };
                let points = if block == 8 {
                    fault.map(|value| ParameterPoint {
                        frame: 0,
                        id: 11,
                        value,
                    })
                } else {
                    None
                };
                let m = allocation_counter::measure(|| {
                    processor
                        .process_automated(
                            &mut AudioBuffer::new(samples, 2).unwrap(),
                            &ctx,
                            points.as_slice(),
                        )
                        .unwrap()
                });
                allocations += m.count_total;
                worker_progress.store(block + 1, Ordering::Release);
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            (pcm, allocations, processor)
        });
        let mut killed = false;
        let mut seen = false;
        while !audio.is_finished() {
            if kill && !killed && progress.load(Ordering::Acquire) >= 16 {
                let status = std::process::Command::new("kill")
                    .arg("-KILL")
                    .arg(pid.to_string())
                    .status()?;
                if !status.success() {
                    return Err("could not terminate owned fixture helper".into());
                }
                killed = true;
            }
            if !seen && owner.poll().is_err() {
                seen = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (pcm, alloc, processor) = audio.join().map_err(|_| "callback worker panicked")?;
        Ok((pcm, alloc, seen, processor))
    })
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 5 {
        return Err(
            "usage: soniclaw-crash-host HELPER REFERENCE.vst3 PROTOCOL.vst3 NEW-DIRECTORY".into(),
        );
    }
    let helper = Path::new(&args[1]).canonicalize()?;
    let reference = Path::new(&args[2]);
    let protocol = Path::new(&args[3]);
    let out = Path::new(&args[4]);
    std::fs::create_dir(out)?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 256,
        channels: 2,
    };
    let config = Config {
        helper,
        response_timeout_ms: 200,
        max_recoveries: 2,
    };
    // SAFETY: command arguments explicitly select trusted test helper/modules and their dependencies.
    let mut owner = unsafe {
        Isolated::load(
            reference,
            Some("E4194457A25F52568C08A76C9D0A6415"),
            spec,
            config.clone(),
        )
    }?;
    let params = owner.parameters()?;
    let gain = params
        .iter()
        .find(|p| p.name == "Gain")
        .ok_or("no Gain parameter")?
        .id;
    let mode = params
        .iter()
        .find(|p| p.name == "Mode")
        .ok_or("no Mode parameter")?
        .id;
    owner.set_parameter(mode, 0.5)?;
    owner.set_parameter(gain, 0.4)?;
    let checkpoint = owner.checkpoint()?;
    std::fs::write(
        out.join("checkpoint.json"),
        serde_json::to_vec_pretty(&checkpoint)?,
    )?;
    let original_pid = owner.helper_pid();
    let (killed, allocations, fault_seen, mut retained) = render(&mut owner, true, None)?;
    let first_status = owner.status();
    if !fault_seen
        || first_status.phase != Phase::Faulted
        || allocations != 0
        || killed[48 * 512..].iter().any(|x| *x != 0.)
    {
        return Err(format!("killed helper did not fail silently: {first_status:?}").into());
    }
    owner.recover()?;
    let recovered_pid = owner.helper_pid();
    if recovered_pid == original_pid {
        return Err("helper was not replaced".into());
    }
    let (recovered, alloc2, fault2, _) = render(&mut owner, false, None)?;
    if alloc2 != 0 || fault2 || recovered[2048..].iter().any(|v| (*v - 0.4).abs() > 1e-5) {
        return Err("checkpoint recovery did not restore gain".into());
    }
    let mut old = [1.; 512];
    retained.process(
        &mut AudioBuffer::new(&mut old, 2)?,
        &ProcessContext {
            events: &[],
            transport: Default::default(),
        },
    );
    if old.iter().any(|x| *x != 0.) {
        return Err("old callback revived after recovery".into());
    }
    owner.stop()?;
    // Fault injection exists only in the opt-in independent fixture.
    let mut faults = Vec::new();
    for (mode, name) in [(0.5, "abort"), (1., "hang")] {
        let mut isolated = unsafe { Isolated::load(protocol, None, spec, config.clone()) }?;
        let start = std::time::Instant::now();
        let (pcm, alloc, seen, _) = render(&mut isolated, false, Some(mode))?;
        let status = isolated.status();
        if !seen
            || alloc != 0
            || status.phase != Phase::Faulted
            || pcm[56 * 512..].iter().any(|v| *v != 0.)
        {
            return Err(format!("injected {name} did not fault silently: {status:?}").into());
        }
        faults.push(serde_json::json!({"fault":name,"status":status,"callback_allocations":alloc,"elapsed_seconds":start.elapsed().as_secs_f64()}));
    }
    owner.recover()?;
    if owner.recover().is_ok() || owner.status().phase != Phase::Quarantined {
        return Err("recovery budget failed to quarantine product".into());
    }
    let report = serde_json::json!({"killed_helper_status":first_status,"original_helper_pid":original_pid,"recovered_helper_pid":recovered_pid,"checkpoint_gain_restored":true,"old_callback_silent":true,"recovery_budget_quarantines":true,"callback_allocations":allocations+alloc2,"injected_faults":faults});
    std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    Ok(())
}
