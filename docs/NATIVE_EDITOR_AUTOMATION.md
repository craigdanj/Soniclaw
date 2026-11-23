# Native editor automation — 0.86.0

Native editor gestures now update selected host controls, grouped undo, saved scalar/native state, and the armed Touch/Latch write take. The reusable gesture model lives in `soniclaw-studio::native_edits`; native protocol adapters live in `soniclaw-host::studio::Owner`. Reference Studio connects both to its existing live controls and automation writer.

## Application workflow

Install a trusted native product and select writable automation IDs as in the native plug-in guide. Play, open its native editor, and move a selected control. Its begin/value/end gesture updates the corresponding `INSTANCE.native.ID` host parameter. Closing the gesture produces one undo group rather than one history entry per intermediate value. To record a lane, select that parameter in Automation write, choose Touch or Latch, and arm/restart before using the native editor. Stop commits the take. Read-mode lanes continue to own playback; select Touch/Latch to override them deliberately.

Stop services pending native edits before retiring host controls and committing the write take. Native snapshots and `Document.parameter_values` then retain the reconciled value. Window closure/Stop ends remaining gestures. Native values received after the final queue drain cannot be promised to be part of that checkpoint; don't keep editing during Stop.

## Reusable API

`native_edits::{Edit,Phase,Gestures}` defines normalized control-thread events. `Edit::validate` checks the stable parameter ID, phase/value shape and finite 0..1 values. `Gestures::apply` accepts an event, Project, optional live Controller, optional automation Writer, History and observed beat. It updates scalar state and arms/ends the matching existing automation gesture. Finish the history group with `Gestures::finish` at the end of a completed application UI batch or on Stop. `active()` allows a UI to keep history grouped across native drags in another window. No native-editor API is called from the audio callback.

`Owner::accept_editor_edit` is a validated adapter ingress for custom native editors; IDs must refer to selected writable host controls. `collect_editor_edits` drains backend queues without requiring a live audio lease, and `take_editor_edits` transfers the ordered normalized events to the application. `finish_editor_gestures` emits remaining ends. `Owner::poll` performs normal backend servicing followed by collection. `Instance::drain_parameter_edits` exposes the ordered VST3 controller gesture queue directly for other applications.

VST3 beginEdit/performEdit/endEdit events are forwarded from the vendor backend's native controller handler. A value without begin/end becomes a single edit, supporting text/keyboard edits. CLAP gesture brackets and in-gesture parameter values are forwarded from the bounded native output queue. Unbracketed CLAP processor parameter feedback updates the actual-value display but does not create an editor write: it may be an echo of host automation. Native editors should send legal gesture brackets. Unselected parameters retain native opaque/parameter-state behavior; they do not gain host automation handles.

## Ownership, timing and limits

Load, poll, edit collection, editor callbacks, history, writes, snapshot and destruction belong on the owning GUI/control thread. Only prepared atomic parameter handles and bounded native processing queues cross to audio. Native edits use the observed GUI playhead beat, the same capture clock as existing faders; this is not hardware/sample-clock capture. Read playback still evaluates the saved lane at sample timing. Native editor gestures may override a pending manual host value by explicit user intent; active Read automation may supersede them at the next audio sample.

Limits: 64 selected controls and 64 simultaneous gestures; 256 queued normalized events per native owner, at most 4096 accepted events per Studio UI cycle. Native ingress overflow reports an error; service/drain the queue and stop/rebuild as needed. CLAP output drops are surfaced rather than silently accepted as complete gestures. The VST3 backend itself has a bounded controller-event buffer, so poll regularly; this integration cannot reconstruct events discarded before its drain. End without begin is ignored, repeated begin is coalesced. Read-only/unselected IDs and invalid values reject without scalar changes. History and write-take limits/staleness rules remain in force.

## Persistence and compatibility

No project schema or parameter identity changes. Project scalar values, automation lanes and verified native blobs use their existing persistence. History shares the existing Project resources. An undo of a completed edit changes the document; publish a rebuilt session through the normal application history workflow to change active DSP. This release replaces the 0.85 host-authoritative informational-only editor boundary for selected, accepted native gestures. Raw unbracketed CLAP feedback retains the boundary described above.

## Example and executed evidence

Build the owned protocol fixture, then run:

```sh
cmake --build build/formats --target soniclaw_host_protocol
cargo run -p soniclaw-native-editor -- NEW_FOLDER build/formats/fixtures/HostProtocolFixture.vst3
```

The owned fixture exports a main-thread editor seam that invokes the real VST3 component handler. The example checks native begin/value/end ordering, host value 0.8, processed peak 0.2, zero Rust callback allocations, one undo group, redo, saved Touch automation/native state and reopen, read-only rejection and bounded overflow. It doesn't open a graphical native editor; the Linux fixture's binary path is used by this example. Physical mouse/window acceptance and broad third-party/Windows/macOS runtime remain pending. [Release checks](validation/v0_86_0/REPORT.md) record executed commands and fixture results.
