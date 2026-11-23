# Automation editing toolkit (0.13)

Soniclaw now provides `soniclaw::automation` / `soniclaw-automation`: serializable beat lanes, an egui editor, bounded gesture recording and prepared playback. This is application automation; exported CLAP's existing incoming parameter-event API remains available separately.

## Workbench walkthrough

1. Start `cargo run -p soniclaw-workbench` and choose a preset.
2. Open **Automation**, then **Add lane**. Select a stable rack parameter ID. Values in the editor are normalized 0–1; playback converts to that parameter's physical range.
3. Double-click the graph to insert points. Set exact beat/value positions in the rows, delete with ×, and choose an outgoing Step, Linear, Smooth or Bezier curve. Bezier controls are normalized **Y** controls with X fixed at one-third and two-thirds. Smooth is cubic smoothstep, not a spline passing through neighboring tangents.
4. Set Grid in beats (0 disables snap) and View length. **Loop view** sets the document loop to `[0, View]` when toggled; change it again after changing View. The API supports arbitrary loop bounds.
5. **Apply automation** validates and rebuilds the project using the existing prepared graph publication path. **Play** uses live device timing; **Preview** uses the offline renderer. Parameter smoothing inside a processor can soften sample-accurate target changes.
6. To record, select Touch or Latch, enable **Record draft**, start audio and drag **Record value**. Touch writes while held. Latch continues writing the last value until **Stop gesture / clear latch**, disabling recording or changing mode. The cursor comes from audio telemetry and the current tempo; GUI recording is coarse and is not a sample-accurate capture of mouse timing. Changes remain a draft until Apply. The record slider does not monitor the gesture into the already-running graph.
7. Apply, then Save, bundle, autosave or save to the preset catalog. Lanes belong to the project document. Reload saved lanes discards the current draft; Undo restores applied project edits. Reload after changing presets or Undo to synchronize a draft with the current model.

The graph currently edits points with exact numeric rows rather than dragging them. Duplicate beats or targets make an invalid draft; fix/delete them before Apply. The editor preserves the last applied runnable graph on a build failure.

## Runnable demonstration

```sh
cargo run -p soniclaw-sdk-tools -- --automation-demo /tmp/new-automation-demo
cargo test -p soniclaw-automation --all-features
cargo test -p soniclaw --test v013
```

The demonstration creates a gain project, records two Latch points, writes project JSON and a 96,000-frame sine fade WAV using 257-frame blocks. The tests independently verify curve evaluation, block partitioning, loop wrap, seeks, tempo changes, mode behavior, transaction failures, project persistence and callback allocations.

## Public document API

```rust
use soniclaw::automation::{Curve, Document, Lane, Mode, Point, Recorder};
let mut doc = Document {
    loop_range: Some([0.0, 4.0]),
    lanes: vec![Lane {
        parameter: "gain.gain".into(), // use the actual ID from plugin.parameters()
        enabled: true,
        mode: Mode::Touch,
        points: vec![Point { beat: 0.0, value: 1.0, curve: Curve::Smooth }],
    }],
};
doc.write(0, 1.13, 0.25, 0.25)?; // snapped to 1.25 beats
let mut recorder = Recorder::default();
recorder.begin(0.7)?;
recorder.tick(&mut doc, 0, 2.0, 0.25)?;
recorder.end();
recorder.stop();
# Ok::<(), soniclaw::core::Error>(())
```

`Document::write` checks the existing document and insertion before mutation, replaces an existing snapped beat, and inserts otherwise. `Lane::value_at` holds the first/last values outside the point range, returns None for disabled/empty lanes, and uses the left point's outgoing curve. `Document::beat` applies the optional loop after its start. Call `validate` before using manually edited public fields. Malformed direct construction is not a supported input to raw `value_at`; prepared playback always validates first.

