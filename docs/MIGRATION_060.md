# Migrating from Soniclaw 0.5 to 0.6

Update all local Soniclaw crate versions together to `0.6.0`. Rust 1.100 remains
the minimum. No rename is required. Existing plug-in product/parameter IDs remain
stable; Workbench FX/Granular are separate new products.

- `RackNode` has `asset: Option<String>`; add `asset: None` to existing Rust struct
  literals, or use `RackNode::new(id, kind)`. Granular/convolution nodes require an
  asset ID and validated scoped WAV resource. `partition`/`latency_frames` are
  structural convolution settings; `mix` is an automatable named control.
- `ProjectState` has optional `modulation`; add `modulation: None` to Rust literals
  or use `ProjectState::empty`. Handle new `ProcessorKind`/`EffectKind` variants
  when matching exhaustively.
- Project schema stays 2: old JSON omitting the additive fields still reads,
  optional empty fields are omitted, and v1 scalar migration remains. Older
  libraries with strict deserializers cannot load new nodes/modulation fields.
- `get()`/`normalized()` retain base values. Audio integrations implementing their
  own parameter reads should call `modulated()` if they want matrix offsets.
  Do not save `modulated()` into presets. A single audio owner writes modulation.
- Host structural reconstruction still requires deactivation and an unchanged
  parameter identity set. Change macro counts/node parameter layouts by making a
  new standalone project/product, not by mutating a running plug-in's parameter
  registry. Invalid reconstruction leaves current base state intact.
- `soniclaw-state` now depends on DSP for the shared serializable modulation
  config; DSP additionally uses serde and the already-used rtrb queue version.
  Workbench optionally adds PNG capture via image. Native features remain opt-in
  for core crates; Workbench enables GUI by default, with headless render available.

Read [module bounds](DSP_MODULATION.md) before using the new resource and
real-time APIs and [release notes](releases/v0.6.0.md) for validation limits.
