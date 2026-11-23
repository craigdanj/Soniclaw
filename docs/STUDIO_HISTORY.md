# Studio-wide undo and redo (0.60)

The unified studio now has one project history above its clip, mixer, automation and comp editors. Transactions can change several parts of a project together, coalesce a gesture and restore the whole candidate when validation or processor preparation fails. Immutable sample buffers are shared; recording files and prepared DSP histories are not copied or deleted by Undo.

## Run and interact

```sh
cargo run -p soniclaw-studio-history -- NEW_DIRECTORY
cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT
```

The software example groups a bass-region move and mixer gain change into one undo entry, restores the original document through real processor preparation, redoes both changes and writes a portable project plus `report.json`. It opens no device or native window.

The reference studio has **Undo project edit** and **Redo project edit**, retained-history information and Command/Ctrl+Z, Command/Ctrl+Shift+Z or Command/Ctrl+Y shortcuts when a text field is not consuming input. A pointer gesture begins a group only after a real project change, so pressing the undo buttons does not disable them by creating an empty group. The arrangement's local undo buttons are hidden in this application. Its reusable editor keeps its original local history by default for other applications. Unapplied comp drafts still have their own local draft history; applying the recipe is a project transaction.

The reference app stops playback before project undo/redo; resume with Play after the change. Finish an active take before undoing. A failed UI history commit stops the runtime rather than leaving newly changed shared parameters playing against a rolled-back project. Structural application actions, completed freezes, applied comp recipes and native checkpoints captured during stopped lifecycle operations enter project history. Opaque native state is captured by the existing stopped host-session workflow, not by sampling plugin DSP memory on every mouse movement. Direct edits in an external native plug-in editor require the explicit native-owner checkpoint lifecycle.

## Public API

`soniclaw::studio::history::History` owns the undo/redo branches. `Limits { entries, retained_bytes }` bounds storage. `edit(project, label, closure)` clones a candidate, runs the closure and validates the complete project before adopting it. An error leaves the original project and history intact. No-op edits retain the redo branch. Starting a new real edit after Undo clears the old redo branch.

`edit_prepared` additionally calls a preparation closure against the validated candidate and returns its owned prepared result only after committing. `undo_prepared` and `redo_prepared` provide the corresponding restore boundary. A preparation error leaves project/history unchanged. Preparation should construct an owned candidate, without replacing an existing live runtime or mutating external native owners as a side effect. The application remains responsible for publishing and retiring the resulting processor at its supported safe boundary. Non-prepared convenience methods are appropriate for stopped document editing.

For editors that already construct validated candidates, `record(previous, current, label)` records their transition. On a fallible error it restores previous. Resource identity is significant: replacing a PCM Arc with another allocation counts as a resource change even when its samples happen to have equal values. `equivalent(a,b)` compares metadata and asset identities for control-side change detection; it does not validate a project.

`begin_group(project,label)` retains an original snapshot. Multiple `edit`/`record` calls then modify current without adding separate entries. `finish_group` validates and commits one original snapshot, or restores it on error. `cancel_group` restores the original without changing the undo/redo branches. Nested groups and undo/redo during a group reject. A no-op completed group creates no entry. `clear` requires no active group and should be called after opening a different project. `undo_label`, `redo_label` and `stats` supply reusable application controls.

## Resource bounds and ownership

All history APIs belong on a control/UI thread. They allocate, serialize metadata, validate resource references and may perform filesystem reads. They do not implement AudioProcessor and must never run in an audio callback. Dropping an evicted snapshot can release the last reference to a large PCM allocation, so eviction and clear also belong off callback.

Limits allow 1–256 entries and a retained-byte estimate from 1 KiB to 1 GiB. Defaults are 32 entries and 256 MiB. Each metadata serialization is bounded during writing to 16 MiB. Retained accounting counts each unique shared PCM allocation once across undo, redo and an active group, plus an eight-times-serialized-metadata estimate, labels and per-resource/container allowances. This is an intentionally conservative budgeting policy, not an allocator/RSS measurement. It excludes the active application's independently owned project, prepared DSP and external files. Temporary candidate/serialization storage can add control-thread memory beyond retained history.

Accepted new edits evict oldest undo entries to satisfy count/byte limits. One oversized snapshot rejects the edit. Group creation must fit alongside retained history; undo/redo can also reject if retaining the current state would exceed the budget. None silently discards the project being restored. The immutable source-file requirement remains: a missing or modified file can invalidate an old snapshot. Repair/reload the resource before restoring it; history does not recreate deleted recordings.

## Persistence and migration

`Project` becomes Clone so snapshots share immutable assets while copying metadata. Project JSON schema, resource IDs and product IDs are unchanged in this release. History is process-local and is not serialized as project content. Save the applied project to persist the chosen state. Files created by freeze, recording or export remain available after Undo, even if the restored project no longer refers to them; resource retirement is a separate application operation. Runtime voices, processing delay lines, transport and device state are not reversible project data.

Unit tests cover grouped cross-domain restoration, no-op edits, branching, cancellation, invalid-editor rollback, entry/byte limits, shared PCM identities and failed processor preparation. The reference-studio headless test exercises its real global undo/redo action boundary. See the [executed release checks](validation/v0_60_0/REPORT.md). Physical devices, third-party DAWs and native-window acceptance are separate checks.
