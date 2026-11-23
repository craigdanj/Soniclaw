# Streaming-capable freeze (0.57)

Freeze now renders streamed WAV regions through the same clip/instrument/insert session engine as memory sources. It waits for the required disk pages off the audio thread, renders fixed-size blocks and writes an immutable pre-mixer stem. The original source, track processors and parameter lanes remain available for reversible unfreeze. This replaces the earlier memory-only freeze restriction.

## Try it

```
cargo run -p soniclaw-stream-freeze -- NEW_DIRECTORY
cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT
```

The software example freezes a two-second paged WAV through a gain insert, compares every exported sample with the expected result, saves a portable project and restores the original streamed track after reopening it. `report.json` records duration, waveform error and job progress. Unit tests cover another paged source, stale result rejection, cancellation and portable unfreeze. The existing synth freeze/reference studio headless action test now waits for its real background task.

In the reference studio, Freeze stops playback and captures current parameter/native state first. Builtin-only sessions then use a background job; the interface shows processed/total frames and a Cancel freeze button. Completed output is checked against the current project before being applied. Editing during preparation can make it stale. Saving before completion saves the current unfrozen project. A native CLAP session uses the synchronous control-thread factory path because its owners must remain on their owning main thread; that operation can pause the interface. Background native GUI rendering is not implied.

## Typed offline rendering

`Document::compile_offline(assets, disk, factory)` returns `OfflineSession`. Call `render_block(buffer, context)` on a control/worker thread and inspect its `Result`. This wrapper deliberately does not implement `AudioProcessor` and does not expose its inner prepared processor. It cannot be passed to the normal live-session publish API. The compiler's offline mode is private; live creation/replacement additionally rejects an offline-marked instance internally.

Offline tracks prefetch/drain their page responses before each mix block using a bounded two-second page wait. The existing streamed-region renderer still performs the actual source-phase, fade and envelope calculations. Missing/unreadable pages fail the offline render instead of publishing a stem containing silent underruns. A page error may be discovered by the deadline; the deadline does not guarantee that an operating-system read or native processor can be interrupted. Live rendering retains its nonblocking silence/telemetry policy and never calls this wait path.

The renderer prepares the full session and captures the selected track before its mixer. It prerolls from timeline zero to preserve processor state and compensates the prepared track alignment latency when selecting output. Length is the requested musical frame range; tail extends that range. Timeline loop behavior remains that of the project clock. Disable looping for a linear arrangement bounce. Live audio/MIDI input must be disabled on the target track before freeze. Side effects and external resources of custom processors remain the factory's responsibility.

## Background ownership and publication

`freeze::render` remains the synchronous API. `render_monitored` additionally accepts a `Monitor` with `cancel()` and `progress() -> (completed,total)`. Total includes preroll and processing latency and is available after preparation. Cancellation is checked before work, between render blocks and before final manifest publication. Hashing/preparation are not instantaneously interruptible.

`freeze::Job::spawn` owns a document/catalog snapshot, destination and a `Factory + Send + 'static`. The factory must create worker-owned processors; do not move main-thread-only native owners into it. Each Job contains exactly one worker and fixed render buffers. The application bounds concurrency by retaining one job per slot; the reference studio permits one. `poll()` returns None while work remains and joins only an already-completed worker. Drop cancels and joins on the caller, so perform shutdown off callback and account for any pending I/O/native work.

`Project::apply_frozen` verifies the result and commits document, disk catalog and reversible manifest together. The fingerprint is recomputed after rendering and again against the current project during apply. Source bytes or track/processor/automation changes therefore reject stale output. Mixer changes remain live by design. A completed stale cache remains a valid artifact in its exclusively created directory; the caller may inspect or retire it after ensuring no player uses it. No unrelated path is deleted.

## Resource and persistence contract

The page cache stays at one MiB per active streamed region; there is no whole-file decode. Up to sixteen streamed regions fit the studio session. Render mix/stem buffers are sized to the prepared block limit, at most 1,024 stereo frames, plus existing processor and PDC buffers. A freeze is limited to four hours and the stereo float-WAV RIFF size bound. Disk hashing uses a fixed 64 KiB buffer and conservatively includes the whole resource catalog. User-defined processors can have additional memory costs.

An existing destination is refused. Ordinary error/cancellation after folder creation removes only that newly created folder; no source or previous cache is overwritten. Native process termination cannot guarantee filesystem cleanup. Only a successfully finalized WAV, checksum and freeze manifest can be verified/published. Project source/freezes schema remains version 1, and the original resource catalog is copied by the portable save workflow. No new required JSON field is introduced in this release.

The release checks demonstrate software rendering and headless controls. Physical audio, desktop windows, long-running storage stress, and arbitrary third-party processor acceptance require target-system tests. See [executed evidence](validation/v057/REPORT.md).
