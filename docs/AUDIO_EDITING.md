# Nondestructive audio editing (0.55)

Audio regions can now be split, joined, slipped, crossfaded and given gain envelopes without modifying their PCM. The same edit transaction and gain calculation serve memory assets and disk-backed WAV regions. The reference studio exposes these operations under its audio tools panel; the arrangement includes both resource kinds.

## Workflow

Select a region in the arrangement, open Audio tools and enter a split frame or source slip. Split creates a new region identity; joining requires adjacent, compatible regions. Select two overlapping regions on the same track to create a linear or equal-power crossfade. Add frame/gain points and apply the gain envelope. Link selected regions when their relative placement should survive a move or deletion. Unlink before deleting an individual member. Undo and redo use the existing bounded editor history.

Splitting alone preserves the waveform, including loop phase, existing fades and gain curves. Joining split siblings restores that waveform. Joining independently authored regions requires contiguous non-looping sources and compatible outer fades; incompatible curves are rejected. Joining is not an implicit render-to-file operation. Slipping changes which source frame plays at the same timeline position. Loops wrap negative slips; a non-looping source cannot slip before its source window and becomes silent after its end.

Crossfades require two properly overlapping, same-track regions. Linear fades preserve unity for identical material; equal-power fades suit less correlated signals and can increase the level of correlated signals. Leave headroom. A gain envelope multiplies the region's static gain and fades. Frames are local to the original envelope basis, with linear interpolation and endpoint holds. Frame zero and the region endpoint are useful first and last points.

## API

`soniclaw::clips::Edit` adds `SplitAudio`, `JoinAudio`, `SlipAudio`, `GainEnvelope`, `LinkAudio` and `CrossfadeAudio`. Use `Document::apply` for a validated atomic model edit, or `TimelineEditor::apply_audio` to include undo. `MoveAudio` and `RemoveClip` propagate through a link group. Slip, gain and fade edits apply to the specified region only. `History` retains sixteen snapshots; audio sample buffers remain shared.

`AudioClip::edits` and `stream::clips::DiskClip::edits` contain optional boxed `RegionEdits`. Metadata tracks unwrapped source phase, independent fade and envelope bases, curve points, fade shape and an optional link identity. Splitting shifts phase and curve offsets rather than rebasing the sound. Resizing resets the fade basis; explicit trim resets phase, clips/rebases the gain envelope and preserves the link/shape. Setting fades changes only the fade basis; setting a gain envelope changes only its envelope basis.

`stream::clips::edit_regions` applies the same edits to streamed metadata without decoding a WAV. `DiskClip::as_audio` and `from_audio` provide explicit control-side conversion. `RegionEdits::gain_at` is the shared render path; it uses bounded lookup and allocates no memory. `AudioClip::source_phase` and `region_gain` expose those calculations.

`studio::project::Project::edit_view` returns an `ArrangementDraft`. Edit its public `document`, then call `replace_edit_view`. Private source aliases distinguish memory and disk catalogs even when their original IDs match. The replacement validates every region and track mapping before committing. Existing `arrangement`/`replace_arrangement` remain memory-only APIs. Globally unique region identities are required in the combined editor. Applying a comp recipe again replaces that comp track's manual playback-region edits; original takes remain unchanged.

## Example and verification

```
cargo run -p soniclaw-audio-editing -- NEW_DIRECTORY
```

The example creates a loop with curved fades and three envelope points, checks sample-exact split/join, slips the source, splits its disk representation, compares every floating-point sample against the memory renderer, saves/reopens a portable project and exercises the actual headless editor controls. The source WAV is float32 to avoid 16-bit export quantization in this comparison. The listening export remains ordinary WAV. `report.json` records the observed error, source immutability and measured callback allocations. Unit tests also cover linked edits, rollback, curve persistence and overlap unity. Physical audio and native windows are separate acceptance checks.

## Persistence and bounds

Schema-1 clip JSON may include an `edits` object. Old JSON loads with `None`; metadata-free output omits the field. Older strict readers may reject the new field. Rust struct literals must add `edits: None` or provide metadata; this is an explicit pre-1.0 source API change. Portable projects persist both memory and disk region metadata and retain source checksums.

Each region accepts at most 1,024 strictly increasing gain points, finite gains from zero through eight and a valid link ID. A document accepts at most 65,536 gain points and retains its 256-region/64-track bounds. Frame arithmetic is checked against the existing 2^48 timeline limit. Disk-source bounds remain those of the streaming WAV engine. Current live studio streaming permits sixteen regions across the session. Every edit operates on the control thread; a prepared audio instance must be rebuilt/published through the existing stopped or live-session transaction. Editor actions can allocate and must never run in the audio callback.

Invalid joins, split points, envelopes and unsupported source aliases leave the old document intact. On project-save failure, keep the current project open and retry into a new destination. Editing never rewrites the source file; optional exports are separate artifacts.
