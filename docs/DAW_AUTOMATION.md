# Native plug-in and mixer automation — 0.84.0

Native CLAP/VST3 controls now participate in Studio's existing saved parameter values, beat lanes, Touch/Latch write takes, live faders and history. Auxiliary return levels join the existing track, send, bus, master and cue controls.

## Using it

Install a trusted native product through the plug-in manager. **Automation IDs** accepts comma-separated writable native IDs; blank selects all writable, non-fixed-range controls, with a maximum of 64 per instance. Larger products require an explicit selection. Read-only, missing, duplicate and out-of-range IDs are rejected during preparation. Changing the selection requires a stopped graph rebuild. Keep existing IDs selected while saved lanes or parameter values reference them, or remove those references deliberately.

Press Play. Native controls appear in the live fader list as `INSTANCE.native.NUMERIC_ID`. Choose one in **Automation write**, choose Touch or Latch, arm and restart, and use its live fader. Stop commits the bounded GUI-clock performance to the project. Read lanes use the existing per-sample beat evaluator. Native inspector values are informational for selected controls; use the live fader for host automation. Unselected writable controls remain manually adjustable through the native inspector. Native editor feedback is displayed but does not automatically create host automation gestures or write takes.

Auxiliary controls use `aux.INDEX.return_gain`, physical range -4..4 (including phase inversion). Lane values are normalized: 0 is -4, 0.5 is zero gain (silence), 0.625 is +1. Track/bus/master/cue controls retain their documented IDs and ranges. Reordering auxiliary returns changes index identity; keep that order stable while using saved lanes.

## API and real-time behavior

`soniclaw_host::studio::automation::{Control,controls,NativeProcessor,Automated}` adapts native metadata to atomic `Arc<Parameter>` handles. `NativeFactory` returns those handles in `Built.parameters`. `Control::native_value` maps normalized 0..1 to CLAP's native range; VST3 values remain normalized. Stable IDs use the native numeric identity, independent of names and parameter enumeration order. All selection, metadata checks, buffers and handles are prepared on the owner thread.

`DirectProcessor::process_automated` accepts at most 64 sorted native-range events for CLAP node zero. It merges with the bounded manual command queue; combined overflow stops the rack and clears output. Studio submits changed values at offset zero of its per-sample sub-blocks, preserving lane timing without calling the control owner from the audio callback. State resources, native editor operations, polling, loading and destruction remain on the owning thread.

VST3 uses the musical bridge's timed normalized points. Packet storage is increased to 1024 changes per fixed 256-frame packet, below the backend's 4096 pending-change bound. One linear lane can produce 256 points per packet; many dense lanes can exceed the limit. Overflow is an explicit processing failure; reduce automation density/selected moving controls and restart. The live worker bridge still adds 1024 frames plus native latency, included in graph compensation. This release does not remove worker scheduling or CPU limits.

`Owner.controls` exposes the shared host controls; `Owner.automation_failures` counts adapter failures. `Owner::poll` reports adapter and VST3 worker failures for restart. Host scalar updates are atomic and independent, not a coherent multi-parameter transaction. Read-only controls are excluded. Metadata changes require rebuilding rather than changing active mappings. Callback paths use prepared arrays/queues and no owner-thread filesystem or UI work.

## Persistence and migration

Native selection lives in descriptor `state.automation_ids`; absent selection preserves small-product compatibility by selecting all writable controls. Native normalized values use the existing `Document.parameter_values` and automation document. Version 0.83 native blobs and legacy CLAP sessions remain readable. Snapshots restore opaque state then parameters, and include the latest selected host values, even if Stop happened before the next callback consumed them. This guarantees the requested host value is retained, not that the plug-in had already rendered it.

Undo/redo and generation saves use existing project operations. Duplicate track processor IDs remap native control lane prefixes and blob keys. Frozen originals retain their automation. Bypass instances expose metadata-free normalized placeholder handles for selected IDs (or writable IDs saved in a small-product snapshot), so existing lanes and saved values remain resolvable. These handles never affect bypass audio and do not imply native metadata discovery. Repair reconstructs the actual ranges from the trusted product.

## Example and validation

Build the owned protocol fixture with `python3 tools/build-formats.py --fetch --no-editor --protocol-fixture`, then run:

```sh
cargo run -p soniclaw-daw-automation -- NEW_FOLDER build/formats/fixtures/HostProtocolFixture.vst3
```

The example uses only fixture gain ID 9 (never its fault-injection parameter), checks the step at source sample 64/output sample 1088, compares different callback partitions, counts Rust callback allocations, and verifies an immediate-stop host target survives snapshot. It accepts an explicitly trusted protocol fixture, not arbitrary products with assumed IDs.

[Executed checks](validation/v0_84_0/REPORT.md) include metadata selection/range tests and existing automation/Studio tests. Physical controllers, native editor gesture capture, broad third-party products and Windows/macOS acceptance remain pending. GUI write timing remains GUI-clock sampled; sample-accurate playback does not imply sample-accurate GUI capture.
