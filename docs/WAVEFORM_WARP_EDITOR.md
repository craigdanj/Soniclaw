# Waveform and warp-marker editor — 0.63.0

## Workflow

In Reference Studio expand **Waveform and warp markers**. Enter an original mono/stereo WAV path, Import, select its disk asset ID and the destination track, then Load source. The source is referenced until a portable save copies it. The loader hashes/decodes off the GUI thread and returns immutable bounded peak pyramids. Zoom the normalized source view; select or drag a marker line to change its source frame, edit its target beat numerically, Insert midpoint in the widest interval, or Remove a selected marker. Choose Preserve pitch or Repitch. Invalid marker drafts show an error and cannot render. Render warp cache, wait, then Apply cache as region; Play auditions the prepared region through the real session engine. Overlapping original regions are retained: disable/mute them deliberately to avoid doubled audio.

This release edits disk WAV sources. In-memory assets can still use the timeline editor; export/import a WAV to use this warp workflow. It does not claim transient detection, an inline spectrogram, dragging target beats on a separate beat ruler or a continuous real-time stretching processor.

## API and runnable example

`studio::wave_edit::Draft::new`, `validate`, transactional `insert`, `build`; `Ready::apply`; `Job::start`, `cancel`, `poll`. Draft names an original disk source, destination track, generated asset and region plus the existing schema-1 `tempo_cache::Recipe`. `build` returns a verified cache binding and output path; `Ready::apply` checks project identity/source association, verifies source/cache/timeline hashes and applies to a cloned candidate before replacing the project. Reapplying the same draft updates its existing cache region; identity collisions fail.

`ui::wave_edit::Editor::show(ui, thumbnail, recipe)` draws channel-zero conservative min/max envelopes, selection and markers and returns whether the draft changed. It performs no file I/O. Callers own validation and publication. Zoom and marker selection are editor view state, separate from the project.

```sh
cargo run -p soniclaw-waveform-warp-editor -- NEW_DIRECTORY
```

The example creates PCM, draws a headless egui frame, inserts markers, rejects a duplicate without changing the draft, renders, rejects a different project, applies through project history, undoes/redoes and reopens a portable save. `report.json` records those assertions.

## Lifecycle, persistence and migration

`Job` owns one worker and cooperative cancellation; poll joins only after completion. Dropping cancels and joins on the control thread, which can wait for DSP/filesystem work. Do not call build, validation, hashing, poll, Drop or application in an audio callback. Marker gestures edit only the draft; accepted jobs retain their submitted copy. Editing the visible draft invalidates an unapplied ready preview. Jobs completing after the user loads another project are rejected by identity. A modified timeline/source is rejected by binding verification. Apply stops the reference device and enters global project history; no callback graph is mutated in place.

The existing portable project stores original WAV, generated WAV and recipe/binding hashes. No schema change is required. Unapplied GUI drafts/zoom are session-local. Original PCM is never changed. Successful caches stay on disk even if their result is discarded or undone; caller retention/cleanup is explicit. Existing waveform cache corruption repair/cancellation behavior applies. A preview is a rendered cache subsequently auditioned through Play, not an extra device stream.

## Bounds, errors and evidence

2..256 strictly increasing source frames and target beats, beats 0..1,000,000; existing 8..192 kHz mono/stereo, four-hour/RIFF, stretch ratio and section bounds apply. Marker source coordinates are bounded by actual file duration. Widget width is clamped to 64..2048 peak queries; these are conservative envelopes rather than sample-accurate displays. Target beat edits may be temporarily invalid; validation blocks render. Schema/future IDs, missing source, generated-cache-as-original, cancellation, stale timeline/source, shared cache and region collisions are explicit errors.

The source release verification includes all-feature workspace Clippy, docs and selected tests; the example provides end-to-end application evidence. Native window and physical playback tests remain pending. Pitch preservation retains the existing stretcher's quality limits; no new listening/quality benchmark is claimed.
