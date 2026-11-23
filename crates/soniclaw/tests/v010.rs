use soniclaw::{catalog::*, prelude::*, state::*};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "soniclaw-browser-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn project(name: &str) -> ProjectState {
    let mut s = ProjectState::empty("test.catalog", name);
    let mut n = RackNode::new("gain", ProcessorKind::Gain);
    n.input_gain = 1.0;
    s.nodes.push(n);
    s.outputs.insert("gain".into(), 1.0);
    s
}
#[test]
fn searchable_catalog_persists_tags_banks_favourites_and_projects() {
    let t = Temp::new();
    let mut c = PresetCatalog::open(t.0.join("catalog")).unwrap();
    let id = c
        .insert(
            &project("Warm Pad"),
            &t.0,
            vec!["Ambient".into()],
            "Factory".into(),
        )
        .unwrap();
    assert_eq!(c.search("warm ambient", Some("Factory"), false).len(), 1);
    assert!(c.search("", None, true).is_empty());
    c.edit_metadata(
        &id,
        "Warm Pad".into(),
        vec!["Ambient".into()],
        "Factory".into(),
        true,
    )
    .unwrap();
    drop(c);
    let c = PresetCatalog::open(t.0.join("catalog")).unwrap();
    assert_eq!(c.search("PAD", None, true).len(), 1);
    assert_eq!(c.project(&id).unwrap().0, project("Warm Pad"));
}
#[test]
fn corrupt_latest_index_recovers_previous_and_never_overwrites_corruption() {
    let t = Temp::new();
    let root = t.0.join("c");
    let mut c = PresetCatalog::open(&root).unwrap();
    let id = c
        .insert(&project("First"), &t.0, vec![], "User".into())
        .unwrap();
    c.edit_metadata(&id, "Second".into(), vec![], "User".into(), false)
        .unwrap();
    drop(c);
    let mut files: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let latest = files.last().unwrap();
    std::fs::write(latest, b"corrupt").unwrap();
    let mut c = PresetCatalog::open(&root).unwrap();
    assert_eq!(c.entry(&id).unwrap().name, "First");
    c.edit_metadata(&id, "Third".into(), vec![], "User".into(), true)
        .unwrap();
    assert_eq!(std::fs::read(latest).unwrap(), b"corrupt");
    assert_eq!(
        PresetCatalog::open(&root).unwrap().entry(&id).unwrap().name,
        "Third"
    );
}
#[test]
fn relocation_is_scoped_and_transactional() {
    let t = Temp::new();
    let original = t.0.join("original");
    let new = t.0.join("new");
    std::fs::create_dir(&original).unwrap();
    std::fs::create_dir(&new).unwrap();
    soniclaw::io::write_wav(original.join("kick.wav"), 48000, 1, &[0.2; 64]).unwrap();
    let mut s = project("Drums");
    s.assets.push(Asset {
        id: "kick".into(),
        path: "kick.wav".into(),
        root_note: 36,
    });
    let mut c = PresetCatalog::open(t.0.join("c")).unwrap();
    let id = c.insert(&s, &original, vec![], "Drums".into()).unwrap();
    std::fs::rename(original.join("kick.wav"), new.join("new-kick.wav")).unwrap();
    assert_eq!(c.missing_assets(&id).unwrap().len(), 1);
    let before = c.project(&id).unwrap();
    assert!(c
        .relocate(&id, &new, &[("kick".into(), "../escape.wav".into())].into())
        .is_err());
    assert_eq!(c.project(&id).unwrap(), before);
    c.relocate(&id, &new, &[("kick".into(), "new-kick.wav".into())].into())
        .unwrap();
    assert!(c.missing_assets(&id).unwrap().is_empty());
    c.export_preset(&id, &t.0.join("drums.soniclaw")).unwrap();
    let restored = soniclaw::project_storage::unpack_bundle(
        &t.0.join("drums.soniclaw"),
        &t.0.join("unpacked"),
    )
    .unwrap();
    assert_eq!(restored.assets.len(), 1);
}
#[test]
fn invalid_metadata_and_corrupt_document_do_not_change_catalog() {
    let t = Temp::new();
    let mut c = PresetCatalog::open(t.0.join("c")).unwrap();
    let id = c
        .insert(&project("A"), &t.0, vec![], "User".into())
        .unwrap();
    let before = c.entries().to_vec();
    assert!(c
        .edit_metadata(&id, "".into(), vec![], "User".into(), false)
        .is_err());
    assert_eq!(c.entries(), before);
    let path = c.root().join(&c.entry(&id).unwrap().project_file);
    std::fs::write(path, b"{}").unwrap();
    assert!(c.project(&id).is_err());
    c.remove(&id).unwrap();
    assert!(c.entries().is_empty());
}
#[test]
fn preview_resamples_and_stops_without_callback_allocations() {
    let audio = Arc::new(soniclaw::io::AudioFile {
        sample_rate: 24000,
        channels: 1,
        samples: vec![0.0, 1.0, 0.0, -1.0],
    });
    let mut p = PreviewPlayer::new(audio).unwrap();
    p.prepare(ProcessSpec {
        sample_rate: 48000.0,
        max_block_frames: 16,
        channels: 2,
    })
    .unwrap();
    let mut data = [0.0; 32];
    let a = allocation_counter::measure(|| {
        p.process(
            &mut AudioBuffer::new(&mut data, 2).unwrap(),
            &ProcessContext {
                events: &[],
                transport: Default::default(),
            },
        )
    });
    assert_eq!(a.count_total, 0);
    assert_eq!(data[2], 0.5);
    assert_eq!(data[4], 1.0);
    assert!(data[16..].iter().all(|x| *x == 0.0));
    p.reset();
}
