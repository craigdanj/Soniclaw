# DAW project and track workspace — 0.72.0

## Workflow and example

Start `cargo run -p soniclaw-desktop-studio --features gui -- --gui` for a blank session, or pass `--new` explicitly. Existing project-folder startup and trusted native module arguments remain supported. New project opens a dialog for name/rate/block size and an explicit Create-and-replace action; save current work first. The new session starts stopped. Save as/Open use the project-folder path field. Durable in-place Save follows in the next release.

Tracks and inspector provides Add audio/instrument, rename, duplicate, delete, reorder, folder assignment and live-MIDI monitoring. New instrument tracks use the stock sine instrument initially. The last track cannot be deleted. Deletion removes model references without deleting source media. Duplicate shares immutable media, creates fresh region/processor identities and copies namespaced controls/automation; a frozen source yields an editable duplicate of its original track. Other frozen tracks and their caches are retained.

Add four-beat MIDI clip creates an empty editable region. The MIDI selector opens any clip on the selected track, including newly recorded takes; arrangement selection also chooses the matching piano roll. Global undo/redo covers structural edits and folders. Playback stops before structural changes; Play publishes the next prepared session.

```sh
cargo run -p soniclaw-daw-workspace -- NEW_DIRECTORY
```

This example creates a blank project, adds/duplicates/reorders an instrument with two clips, assigns folders, compiles, undoes/redoes and reopens the portable folder.

## API and ownership

`studio::workspace::{blank, apply, Command, Kind, Layout}`. `blank(name,rate,max_block)` constructs a valid one-track project. Commands Add/Rename/Delete/Duplicate/Move/Folder/AssignFolder/AddMidi operate on a cloned candidate, reindex clip track references and validate before committing. Wrap apply in History::edit for undo. All operations are allocating control-thread operations; stop audio/native leases before changing structure. No command opens hardware or deletes source files. The GUI owns publication/restart.

Processor parameter IDs are namespaced under descriptor ID and mixer track ID; duplication remaps those prefixes. Custom factories should follow that namespace contract. Regions, assets, folders and processors retain distinct identities. Duplicating uses IDs up to 40 characters so derived processor/region IDs stay within the 64-byte engine bound. The caller supplies unique track and clip IDs; UI generates them. A failed command leaves the model unchanged. Native opaque state is copied as configuration; active voices/DSP history are not duplicated.

## Persistence and migration

Portable project schema advances to 3 and stores track folder Layout. Schemas 1/2 remain readable; open_with_report records migration. Legacy schema-1 identity derivation remains stable when layout is absent. Older framework readers reject schema 3; save a separate copy when evaluating upgrades. Layout participates in project history, recovery and consolidation. Folder membership is presentation metadata; it does not change mixer routing. Existing empty projects gain one audio track through blank creation rather than permitting an invalid zero-track engine.

View selection, open dialogs and inspector edit text are UI state. Project/track names, ordering, clips and folder assignments are portable. Existing sources and previous folders remain untouched. Unfreeze reindexes restored region track references after track moves.

## Bounds, errors and evidence

Existing stereo session limits remain: 32 tracks, 16 streamed regions, 4 auxiliaries and engine/DSP budgets. This release does not yet provide independent physical inputs, arbitrary submix buses or device acceptance. Folder count is 128, names 1..256 bytes; invalid IDs, collisions, unsupported indices, deleting the final track, invalid clip spans and metadata overflow reject atomically. Long-running processing/compilation still belongs outside callback. Source/media cleanup is explicit through the existing audit/consolidation workflow.

Selected Rust tests, all-feature workspace Clippy/docs and the example are recorded in the release validation folder. Native desktop/device behavior on Windows/macOS and physical I/O remain pending unless separately executed. This is the first component of the requested DAW milestones, not a declaration that all three milestones are complete.
