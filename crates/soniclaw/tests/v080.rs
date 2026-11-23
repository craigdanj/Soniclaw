use soniclaw::{prelude::*, project_loading::*, project_storage::*, state::*};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "soniclaw-v080-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn descriptor() -> soniclaw::plugin::PluginDescriptor {
    soniclaw::plugin::PluginDescriptor {
        id: "test.storage",
        name: "Storage test",
        vendor: "Soniclaw",
        version: "0.146.0",
        is_instrument: true,
        accepts_midi: true,
    }
}
fn fixture(root: &Path) -> ProjectState {
    soniclaw::io::write_wav(
        root.join("source.wav"),
        48000,
        1,
        &(0..2048)
            .map(|i| (i as f32 * 0.1).sin() * 0.2)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let mut s = ProjectState::empty("test.storage", "Portable sampler");
    s.assets.push(Asset {
        id: "tone".into(),
        path: "source.wav".into(),
        root_note: 60,
    });
    s.zones.push(SampleZone::new("tone"));
    s.nodes
        .push(RackNode::new("sampler", ProcessorKind::Sampler));
    s.outputs.insert("sampler".into(), 1.0);
    s
}
fn spec() -> ProcessSpec {
    ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 64,
        channels: 2,
    }
}
fn poll(loader: &mut AsyncProjectLoader) -> LoadOutcome {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(o) = loader.try_poll() {
            return o;
        }
        assert!(Instant::now() < deadline, "loader timed out");
        std::thread::yield_now();
    }
}
fn request(s: &ProjectState, root: &Path) -> LoadRequest {
    LoadRequest::Snapshot {
        state: Box::new(s.clone()),
        root: root.to_owned(),
    }
}

#[test]
fn portable_bundle_roundtrip_assets_and_prepared_audio() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let b = t.0.join("project.soniclaw");
    pack_bundle(&s, &t.0, &b).unwrap();
    let dest = t.0.join("restored");
    let restored = unpack_bundle(&b, &dest).unwrap();
    assert_eq!(restored.assets[0].path, "assets/0.wav");
    assert_eq!(
        std::fs::read(t.0.join("source.wav")).unwrap(),
        std::fs::read(dest.join("assets/0.wav")).unwrap()
    );
    let mut loader = AsyncProjectLoader::new(descriptor());
    loader
        .submit(LoadRequest::File(dest.join("project.json")), Some(spec()))
        .unwrap();
    let loaded = poll(&mut loader).result.unwrap();
    assert!(loaded.prepared_spec().is_some());
    let mut p = loaded.into_processor();
    let prepare = allocation_counter::measure(|| p.prepare(spec()).unwrap());
    assert_eq!(
        prepare.count_total, 0,
        "same-spec prepared handoff must reuse resources"
    );
    let mut data = [0.0; 128];
    let e = [Event {
        frame: 0,
        message: MidiMessage::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        },
    }];
    p.process(
        &mut AudioBuffer::new(&mut data, 2).unwrap(),
        &ProcessContext {
            events: &e,
            transport: Transport::default(),
        },
    );
    assert!(data.iter().all(|x| x.is_finite()));
    assert!(data.iter().any(|x| x.abs() > 0.001));
}
#[test]
fn bundle_failure_preserves_existing_destinations_and_removes_partial_import() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let b = t.0.join("p.soniclaw");
    pack_bundle(&s, &t.0, &b).unwrap();
    let bytes = std::fs::read(&b).unwrap();
    assert!(pack_bundle(&s, &t.0, &b).is_err());
    assert_eq!(bytes, std::fs::read(&b).unwrap());
    let dest = t.0.join("existing");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join("sentinel"), b"keep").unwrap();
    assert!(unpack_bundle(&b, &dest).is_err());
    assert_eq!(std::fs::read(dest.join("sentinel")).unwrap(), b"keep");
    for (name, mut bytes) in [
        ("digest", bytes.clone()),
        ("truncated", bytes[..bytes.len() - 3].to_vec()),
        ("trailing", bytes.clone()),
    ] {
        if name == "digest" {
            *bytes.last_mut().unwrap() ^= 1;
        }
        if name == "trailing" {
            bytes.push(5);
        }
        let bad = t.0.join(format!("{name}.soniclaw"));
        std::fs::write(&bad, bytes).unwrap();
        let out = t.0.join(name);
        assert!(unpack_bundle(&bad, &out).is_err());
        assert!(!out.exists());
    }
}
#[test]
fn bundle_rejects_traversal_even_with_valid_manifest_digest_and_oversized_headers() {
    use sha2::{Digest, Sha256};
    let t = Temp::new();
    let s = fixture(&t.0);
    let b = t.0.join("p.soniclaw");
    pack_bundle(&s, &t.0, &b).unwrap();
    let mut bytes = std::fs::read(&b).unwrap();
    let len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut json = std::str::from_utf8(&bytes[48..48 + len])
        .unwrap()
        .to_owned();
    json = json.replace("assets/0.wav", "../evil!.wav");
    assert_eq!(json.len(), len);
    bytes[48..48 + len].copy_from_slice(json.as_bytes());
    bytes[16..48].copy_from_slice(&Sha256::digest(json.as_bytes()));
    std::fs::write(&b, &bytes).unwrap();
    assert!(unpack_bundle(&b, &t.0.join("bad")).is_err());
    assert!(!t.0.join("evil!.wav").exists());
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    std::fs::write(&b, bytes).unwrap();
    assert!(unpack_bundle(&b, &t.0.join("huge")).is_err());
}
#[test]
fn recovery_skips_corruption_and_wrong_products_and_rotates_after_commit() {
    let t = Temp::new();
    let mut s = fixture(&t.0);
    let store = RecoveryStore::new(t.0.join("recovery"), 3).unwrap();
    let first = store.checkpoint(&s, &t.0).unwrap();
    s.name = "second".into();
    let second = store.checkpoint(&s, &t.0).unwrap();
    let mut other = s.clone();
    other.product_id = "other.product".into();
    store.checkpoint(&other, &t.0).unwrap();
    std::fs::write(&second, b"broken").unwrap();
    assert_eq!(
        store.latest(&s.product_id).unwrap().unwrap().project.name,
        "Portable sampler"
    );
    assert!(store.latest("missing").unwrap().is_none());
    s.name = "fourth".into();
    store.checkpoint(&s, &t.0).unwrap();
    assert!(!first.exists());
    assert_eq!(
        store.latest(&s.product_id).unwrap().unwrap().project.name,
        "fourth"
    );
    assert_eq!(std::fs::read_dir(t.0.join("recovery")).unwrap().count(), 3);
}
#[test]
fn recovery_digest_mutation_is_rejected_and_user_project_is_unchanged() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let store = RecoveryStore::new(t.0.join("r"), 2).unwrap();
    let p = store.checkpoint(&s, &t.0).unwrap();
    let original = s.to_json().unwrap();
    std::fs::write(t.0.join("project.json"), &original).unwrap();
    let mut e: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    e["payload"] = e["payload"]
        .as_str()
        .unwrap()
        .replace("Portable sampler", "Changed sampler")
        .into();
    std::fs::write(p, serde_json::to_vec(&e).unwrap()).unwrap();
    assert!(store.latest(&s.product_id).unwrap().is_none());
    assert_eq!(
        std::fs::read_to_string(t.0.join("project.json")).unwrap(),
        original
    );
}
#[test]
fn latest_load_wins_and_factory_runs_on_a_separate_thread() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let main = std::thread::current().id();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let mut loader = AsyncProjectLoader::with_factory(move |s, r| {
        assert_ne!(std::thread::current().id(), main);
        if count.fetch_add(1, Ordering::SeqCst) == 0 {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }
        soniclaw::project::create_plugin(descriptor(), s.clone(), r)
    });
    loader.submit(request(&s, &t.0), Some(spec())).unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut newer = s.clone();
    newer.name = "newer".into();
    let generation = loader.submit(request(&newer, &t.0), None).unwrap();
    release_tx.send(()).unwrap();
    let out = poll(&mut loader);
    assert_eq!(out.generation, generation);
    assert_eq!(out.result.unwrap().state().name, "newer");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[test]
