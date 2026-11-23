# Example registered processor pack

An ordinary Rust library crate that registers Tape Colour with Soniclaw. It has no dependency on the facade or workbench and can be reused by other applications.

```rust,ignore
let mut registry = soniclaw_registry::ProcessorRegistry::default();
soniclaw_example_processors::register(&mut registry)?;
let node = registry.node("tape", soniclaw_example_processors::TAPE_ID)?;
```

The pack declares four scalar controls (drive, tone, mix, output), a versioned structural asymmetry setting, a version 1-to-2 migration and an optional egui editor. DSP uses the parameter handles supplied by the registry. This simple saturation/low-pass example is not a physically calibrated tape model or an oversampled mastering processor.

Run `cargo run -p soniclaw-workbench -- --preset 6` or `cargo run -p soniclaw-sdk-tools -- --registry-demo /new/folder` from the workspace root. Enable `ui` when using its custom editor. See [the registry guide](../../docs/REGISTRY.md) for lifecycle, assets, bounds and product compatibility.
