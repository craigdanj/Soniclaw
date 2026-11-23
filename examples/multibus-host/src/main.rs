use soniclaw_bus::{flex::*, multigraph::*};
use soniclaw_core::*;
use soniclaw_vst3::buses::BusInstance;
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args_os().collect();
    let bundle = std::path::Path::new(
        args.get(1)
            .ok_or("usage: soniclaw-multibus-host reference.vst3 NEW-DIRECTORY")?,
    );
    let directory = std::path::Path::new(args.get(2).ok_or("missing new report directory")?);
    std::fs::create_dir(directory)?;
    let spec = ProcessSpec {
        sample_rate: 48000.,
        max_block_frames: 256,
        channels: 2,
    };
    // SAFETY: user explicitly selected the trusted reference multi-product native bundle.
    let mut owner =
        unsafe { BusInstance::load(bundle, Some("155D45AD35A25FFA9E5F74B42FFE7AE9"), spec) }?;
    owner.set_input_role(1, Role::Sidechain)?;
    let layout = owner.layout().clone();
    if layout.inputs.len() != 2 || layout.outputs.len() != 1 {
        return Err("expected native main/key input buses".into());
    }
    let external = layout.clone();
    let endpoint = |node: Option<&str>, bus, channel| Endpoint {
        node: node.map(str::to_owned),
        bus,
        channel,
    };
    let mut connections = Vec::new();
    for (bus, shape) in layout.inputs.iter().enumerate() {
        for channel in 0..shape.channels {
            connections.push(Connection {
                from: endpoint(None, bus, channel),
                to: endpoint(Some("ducker"), bus, channel),
                gain: 1.,
            });
        }
    }
    for channel in 0..2 {
        connections.push(Connection {
            from: endpoint(Some("ducker"), 0, channel),
            to: endpoint(None, 0, channel),
            gain: 1.,
        });
        connections.push(Connection {
            from: endpoint(None, 0, channel),
            to: endpoint(None, 0, channel),
            gain: 1.,
        });
    }
    let blueprint = Blueprint {
        schema: 1,
        layout: external.clone(),
        nodes: vec![NodeDef {
            id: "ducker".into(),
            layout: layout.clone(),
        }],
        connections,
    };
    std::fs::write(
        directory.join("graph.json"),
        serde_json::to_vec_pretty(&blueprint)?,
    )?;
    let mut callback = Some(owner.start_buffered()?);
    let mut graph = blueprint.build(48000., 256, |_| {
        Ok(Box::new(callback.take().ok_or(Error::InvalidRouting)?))
    })?;
    let latency = graph.latency_frames();
    let result = std::thread::scope(|scope| {
        let handle = scope.spawn(move || {
            let main = vec![1.; 256 * layout.inputs[0].channels];
            let mut key = vec![0.; 256 * layout.inputs[1].channels];
            let mut out = vec![0.; 160 * 512];
            let mut allocations = 0;
            for block in 0..160 {
                key.fill(if (32..64).contains(&block) { 1. } else { 0. });
                let ctx = ProcessContext {
                    events: &[],
                    transport: Transport {
                        sample_position: block as u64 * 256,
                        tempo: 120.,
                        playing: true,
                    },
                };
                let measure = allocation_counter::measure(|| {
                    graph
                        .process(
                            &[&main, &key],
                            &mut [&mut out[block * 512..(block + 1) * 512]],
                            &ctx,
                        )
                        .unwrap()
                });
                allocations += measure.count_total;
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            (out, allocations)
        });
        while !handle.is_finished() {
            owner.owner().poll()?;
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        handle
            .join()
            .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
                "audio thread panicked".into()
            })
    })?;
    owner.owner().stop()?;
    let stats = owner.owner().stats();
    let average = |a: usize, b: usize| {
        result.0[a * 512..b * 512].iter().copied().sum::<f32>() / ((b - a) * 512) as f32
    };
    let baseline = average(16, 24);
    let ducked = average(60, 64);
    let recovered = average(152, 160);
    if latency != 1024
        || result.1 != 0
        || stats.process_errors != 0
        || stats.underruns != 0
        || stats.input_drops != 0
        || stats.output_drops != 0
        || result.0[..2048].iter().any(|x| *x != 0.)
        || (baseline - 2.).abs() > 0.001
        || ducked >= 1.5
        || recovered < 1.99
    {
        return Err(format!("multibus check failed latency={latency} baseline={baseline} ducked={ducked} recovered={recovered} allocations={} stats={stats:?}",result.1).into());
    }
    let report = serde_json::json!({"layout":external,"callback_allocations":result.1,"latency":latency,"baseline_dry_plus_wet":baseline,"key_active_dry_plus_wet":ducked,"key_released_dry_plus_wet":recovered,"delay_compensated_dry_path":true,"stats":stats});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
