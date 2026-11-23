use soniclaw::{
    core::*,
    studio::{
        self,
        native_state::{Blob, Identity},
        project::{Project, ProjectError},
        workspace,
    },
};
use soniclaw_host::{
    formats::{PluginFormat, TrustedLoader},
    studio::{registry, NativeFactory},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
fn main() -> Result<(), ProjectError> {
    let a: Vec<_> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("--native-scan-child") {
        let f = if a.get(2).map(String::as_str) == Some("clap") {
            PluginFormat::Clap
        } else {
            PluginFormat::Vst3
        };
        // SAFETY: parent explicitly selects the trusted owned fixture for this child.
        return unsafe {
            registry::child(
                f,
                Path::new(a.get(3).ok_or("module")?),
                Path::new(a.get(4).ok_or("output")?),
            )
        };
    }
    let out = PathBuf::from(
        a.get(1)
            .ok_or("usage: soniclaw-daw-plugins NEW_FOLDER [clap|vst3 TRUSTED_MODULE]")?,
    );
    std::fs::create_dir(&out)?;
    let mut p = workspace::blank("Native persistence".into(), 48000, 128)?;
    let id = "native.fixture";
    let module = out.canonicalize()?.join("absent.clap");
    let blob = Blob::new(
        Identity {
            format: "clap".into(),
            module: module.clone(),
            product: "owned.fixture".into(),
            version: "1".into(),
        },
        BTreeMap::from([(7, 0.25)]),
        Some(vec![73; 131072]),
    )?;
    p.native_states.insert(id.into(), blob.clone());
    p.document.tracks[0].inserts.push(studio::Descriptor{id:id.into(),kind:"clap".into(),state:serde_json::json!({"path":module,"plugin_id":"owned.fixture","native_state":true,"bypass":true,"latency_frames":17})});
    let mut history = studio::history::History::default();
    let before = p.clone();
    p.native_states.insert(
        id.into(),
        Blob::new(
            blob.metadata().identity.clone(),
            BTreeMap::from([(7, 0.5)]),
            Some(vec![14; 131072]),
        )?,
    );
    history.record(before, &mut p, "Native state")?;
    history.undo(&mut p)?;
    assert_eq!(p.native_states[id].bytes(), blob.bytes());
    history.redo(&mut p)?;
    p.save_new(&out.join("session"))?;
    let q = Project::open(&out.join("session"))?;
    assert_eq!(q.native_states[id].bytes(), p.native_states[id].bytes());
    assert!(std::fs::metadata(out.join("session/project.json"))?.len() < 65536);
    let loader = TrustedLoader::default();
    let mut owners = vec![];
    let mut factory = NativeFactory {
        helpers: &Default::default(),
        bus_graph: &mut Default::default(),
        mode: soniclaw_host::studio::Mode::Live,
        spec: q.document.spec(),
        timeline: &q.document.timeline,
        assets: &q.assets,
        states: &q.native_states,
        loader: &loader,
        owners: &mut owners,
        last_error: None,
    };
    let compiled = q.document.compile(&q.assets, &mut factory)?;
    assert_eq!(compiled.latency_frames(), 17);
    drop(compiled);
    assert!(owners.is_empty());
    let file = out.join("session/native-states/0.bin");
    std::fs::write(file, vec![0; 131072])?;
    assert!(Project::open(&out.join("session")).is_err());
    let mut native = serde_json::Value::Null;
    if let (Some(f), Some(module)) = (a.get(2), a.get(3)) {
        let format = if f == "clap" {
            PluginFormat::Clap
        } else {
            PluginFormat::Vst3
        };
        // SAFETY: command line explicitly selects trusted owned native fixtures.
        let report = unsafe {
            registry::scan(
                &std::env::current_exe()?,
                Path::new(module),
                format,
                Duration::from_secs(10),
            )
        }?;
        if let Some(e) = &report.error {
            return Err(e.clone().into());
        }
        let mut cache = registry::Registry::default();
        cache.apply(report)?;
        cache.write(&out.join("registry.json"))?;
        let cache = registry::Registry::read(&out.join("registry.json"))?;
        let e = cache.usable(0)?;
        let mut p = workspace::blank("Owned native fixture".into(), 48000, 128)?;
        let mut d = registry::descriptor(e, "native.test", false);
        if module.contains("HostProtocolFixture") {
            d.state["state_mode"] = serde_json::json!("parameters");
        }
        p.document.tracks[0].inserts.push(d.clone());
        // SAFETY: same explicitly selected owned fixture, verified against scan fingerprint.
        let loader = unsafe { TrustedLoader::new(std::slice::from_ref(&e.product.path)) }?;
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
        let mut b = factory.create_detailed(&d)?;
        b.processor.prepare(p.document.spec())?;
        let mut peak = 0f32;
        let mut count = 0;
        for i in 0..48 {
            let mut pcm = [0.25; 256];
            let context = ProcessContext {
                events: &[],
                transport: Transport {
                    playing: true,
                    sample_position: i * 128,
                    ..Default::default()
                },
            };
            let alloc = allocation_counter::measure(|| {
                b.processor
                    .process(&mut AudioBuffer::new(&mut pcm, 2).unwrap(), &context)
            });
            count += alloc.count_total;
            peak = peak.max(pcm.iter().copied().fold(0f32, |a, b| a.max(b.abs())));
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(count, 0);
        assert!(peak > 0.);
        let latency = b.processor.latency_frames();
        drop(b);
        let saved = owners[0].snapshot()?;
        p.document.tracks[0].inserts[0].state["native_state"] = serde_json::json!(true);
        p.native_states.insert(d.id.clone(), saved);
        owners.clear();
        p.save_new(&out.join("native"))?;
        let q = Project::open(&out.join("native"))?;
        let mut factory = NativeFactory {
            helpers: &Default::default(),
            bus_graph: &mut Default::default(),
            mode: soniclaw_host::studio::Mode::Live,
            spec: q.document.spec(),
            timeline: &q.document.timeline,
            assets: &q.assets,
            states: &q.native_states,
            loader: &loader,
            owners: &mut owners,
            last_error: None,
        };
        drop(factory.create_detailed(&q.document.tracks[0].inserts[0])?);
        owners.clear();
        native = serde_json::json!({"format":f,"product":e.product.id,"callback_allocations":count,"latency":latency,"peak":peak,"save_reload":true});
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"version":"0.146.0","blob_bytes":131072,"metadata_under_64k":true,"undo_redo":true,"corruption_rejected":true,"missing_bypass_latency":17,"native":native}),
        )?,
    )?;
    Ok(())
}
