# Studio Touch/Latch automation recording — 0.69.0

## Workflow

Play Reference Studio, expand Automation write, select a live session parameter, choose Touch or Latch and a beat grid, then Arm lane and restart writing. Arming stops, captures current state, creates/enables the target lane with its current normalized value and prepares a fresh session with the correct Gesture mode. Drag the matching top-level session fader, or use its supported keyboard/wheel edits. Touch writes while held and returns control to existing lane playback on release. Latch writes the last touched value after release until Finish automation and stop/Stop. Discard pending write clears live gesture override and stops without publishing pending points. Arming failure restores the pre-arm project.

A pending take edits an isolated automation document. Committed points are published on Stop, enter global project history as one automation edit and are used on the next Play. Arming itself is also an undoable lane/mode edit. Undo/redo is disabled while writing; finish/discard first. The point editor is disabled during a write. Arrangement/routing changes made elsewhere invalidate a pending take instead of overwriting newer structure.

## API and example

`studio::automation_write::arm(project, parameter_id, normalized_initial, mode)` is stopped/control-thread-only. The caller obtains a real target and its normalized initial value from a prepared controller. Arming does not invent a parameter or bypass the compiler's target validation. Prepare/publish the armed document, then create `Writer::new(project, parameter_id, grid)` and use its `lane_index` to address the corresponding live `Controller::gestures` handle.

Call `Writer::input(beat, normalized_value, held)` and live `Gesture::set(value)` on a control edit. Input writes immediately so a single keyboard/wheel edit is captured. `end`/`Gesture::end` releases Touch; Latch keeps writing. Tick with caller-supplied musical beats while transport advances. `Writer::commit(&mut project)` verifies project identity and unchanged structure/lanes (live parameter-value snapshots are allowed), stops its recorder and transactionally publishes the pending automation. `pending`, `parameter`, `lane_index` and `point_count` support inspection. A failed commit retains the Writer and leaves the project unchanged; callers can inspect/export pending points or discard it. The studio retains a stale take until explicit discard rather than silently replacing newer lanes.

```sh
cargo run -p soniclaw-studio-automation-write -- NEW_DIRECTORY
```

The example checks a real prepared Gesture changes the session parameter, Touch stops after release, Latch continues, grid snapping, committed undo/redo and portable reopening. Unit tests additionally reject stale structure while allowing parameter snapshot changes. This exercises the processor/control bridge with virtual blocks; physical GUI/hardware recording was not run.

## Timing and playback

The library accepts exact musical beats from its caller. Reference UI uses the most recently published callback playhead and the project's TempoMap at each roughly 30 Hz UI frame. This is **GUI/control-rate automation capture**, not sample-accurate host gesture timestamping. A 1/16-beat default grid limits redundant points. Zero grid records unsnapped caller beats and can exhaust capacity quickly. Repeated writes at the same snapped beat replace the point; looping/rewinding overwrites matching beat positions, retaining other points. There is no multi-pass automation comp or automatic thinning.

Live Gesture handles preserve audible Touch/Latch override against the prepared lane while recording; rebuilding the whole processor per fader motion is unnecessary. Pending lane edits are not incrementally installed in the running processor. On stop/restart, committed lanes play through the existing prepared automation engine. Only parameters exposed through the studio Controller are selectable; native CLAP generic sliders currently use their separate host owner API and are not bridged into this writer.

## Ownership, persistence, bounds and recovery

Writer/arm/tick/commit allocate and validate on the control thread; the audio callback only uses existing atomic Gesture and parameter handles. The bounded automation document supports 64 lanes and 4,096 total points; normalized values are finite 0..1, beats 0..1,000,000, write grid 0..16 beats. Invalid modes, values, IDs, missing armed lanes, capacity, stale structure and failed preparation are explicit errors. Existing points are replaced/inserted by the underlying transactional Recorder, not erased wholesale in untouched ranges.

Committed lanes already use the schema-1 studio automation representation; no project schema migration. Pending Writer state is transient and not included in portable saves/recovery until committed; a crash can lose the pending take. Save/normal Stop attempts commit before serializing. A stale write causes Stop to complete device/native teardown while retaining the pending Writer and reporting the error; Discard permits further work. Native capture errors remain explicit. This release does not add a crash journal for pending gestures, hardware MIDI learn, native plug-in parameter automation or sample-accurate GUI events. See the validation report for exact executed evidence.
