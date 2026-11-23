# Custom DSP registry — v0.9.0

The `soniclaw-registry` crate connects compiled Rust DSP to project nodes, graph routing, parameter automation/modulation, menus and optional egui editors. The facade exports it as `soniclaw::registry`.

A separately compiled example pack lives in `examples/processor-pack` (`soniclaw-example-processors`). Its Tape Colour effect is registered in the workbench, appears under **+ Registered processor**, and has a seventh preset. Its custom editor appears on Controls; an unregistered editor falls back to the workbench's standard parameter controls.

## Application setup

```rust,ignore
use std::sync::Arc;
use soniclaw::registry::ProcessorRegistry;

let mut registry = ProcessorRegistry::default();
soniclaw_example_processors::register(&mut registry)?;
let node = registry.node("tape", "org.soniclaw.example.tape")?;
// Add node to ProjectState, set input/edges/output, then build with this registry.
let plugin = soniclaw::project::create_plugin_with_registry(
    descriptor, project, asset_root, Arc::new(registry),
)?;
```

The example is a local workspace crate; dependencies are not published by this release. Include processor-pack as a path dependency when reproducing the snippet. In a real product, replace its provisional namespace and provide your own descriptor.

## Registration contract

`Registration::new(metadata, parameter_specs, factory)` defines a processor. Metadata includes a stable namespaced ID, display name, category, configuration version and instrument flag. `ParameterSpec` defines a local key, label, range, default and unit. Global parameter IDs are `node_id.key`.

The factory receives a `BuildContext` containing the node, project, scoped asset root and registry-created `Arc<Parameter>` controls. Clone those exact controls into your DSP. A constructor must not change reused controls or perform audio processing; the project factory validates the entire candidate before publishing values. Use `parameter.modulated()` in processing when supporting modulation. Scalar sample-accurate automation uses the existing Engine bindings. Block-size-independent smoothing and realtime behavior are the processor author's responsibility.

Build or migrate only on a control/resource worker. `prepare` must allocate all callback storage. Processing must not query the registry, access files, lock, allocate or destroy control resources. The registry itself performs no callback lookup. Registered Rust code runs inside the application and is trusted; no crash or resource sandbox is provided.

A registry accepts at most 256 definitions and 32 scalar parameters per definition; project parameters are bounded at 4,096. Processor IDs are namespaced ASCII tokens separated by dots, with a total maximum of 128 bytes. Custom node IDs and local parameter keys are 1–64 ASCII letters/digits/underscore/hyphen. Parameter metadata strings are owned by core Parameter objects; repeated loads do not leak identifier strings. Duplicate registrations, bad ranges, unknown settings and incompatible reused host parameter metadata fail with errors.

Freeze a completed registry in `Arc<ProcessorRegistry>` and retain it for the application's lifetime. The workbench uses one shared registration set for editor construction, previews, background loading, live graph replacement and host-state restore. `ProjectSession::with_registry` retains registrations across build/edit/undo/redo; its `new` constructor uses an empty registry. Applications must use the same registry in every construction path; plain `create_plugin` deliberately has an empty custom registry and reports missing registrations.

## Saved state and migration

Project schema remains 2. A registered node adds `kind: "custom"` and an optional `custom` field:

```json
{
  "id": "tape",
  "kind": "custom",
  "settings": {"drive": 4.0, "mix": 0.75},
  "input_gain": 1.0,
  "custom": {
    "processor_id": "org.soniclaw.example.tape",
    "version": 2,
    "data": {"asymmetry": 0.1}
  }
}
```

This is a node fragment, not a full runnable project. Structural `data` is bounded to 64 KiB per custom node and the existing 4 MiB project JSON bound still applies. It records configuration, not voices/filter history. Asset-bearing processors should use project asset IDs and `ProjectState::resolve_asset`; opaque paths inside `data` are not discovered or copied by the bundle system.

`with_default_state(value)` supplies a fresh node's configuration; its default is JSON null. `with_migration` converts an older version's data directly to the current version. Tape Colour demonstrates version 1 `bias` becoming version 2 `asymmetry`. Future versions, missing migrations or missing registrations are rejected. Normalization produces a new document; successful load/save records the current version. A failed restore leaves the old project and controls intact for the example's well-behaved factory. Migration code is trusted application code and must not mutate existing parameter objects or external state.

Parameter renames, removals and range changes require explicit product migration/compatibility planning; the generic data migrator does not rewrite parameter IDs. Deactivated host restore keeps the exported parameter set and exact Arc identities stable. Standalone editing can build a new set while stopped or prepare a replacement graph. Bundles and autosave retain the custom state but do not contain executable Rust code: the receiving application must include/register the processor pack.

## Optional editors

Enable registry `ui` (also enabled by facade `ui`) to use `with_editor`. Its callback receives egui UI plus this node's parameters and returns `true` when an edit gesture should be recorded in document history. The example returns true on drag completion or keyboard/value edits. Call `show_editor`: `Some(commit)` means it drew a custom editor, while `None` requests generic controls. Editor registration is optional; it is not a third-party CLAP editor attachment mechanism.

## Run the example

```sh
cargo run -p soniclaw-workbench -- --preset 6
cargo run -p soniclaw-sdk-tools -- --registry-demo /new/registry-demo
```

The walkthrough renders dry/processed WAV files, migrates old state, verifies host parameter handle reuse and packs/unpacks/reconstructs a portable registered project. Existing CLAP adapters can export products built with `create_plugin_with_registry`; the four workbench CLAP product identities remain unchanged in this release. The Tape preset is a standalone registry demonstration, not an additional exported product.
