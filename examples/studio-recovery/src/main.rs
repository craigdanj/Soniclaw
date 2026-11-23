use soniclaw::studio::{
    project::{Project, ProjectError},
    recovery::{Autosave, Limits, Store},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn wait(auto: &mut Autosave) -> Result<u64, ProjectError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = auto.poll() {
            return Ok(result?.snapshot.generation);
        }
        if Instant::now() > deadline {
            return Err("autosave timed out".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn main() -> Result<(), ProjectError> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-recovery NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let (d, a) = soniclaw_studio_demo::fixture();
    let initial = Project::new("Recovery example".into(), d, a, Default::default())?;
    let legacy = root.join("legacy-project");
    initial.save_new(&legacy)?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(legacy.join("project.json"))?)?;
    json["schema"] = serde_json::json!(1);
    json.as_object_mut().unwrap().remove("project_id");
    std::fs::write(
        legacy.join("project.json"),
        serde_json::to_vec_pretty(&json)?,
    )?;
    std::fs::copy(legacy.join("assets/0.wav"), root.join("exact-original.wav"))?;
    std::fs::remove_file(legacy.join("assets/0.wav"))?;
    let issues = Project::resource_issues(&legacy)?;
    assert_eq!(issues.len(), 1);
    Project::repair_missing(
        &legacy,
        &issues[0].id,
        issues[0].kind,
        &root.join("exact-original.wav"),
    )?;
    let loaded = Project::open_with_report(&legacy)?;
    assert_eq!(loaded.source_schema, 1);
    assert_eq!(loaded.migrations.len(), 1);
    let mut p = loaded.project;
    p.save_new(&root.join("upgraded-project"))?;
    let store = Store::open(
        &root.join("recovery"),
        &p.project_id,
        Limits {
            snapshots: 2,
            retained_bytes: 32 * 1024 * 1024,
        },
    )?;
    let mut auto = Autosave::new(store.clone(), Duration::from_secs(30))?;
    assert!(auto.checkpoint_now(Duration::ZERO, &p)?);
    let first = wait(&mut auto)?;
    p.name = "Newer checkpoint".into();
    assert!(auto.checkpoint_now(Duration::from_secs(1), &p)?);
    let second = wait(&mut auto)?;
    let snapshots = store.inventory()?.snapshots;
    std::fs::write(
        snapshots[0].directory.join("assets/0.wav"),
        b"corrupt newest snapshot",
    )?;
    std::fs::create_dir(root.join("recovery/snapshot-00000000000000000003"))?;
    let restored = store.restore_latest(&root.join("working-project"))?;
    assert_eq!(restored.snapshot.generation, first);
    assert_eq!(restored.project.name, "Recovery example");
    let warnings = restored.warnings.len();
    assert!(warnings > 0);
    let saved = store.checkpoint(&restored.project)?;
    assert_eq!(saved.snapshot.generation, 4);
    assert!(!snapshots[1].directory.exists());
    assert_eq!(
        Project::open(&root.join("working-project"))?.name,
        "Recovery example"
    );
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"schema_migrated_from":1,"schema_saved_as":3,"missing_media_repaired_with_exact_hash":true,"background_generations":[first,second],"restored_generation":restored.snapshot.generation,"recovery_warnings":warnings,"incomplete_generation_skipped":3,"working_copy_survived_retention":true,"native_code_loaded":false,"physical_device_tested":false}),
        )?,
    )?;
    Ok(())
}