fn cancellation_and_full_queue_have_explicit_behavior() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut loader = AsyncProjectLoader::with_factory(move |s, r| {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        soniclaw::project::create_plugin(descriptor(), s.clone(), r)
    });
    loader.submit(request(&s, &t.0), None).unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    loader.submit(request(&s, &t.0), None).unwrap();
    loader.submit(request(&s, &t.0), None).unwrap();
    assert!(loader.submit(request(&s, &t.0), None).is_err());
    loader.cancel();
    release_tx.send(()).unwrap();
    drop(loader); // joins after discarding cancelled queued requests
}
#[test]
fn failed_load_can_be_followed_by_valid_load_and_background_autosave_recovery() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let mut loader = AsyncProjectLoader::new(descriptor());
    loader
        .submit(LoadRequest::File(t.0.join("missing.json")), None)
        .unwrap();
    assert!(poll(&mut loader).result.is_err());
    let d = t.0.join("recovery");
    let worker = AutosaveWorker::new(RecoveryStore::new(&d, 5).unwrap());
    worker.try_checkpoint(s.clone(), t.0.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(r) = worker.try_poll() {
            r.unwrap();
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    loader
        .submit(
            LoadRequest::Recovery {
                directory: d,
                product: s.product_id.clone(),
            },
            Some(spec()),
        )
        .unwrap();
    let loaded = poll(&mut loader).result.unwrap();
    assert_eq!(loaded.state(), &s);
}

#[test]
fn bundle_import_request_loads_resources_from_new_root() {
    let t = Temp::new();
    let s = fixture(&t.0);
    let b = t.0.join("p.soniclaw");
    pack_bundle(&s, &t.0, &b).unwrap();
    let destination = t.0.join("import");
    let mut loader = AsyncProjectLoader::new(descriptor());
    loader
        .submit(
            LoadRequest::Bundle {
                path: b,
                destination: destination.clone(),
            },
            Some(spec()),
        )
        .unwrap();
    let loaded = poll(&mut loader).result.unwrap();
    assert_eq!(loaded.root(), destination.canonicalize().unwrap());
    assert_eq!(loaded.state().assets[0].path, "assets/0.wav");
    assert!(loaded.root().join("project.json").is_file());
}
