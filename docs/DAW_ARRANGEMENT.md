# Arrangement and musical timeline workflow — 0.78.0

## Workflow

Reference Studio's **Song arrangement and musical timeline** panel adds Play from beat and Locate, tempo/ramp points, meter changes, named markers/sections and duplication. Space toggles Play/Stop when a text widget does not own the keyboard. Locate stops safely, scrolls the arrangement and sets the next start; Play prepares streamed media at that position and seeks the new engine before running. An enabled loop constrains an outside cursor to its start. Recording retains its explicit audio/MIDI start controls.

Select a tempo/meter row to edit it; Apply replaces the point at that beat and Remove removes it. The initial beat-zero tempo/meter cannot be removed. Meter changes must meet the transport map's bar-boundary requirements. Audio normally stays at fixed sample/second positions when tempo changes. Anchor audio starts to beats moves their starts while preserving recorded content duration/pitch; it does not time-stretch audio. MIDI and automation keep musical beat positions. Frozen/warp caches require unfreeze/rebuild before their timeline changes; beat anchoring rejects existing fixed-sample comp recipes rather than silently moving their binding.

New label clears the ID; Apply marker uses Start/beat, while Apply section uses Start/End. Click a saved label to edit it, Locate a marker, or delete a row. Select arrangement regions and a destination beat to duplicate them together. Duplicate selected section clips regions to its range and adds a new section label at the destination. Audio source phase, fades/envelopes and linked-copy identities are preserved nondestructively; MIDI source offset, looping, notes and controller points follow. Files and original regions remain untouched.

Copy automation is explicit. A region selection copies automation for its tracks/processors; a whole section copies all lanes. It overwrites destination points including both boundaries. The copied end holds until the next existing destination point. Disabled lanes/modes remain disabled/unchanged. Partial Smooth/Bezier segments are subdivided mathematically so their values inside the copied span remain accurate. Overlapping destination/source spans use the original source snapshot. This is arrangement duplication, not ripple insertion/deletion.

```sh
cargo run -p soniclaw-daw-arrangement -- NEW_DIRECTORY
```

The example copies a section containing looping audio and expressive MIDI, samples its copied automation curve, verifies phase/fade metadata and undo/redo, compares seconds/beats anchors, reopens labels and renders nonzero audio from a chosen nonzero playback sample. No hardware opens.

## API and ownership

`studio::arrangement::{Model,Marker,Section,Command,AudioAnchor,apply,change_timeline,duplicate_selection,duplicate_section}`. The project layout holds portable labels. Commands upsert/remove tempo, meters and labels. Change_timeline validates transport and resource bindings before adoption. Duplicate functions return the new region IDs for UI selection. Every edit uses a cloned candidate and validates before commit; wrap it in History::edit for undo. The GUI stops/snapshots native processing before structural actions, prepares media pages for playback and preserves existing trusted factory ownership. File playback/rendering remains callback-only on prepared storage; all cloning, curve splitting, hashing/resource validation and graph preparation are control/worker work.

Audio duplication works through Project::edit_view and its typed memory/disk aliases. A crop advances source, fade and envelope phases instead of decoding/resampling. Copied linked clips receive a fresh shared link ID, distinct from originals. MIDI retains source data, adjusting its source phase and duration; a wholly trailing silent source slice becomes a valid empty clip. Selection copies retain relative musical starts. Audio content stays fixed seconds, so copying into a region with different tempo does not automatically stretch it; use the elastic editor for that operation.

## Persistence and migration

Project schema 3 gains optional layout arrangement metadata, omitted when empty. Previous projects read with no labels; legacy identity derivation remains stable for empty data. Tempo/meter changes remain in the existing Timeline schema 1. MIDI control data and original raw performances remain portable. Old strict readers may reject populated new fields. Cursor, inspector edit text and current selection are reference UI state; marker/section names and positions are project state. View scroll/zoom follows the existing optional sidecar. Existing cached sources are never silently invalidated or deleted.

## Bounds, errors, evidence and limits

There are at most 1024 markers, 128 sections, unique IDs across both, nonblank names up to 256 bytes and beat coordinates 0–1000000 with increasing section ranges. Tempo/meter limits remain the transport's: 1024 points each, 20–400 BPM and supported meters/bar boundaries. Clip/track/resource bounds remain unchanged; large copies or automation expansion can exceed them and then reject atomically. No native module is loaded for a model edit. Invalid spans, missing selections/sections, ID/name/range errors, invalid meter placement, stale caches and unsupported curve subdivision report an error and preserve the previous project. The automation model's lane/point limits still apply.

These labels are not a score/notation or video-timecode editor. Copies preserve rendered/fixed audio; automatic source re-warp after arbitrary tempo changes is explicit future work in the elastic workflow. Comp recipes use their recorded sample clock, so use seconds anchoring or render/bounce before retiming those bindings. Choosing a historical MIDI take after changing its tempo/loop may require explicit import because its original take identity/timeline checks remain enforced. Tests cover transactional failures, exact partial spline values and GUI label edit/undo. Full validation/example evidence is adjacent to release notes; physical hardware, arbitrary native products and Windows/macOS runtime remain pending.
