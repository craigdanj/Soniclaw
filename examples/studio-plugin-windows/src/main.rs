use soniclaw_host::{
    editor_workspace::{Mode, Workspace},
    live::RackEntry,
};
fn main() -> Result<(), soniclaw_host::HostError> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: studio-plugin-windows NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&root)?;
    let entry = RackEntry {
        path: "/example/trusted/synth.clap".into(),
        plugin_id: "org.example.synth".into(),
    };
    let mut workspace = Workspace::default();
    let state = workspace.for_instance("lead.synth", &entry)?;
    state.mode = Mode::Owned;
    state.scale = 1.5;
    state.size = Some([800, 600]);
    state.reopen = true;
    let text = workspace.to_json()?;
    std::fs::write(root.join("plugin-windows.json"), &text)?;
    let mut restored = Workspace::from_json(&text)?;
    assert!(restored.for_instance("lead.synth", &entry)?.reopen);
    let changed = RackEntry {
        path: entry.path.clone(),
        plugin_id: "another-product".into(),
    };
    assert!(!restored.for_instance("lead.synth", &changed)?.reopen);
    restored.windows.get_mut("lead.synth").unwrap().scale = 99.;
    assert!(restored.to_json().is_err());
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"preferences_roundtrip":true,"identity_change_resets_reopen":true,"invalid_scale_rejected":true,"modules_loaded":0,"native_window_runtime_tested":false}),
        )?,
    )?;
    Ok(())
}
