use soniclaw::{
    automation::{AutomatedProcessor, Curve, Document, Lane, Mode, Point},
    core::*,
    studio::{project::ProjectError, workspace, Descriptor},
};
use soniclaw_host::{
    formats::{self, PluginFormat, TrustedLoader},
    studio::NativeFactory,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
fn run(module: &Path, parts: &[usize]) -> Result<(Vec<f32>, u64, String), ProjectError> {
    let p = workspace::blank("Native automation".into(), 48000, 256)?;
    // SAFETY: this example accepts only explicitly selected trusted native modules.
    let products = unsafe { formats::products(PluginFormat::Vst3, module) }?;
    let product = products.first().ok_or("no native products")?;
    // SAFETY: caller selected this module and its dependencies.
    let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&product.path)) }?;
    let d = Descriptor {
        id: "gain".into(),
        kind: "vst3".into(),
        state: serde_json::json!({"path":product.path,"class_id":product.id,"state_mode":"parameters","automation_ids":[9]}),
    };
    let mut owners = vec![];
    let mut factory = NativeFactory {
        helpers: &Default::default(),
        bus_graph: &mut Default::default(),
        mode: soniclaw_host::studio::Mode::Live,
        spec: p.document.spec(),
        timeline: &p.document.timeline,
        assets: &p.assets,
        states: &p.native_states,
        loader: &loader,
        owners: &mut owners,
        last_error: None,
    };
    let built = factory.create_detailed(&d)?;
    let parameter = built.parameters[0].clone();
    assert_eq!(parameter.info().id, "gain.native.9");
    let map = std::sync::Arc::new(soniclaw::transport::TempoMap::compile(
        &p.document.timeline,
    )?);
    let automation = Document {
        lanes: vec![Lane {
            parameter: parameter.info().id.into(),
            enabled: true,
            mode: Mode::Read,
            points: vec![
                Point {
                    beat: 0.,
                    value: 0.25,
                    curve: Curve::Step,
                },
                Point {
                    beat: map.beat_at_sample(64, 48000.)?,
                    value: 0.75,
                    curve: Curve::Step,
                },
                Point {
                    beat: map.beat_at_sample(320, 48000.)?,
                    value: 0.5,
                    curve: Curve::Step,
                },
            ],
        }],
        loop_range: None,
    };
    let (processor, _) = AutomatedProcessor::new(built.processor, automation, &built.parameters)?;
    let mut processor = processor.with_tempo_map(map);
    processor.prepare(p.document.spec())?;
    let mut output: Vec<f32> = Vec::new();
    let mut pos = 0;
    let mut allocations = 0;
    while pos < 4096 {
        let n = parts[(output.len() / 2) % parts.len()].min(4096 - pos);
        let mut pcm = vec![1.; n * 2];
        let ctx = ProcessContext {
            events: &[],
            transport: Transport {
                sample_position: pos as u64,
                playing: true,
                tempo: 120.,
            },
        };
        allocations += allocation_counter::measure(|| {
            processor.process(&mut AudioBuffer::new(&mut pcm, 2).unwrap(), &ctx)
        })
        .count_total;
        output.extend(pcm.iter().step_by(2).copied());
        pos += n;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(allocations, 0);
    assert_eq!(
        owners[0]
            .automation_failures
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert!(
        (output[1024 + 63] - 0.25).abs() < 1e-6,
        "before step {}",
        output[1087]
    );
    assert!(
        (output[1024 + 64] - 0.75).abs() < 1e-6,
        "at step {}",
        output[1088]
    );
    assert!((output[1024 + 320] - 0.5).abs() < 1e-6);
    parameter.set_normalized(0.9)?;
    drop(processor);
    let blob = owners[0].snapshot()?;
    assert!((blob.metadata().parameters[&9] - 0.9).abs() < 1e-6);
    Ok((output, allocations, parameter.info().id.into()))
}
fn main() -> Result<(), ProjectError> {
    let a: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(a.get(1).ok_or("NEW_FOLDER TRUSTED_PROTOCOL_FIXTURE")?);
    let module = Path::new(a.get(2).ok_or("trusted owned protocol fixture")?).canonicalize()?;
    std::fs::create_dir(&out)?;
    let (x, n, id) = run(&module, &[128])?;
    let (y, _, _) = run(&module, &[63, 127, 31, 97])?;
    let error = x
        .iter()
        .zip(&y)
        .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(error < 1e-6, "partition residual {error}");
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","parameter":id,"bridge_latency":1024,"step_source_frame":64,"step_output_frame":1088,"partition_residual":error,"callback_allocations":n,"pending_host_value_saved":true}),
        )?,
    )?;
    Ok(())
}
