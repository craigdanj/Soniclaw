# Migrating 0.6 → 0.7

Update Soniclaw crates together to 0.7.0; Rust 1.100 remains required.
Existing product/parameter IDs are unchanged. Workbench appends separate
`org.soniclaw.workbench.polymod` and `.sampler` products after its original two.

- `ProjectState` adds optional `midi` and default-empty `layout`.
- `RackNode` adds optional `voice_modulation`; use None in old literals or
  `RackNode::new`. Handle `ProcessorKind::PolyModSynth` in exhaustive matches.
- Saved `SampleZone` and runtime `Zone` add `start_frame:0`, `end_frame:None`,
  `loop_crossfade:0`; use constructors for defaults. Disk factories reject
  non-default regions/fades; memory samples implement them.
- Schema remains 2. Optional default fields omit serialization; old JSON and v1
  migration remain. Older strict SDKs cannot load new fields/kinds. Update Rust
  struct literals, or use `ProjectState::empty`, `SampleZone::new`, `Zone::single`.
- `ExpressiveSynth` adds optional cutoff and note-local modulation; defaults have
  empty routing. PolyModSynth has named controls, structural modulation/MPE config.
- Live graphs keep a fixed latency budget and fresh history; Workbench adds 2048
  frames on live Apply. Offline preview latency remains the unpadded graph latency.
- Optional hosting is trusted native loading with a scoped thread lease, not a
  persistent device-host/editor adapter. Development heap probes are optional.

State shares MIDI config; UI shares document types. New dependencies are pinned
libloading, serde/rtrb and optional allocation-counter. Portable features remain
separate from native audio/UI. The workbench enables hosting and permits documented
unsafe native-load entry points; DSP and portable document code keep their safety rules.
Read [SDK contracts](SDK_070.md) and [hosting](HOSTING.md).