`Recorder` is control-thread state, not an audio callback recorder. Call `begin(value)` for every changed gesture value, `end()` on release, `tick` with explicit transport beats, and `stop()` on transport stop or lane change. Read records nothing. Touch records held values; Latch continues after release. A tick returns whether it wrote. Capacity errors preserve the document. `snap` rejects invalid beat/grid values.

## Prepared playback and gesture monitoring

`AutomatedProcessor::new(inner, document, parameters)` validates all lanes and resolves each stable string to an `Arc<Parameter>` before returning `(processor, gestures)`. Retain the ordered `Vec<Gesture>` if your application wants monitored gestures. Prepare the processor, then process using a playing `ProcessContext`. It implements `AudioProcessor` and can be inserted into an Engine or Graph. The facade's project builder automatically wraps project graphs when `ProjectState.automation` is Some; its convenience builder does not expose gesture handles. Use the wrapper directly when monitoring is needed.

`Gesture::set(normalized)` starts/updates an override in Touch/Latch; Read ignores overrides. `end()` restores lane playback in Touch and keeps Latch active. `clear()`, processor reset, or a stopped process context clears it. Feed the control-thread Recorder separately if the gesture should be persisted. Gestures use atomic values and flags; they never edit a document or allocate on the callback.

The wrapper evaluates each active lane per sample and invokes the inner processor on one-frame slices, preserving MIDI frame offsets. Target values are linear in the physical range; logarithmic knob mapping is an application concern. Existing processor smoothing/modulation still applies. Project lanes set base parameters before inner DSP each sample, so they take precedence over external parameter stores while playing; do not concurrently automate the same target through CLAP and a project lane. Disable/remove that lane when a host should own it.

## Timing and thread ownership

Documents, editing, serialization, target resolution, Recorder and preparation belong off the callback. The prepared processor owns its immutable document, target Arcs, scratch MIDI buffer and inner DSP. Only process/reset run on its audio owner. Allocate a replacement off-thread and publish through `graph::live` when changing lanes. The callback does no locks, file access, heap allocation or ID lookup. Cost is bounded by at most 64 lanes × frames plus inner processing; per-sample invocation can be expensive for spectral or block-oriented processors. Measure your application before enabling many lanes. No universal callback deadline guarantee is made.

Contiguous blocks preserve accumulated beat phase when tempo changes. A seek/discontinuity re-anchors from absolute sample position at the new current tempo. This is not a DAW tempo map or incoming PPQ position API. Looping wraps lane time only; it does not rewind DSP or emit MIDI. Stopping bypasses lane evaluation and clears overrides. Empty/disabled documents use the normal block processing path. MIDI slices have a maximum of 1024 events per block; invalid event, channel, frame or tempo input clears output.

## Persistence, migration and failure recovery

`ProjectState` schema 2 gains optional `automation`; absent means no timeline and old projects need no migration. New readers accept old documents. Older strict readers reject the new field; remove automation explicitly for an older target. DSP history, accumulated tempo anchors and active gesture/latch state are deliberately not serialized. Project JSON's existing 4 MiB limit still applies. Presets, bundles, undo and recovery carry the optional document via project serialization.

Limits: 64 unique lane targets; 4096 points total; target ID at most 256 bytes; finite beats from 0 to 1,000,000; strictly increasing unique point beats; normalized values and Bezier controls 0–1; finite nonnegative snap grid; ordered finite loop bounds. Missing targets fail reconstruction before graph publication. These constraints are checked even for disabled lanes. Capacity/range/target errors return `InvalidParameter`; no silently dropped points. Fix the draft, reduce points, use a known ID or reload saved lanes. Asset transactions and graph publication retain their previous recovery guarantees.

## Verification and scope

See [0.13 results](validation/v013/REPORT.md) and [release notes](releases/v0.13.0.md). Synthetic/offline checks and a native editor capture do not establish physical audio latency or acceptance in a real DAW. Windows/macOS runtime checks remain pending. Full tempo-map import, clip envelopes, spline tangent editing, monitored native record sliders and sample-accurate GUI timestamps are future extensions.
