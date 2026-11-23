# Loop recording and take comping (0.54)

This release records sample-exact musical loop/transport boundaries with synchronized stem audio, derives immutable take lanes and builds nondestructive comp recipes. It includes a reusable take-lane editor, a bounded-memory comp renderer and reference-studio/project integration.

## Run

```sh
cargo run -p soniclaw-loop-comp -- new-comp-demo
cargo run -p soniclaw-desktop-studio --features gui -- --gui my-project
```

The software demo captures four loop passes, chooses two ranges from different passes, crossfades them, exports a float WAV, adds a disk-backed comp track, saves/reopens the portable project and plays the comp through the real session engine. It verifies the original WAV is unchanged, the expected audio levels, zero capture/playback allocations and the headless take-lane UI. Reports and recipes are written beside the source take. The synthetic pass test is complemented by an engine test that captures actual loop wraps at 0/240/480/720/960 across uneven 127-frame callbacks.

In the reference studio enable Loop arrangement, set its beat bounds and Apply edits/Play. Record stems for several cycles, Finish take, then open Loop takes and comping. Load the take's passes, choose a recorded stem, select a pass and frame range, and press Use selected pass in range. Clicking an existing comp section selects its range. Crossfade frames controls the joins. Undo/Redo comp operates on recipe drafts. Add/update comp track commits a new audio-only track; original tracks and recordings remain present. Mute the source tracks while auditioning the comp. Saving preserves applied recipes and take resources; un-applied comp drafts are not saved.

## Boundary capture

`record::multitrack::Tap::capture_with_loops(sample, inputs, boundaries)` accepts sorted `LoopBoundary { sample, source_frame, kind }` annotations on the same acquisition clock as PCM. Boundaries and audio publish together inside the same fixed SPSC packets, so a finalized take cannot race against a separate delayed marker queue. No UI polling or timer estimates a wrap.

Studio recording supplies these annotations automatically from the actual prepared musical clock. `BoundaryKind::Loop` marks a real wrap or an initial/resumed start at the loop beginning. Transport stops/resumes and discontinuous seeks generate Transport boundaries. Only adjacent Loop boundaries form a complete pass, so a paused/interrupted cycle does not silently become a valid take lane. Recording may begin mid-cycle; the incomplete first/last cycles are retained in the source WAV but excluded from complete passes. Record at least two wrap boundaries to obtain a complete lane.

Standalone callers remain responsible for supplying truthful source-clock annotations. The unchanged `capture` method supplies none and still records ordinary takes. Metadata is optional when reading older take manifests. New fields can be rejected by older strict readers; keep the framework and take-reader versions aligned.

## Comp model and timing

`record::comp::Recipe::new(&take)` derives source passes from a complete, gap-free take. All selected passes must have the same loop start and frame duration; recordings with a changed loop range/tempo need to be split before comping. `choose(take, start, end, pass_id)` splits/merges recipe sections transactionally. A failed selection leaves the recipe unchanged. Sections cover exactly one loop with no holes, and each chooses a complete immutable pass. `to_json`/`from_json` validate schema 1 and verify the saved pass identities against the referenced take annotations.

`regions(take, stem_id)` produces source offsets, timeline positions, lengths and fades. Each stem's recorded processing latency is added to its source offset exactly once. A pass is eligible only when the source file also contains enough latency tail for every stem. This preserves synchronized selection across stems; callers can apply the same recipe to several recorded tracks. It does not measure or compensate physical converter latency.

Crossfades extend neighboring sections around their join without modifying source files. They use linear amplitude ramps, appropriate for a first comping workflow; no phase correction or equal-power correlation detector is implied. Fade ramp length is one less than overlap length, keeping unity amplitude when crossfading identical aligned signals. Crossfade length must be zero or 2–4096 frames, and each section must leave enough room for the overlap. Very short sections reject excessive fades rather than changing the requested edit silently.

`render_new(take_directory, stem, new_wav)` exports one loop as float32 WAV using fixed 1024-frame working PCM and seekable source reads. It runs off callback, refuses an existing output and removes only its newly created output on failure. The original take is never rewritten.

## Projects and reusable UI

`studio::project::Comp` stores take index, stem ID and validated recipe. `Project::apply_comp(new_track_id, comp)` creates or updates an audio-only comp track with streamed regions, adding its WAV to the disk catalog. Existing non-comp tracks cannot be overwritten. Recorded post-insert stems are not passed through the original instrument/inserts again. Other source tracks are retained and are not automatically muted. The project persists applied recipes and reconnects them to relocated take folders.

`ui::comp::CompEditor` edits a recipe using a virtualized pass list, range controls, section buttons and 16-entry snapshot undo. It owns no file or playback thread. `reset(recipe)` clears its draft history when selecting another take. The reference application uses this same component. Recipe-level edits are independent of the arrangement editor's history.

## Bounds, errors and verification

Existing recorder limits remain. A callback can annotate at most one boundary per frame and at most the prepared 1024 frames. The worker retains at most 4096 annotations per take; exhaustion marks the take incomplete with an explicit error. History adds bounded source-position/kind metadata alongside its existing PCM/timestamps. Missing PCM or queue drops cause recipe creation to fail. The take manifest remains capped at one MiB.

Comp recipes allow at most 256 sections and a one-MiB JSON input. The project's existing 16-streamed-region total and 32-track session bounds still apply to comp playback: a dense recipe can export offline but be rejected as a live comp until simplified. Source stems must be project-rate mono/stereo for live studio playback; offline comp rendering supports the recorder's channel counts. All reads, seeks, JSON, history and export occur on control/worker threads. Callback capture uses preallocated atomic staging/fixed packets and performs no allocation or disk access.

Tests exercise actual-clock wraps, fixed-packet annotation transport, immutable source bytes, recipe rollback, overlap rendering, portable comp reload and zero callback allocations. See [release evidence](validation/v054/REPORT.md). Physical loop recording, long disk-stress runs and native GUI use remain target-machine acceptance work.
