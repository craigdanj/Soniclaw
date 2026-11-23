# Scalable arrangement and piano-roll editors (0.53)

`soniclaw::ui::timeline::{TimelineEditor, View, Selection, Action}` provides scrollable, zoomable editing surfaces backed by the existing clip document. Painting and hit testing are culled to visible regions/notes; the model retains exact frame/beat positions. These APIs are reusable UI-thread components and do not own playback or decode audio.

## Run and integrate

```sh
cargo run -p soniclaw-scalable-editors -- new-editor-demo
cargo run -p soniclaw-desktop-studio --features gui -- --gui my-project
```

The software demo creates a region on track 32 with all 128 MIDI pitches, scrolls the arrangement to it, applies a 128-note grouped edit, exercises undo/redo, visits both MIDI pitch endpoints and writes the edited document, viewport JSON and evidence report. It renders real egui frames headlessly; no native window or audio device is opened.

The reference studio now uses the combined arrangement editor across its tracks and the full piano roll for the selected track's first MIDI clip. Its project helper exposes `Project::arrangement()` and `replace_arrangement(draft)` for this integration. Track tags map to session track order on export/import; the full candidate validates before any track changes. This combined view requires globally unique clip IDs and the clip document's region-count bound. Sessions with repeated per-track clip IDs can keep using per-track views or explicitly assign unique IDs before aggregation. Streamed clips remain playback resources and are not represented as editable waveform regions by this first combined view.

```rust,ignore
let mut editor = TimelineEditor::default();
let mut draft = project.arrangement()?;
editor.arrangement(ui, &mut draft, project.document.tracks.len())?;
editor.piano_roll(ui, &mut draft, "melody")?;
project.replace_arrangement(draft)?;
```

Only publish/recompile the validated project through the engine's normal control-thread lifecycle. The reference studio's Apply edits button rebuilds/restarts audio; moving a rectangle does not mutate a running processor.

## Navigation and layout

Both surfaces use horizontal/vertical scroll areas. Frames-per-pixel controls arrangement zoom; beats-per-pixel controls piano-roll zoom. Each arrangement track has its own height, adjustable by dragging its lower edge; the piano roll has a key-height control. It draws the full MIDI range 0–127, with only visible rows/notes painted. Notes and clips beyond the previous six-lane/two-octave view can be reached by scrolling. `last_visible` reports the number of objects painted by the most recent surface call, useful for diagnostics.

`View` schema 1 holds zoom, per-track heights, scroll offsets and snap settings. `to_json`/`from_json` validate bounded JSON. The reference application saves optional `editor-view.json` beside `project.json`; missing view files use defaults. Invalid saved views fail before replacing the active project. Core project loading does not depend on GUI view metadata. A view-file write failure can leave an otherwise valid saved project folder; reopen it with defaults or fix/remove that optional file.

## Selection, movement and undo

Click a clip/note to select it. Shift/Ctrl/Command extends or toggles selection. Drag blank space to marquee intersecting model regions/notes. `select_regions` and `select_notes` provide the same selection semantics without pointer input, allowing command palettes and accessible alternative controls. A drag of one selected object moves the whole selection; the outline previews movement and the model transaction commits on release. Arrangement vertical moves use actual resized track rows.

Focused clips use Left/Right to nudge by frame snap and Delete to remove selected clips. Focused notes use Left/Right for beat snap, Up/Down for semitone transposition and Delete to remove selected notes. Select all notes and Add middle C are ordinary buttons. Existing custom controls and clip views retain their previous APIs; they are not removed.

`apply(document, Action)` clones/validates a candidate through the shared command History, so an out-of-range pitch, negative time, invalid track tag or missing selected identity rolls the entire edit back. It does not silently clamp individual notes and distort the group. Snap rounds each selected item's resulting position; items originally off-grid may change their relative spacing. Undo/redo affects a whole committed operation, not individual selected objects. The history retains 16 document snapshots and is count-bounded rather than byte-bounded. Call `clear_history` after loading/replacing a different document; the reference studio does this on Open. Selection is editor state, not an audio document change.

## Bounds and ownership

All editing, layout, history and JSON work is UI/control-thread work. No new processing or allocation is introduced into the audio callback. Track count is 1–64; heights 24–180 pixels; key height 8–40 pixels. Frames-per-pixel range 1–192000, beats-per-pixel 0.0001–4, serialized view at most 8 KiB, scroll offsets/canvas width at most ten million pixels. The canvas is large and virtualized, not mathematically infinite; zoom out to navigate long projects. Pixel painting uses f32 while frame/beat editing uses integer/f64 model values, so extreme zoom/time combinations have normal display precision limits.

Underlying clip limits remain: 256 regions and 65536 source notes, full MIDI pitch range and 64 track tags. The studio project imposes its own 32-track session limit and rejects edits to nonexisting session tracks. Piano-roll width follows source length. Region/note IDs must remain stable for selection and focus. Full-screen-reader certification, automatic scroll-to-selection and virtualization of streamed waveform previews are separate work.

## Verification

Tests cover multi-region selection, track movement, grouped undo/redo, full-pitch selection, transactional failure at MIDI 127, persisted view validation, actual focused-region keyboard nudging and viewport access to track 13/MIDI endpoints. The executable visits track 32/all pitches and writes a real edit report. The reference studio headless UI/save/reopen tests run against the replacement editors. See [release verification](validation/v053/REPORT.md) for executed checks and pending native desktop acceptance.
