use soniclaw::{prelude::*, record, studio::*};
#[cfg(feature = "native")]
use std::sync::Arc;
fn render(
    d: &Document,
    a: &soniclaw::clips::Assets,
    block: usize,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let prepared = d.compile(a, &mut Builtins)?;
    let (mut controller, mut processor) = live_pair(prepared, 0, 64)?;
    controller.set_playing(true)?;
    let mut out = vec![0.; 144000 * 2];
    let alloc = allocation_counter::measure(|| {
        for b in out.chunks_mut(block * 2) {
            processor.process(
                &mut AudioBuffer::new(b, 2).unwrap(),
                &ProcessContext {
                    events: &[],
                    transport: Default::default(),
                },
            );
        }
    });
    assert_eq!(alloc.count_total, 0);
    assert_eq!(controller.status().failures, 0);
    assert_eq!(controller.status().playhead, 48000);
    Ok(out)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let path = std::path::PathBuf::from(
        args.get(1)
            .ok_or("usage: soniclaw-studio-demo NEW_DIRECTORY [--device]")?,
    );
    std::fs::create_dir(&path)?;
    let (d, a) = soniclaw_studio_demo::fixture();
    let one = render(&d, &a, 64)?;
    let two = render(&d, &a, 127)?;
    let error = one
        .iter()
        .zip(&two)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(error < 1e-6);
    soniclaw::io::write_wav(path.join("session.wav"), 48000, 2, &one)?;
    let prepared = d.compile(&a, &mut Builtins)?;
    let (mut controller, mut processor) = live_pair(prepared, 0, 64)?;
    let (mut recorder, tap) = record::Recorder::new(1024);
    recorder
        .start(
            path.join("recorded-mix.wav"),
            48000,
            record::RecordSource::Processed,
        )
        .map_err(|e| e.to_string())?;
    processor.attach_recorder(tap);
    controller.set_playing(true)?;
    let mut block = [0.; 512];
    for _ in 0..64 {
        processor.process(
            &mut AudioBuffer::new(&mut block, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    recorder.stop().map_err(|e| e.to_string())?;
    assert_eq!(recorder.summary().dropped_frames, 0);
    let snapshot = controller.snapshot()?;
    std::fs::write(path.join("session.json"), snapshot.to_json()?)?;
    let rev = controller.replace(snapshot.compile(&a, &mut Builtins)?)?;
    for _ in 0..4 {
        processor.process(
            &mut AudioBuffer::new(&mut block, 2)?,
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        );
    }
    controller.collect_retired();
    assert_eq!(controller.status().applied_revision, rev);
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"frames":144000,"partition_error":error,"peak":one.iter().map(|v|v.abs()).fold(0f32,f32::max),"recorded_frames":recorder.summary().written_frames,"revision":rev,"status":controller.status(),"callback_allocations":0,"physical_device_tested":false}),
        )?,
    )?;
    if args.iter().any(|a| a == "--device") {
        #[cfg(feature = "native")]
        {
            let prepared = d.compile(&a, &mut Builtins)?;
            let (mut controller, processor) = live_pair(prepared, 0, 64)?;
            controller.set_playing(true)?;
            let mut engine = Engine::default();
            engine.push(processor);
            let telemetry = Arc::new(soniclaw::device::Telemetry::default());
            let host = soniclaw::device::OutputHost::start(engine, None, 256, vec![], telemetry)
                .map_err(|e| e.to_string())?;
            println!("Playing on {} for five seconds", host.device_name);
            std::thread::sleep(std::time::Duration::from_secs(5));
            drop(host);
        }
        #[cfg(not(feature = "native"))]
        {
            return Err("rebuild with --features native for --device".into());
        }
    }
    Ok(())
}
