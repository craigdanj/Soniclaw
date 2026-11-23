# Studio recording controls — 0.62.0

## Workflow and example

Open the reference studio (`cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT`), expand **Recording controls**, arm individual tracks and optionally the master. Choose **Dry mapped input** for unprocessed input or **Track with inserts and clips** for the complete track signal. A rendered track includes arrangement clips and monitored input: it is not an isolated wet-input stem. Enable default audio input deliberately, select zero-based L/R input channel indices (use 0/0 for mono), and enable each track's monitor checkbox if desired. Playback applies this monitor overlay without changing the portable document. Observe safe monitor levels externally.

Record starts a fresh prepared engine; playing first is unnecessary. Set start beat, count-in quarter beats, up to ten seconds of pre-roll, and optional single-pass punch beats. Count-in holds arrangement transport while live input/MIDI continues. Pre-roll is clipped at timeline zero. Count-in tempo is constant at the prepared start tempo; subsequent metronome follows the actual session tempo map and loop clock. Count-in sounds even with ongoing metronome disabled. Click is added after the recording taps, so no stem contains it. Set click gain before Record; metronome can be toggled live. Finish take ends manually; punch completion automatically finalizes on the control thread while backing playback continues.

Run a hardware-independent end-to-end example:

```sh
cargo run -p soniclaw-studio-recording -- NEW_DIRECTORY
```

It supplies deterministic stereo PCM, records an exact punch, verifies dry audio and click exclusion, saves a portable project and reopens its take. `report.json` contains the executed schedule. See unit tests for callback partition invariance, monitoring through inserts and zero callback allocation.

## Public API and ownership

`studio::recording::{Settings, Arm, Source, prepare, ArmedSession, Schedule, RecordingProcessor, Progress, Status}`. Validate settings against the current document; `Settings::for_document` arms existing tracks. Call `prepare(project, factory, settings, new_take_directory, take_id, latency_budget)` on the stopped control thread. It builds/ primes processors, opens the multitrack writer, queues its tap before the first callback and returns the processor, controller, recorder and shared progress. Publish the processor through an Engine; do not independently pause/seek its controller while recording. The wrapper owns transport timing and rebases MIDI events at count-in boundaries. Fixed event/click storage and atomic progress require no callback file I/O, locks or allocation. DSP and metronome computation still consume CPU; no throughput benchmark is claimed.

`Progress::status` reports count-in/record/punch-complete phases and acquisition frames. `set_metronome` is atomic. Poll completion on the control thread, call `Recorder::stop`, detach the tap and collect retired owners via the controller. The writer's exact punch interval clamps callbacks which race with GUI polling. Stop the device before destroying native owners; the existing studio stop/checkpoint lifecycle remains authoritative. No disk I/O or native checkpoint belongs in the audio callback.

## Alignment, persistence and compatibility

Schedule separates project start, acquisition count-in, pre-roll and the maximum selected stem latency tail. Count-in is excluded from recorded files. The take's clock annotations preserve musical source position, and existing comp alignment uses per-stem latency. The master uses the prepared live graph's fixed latency budget. A looped arrangement is supported only for manual open-ended recording; explicit punch rejects looping because its single-pass beat range would otherwise be ambiguous.

Settings use strict schema 1 JSON, bounded to 16 KiB via `to_json/from_json`; callers may persist it separately and validate against the reopened document. Reference studio controls are session-local and reset when opening/restoring a project. It writes `recording-schedule.json` beside each original take for inspection; the core portable project copies take manifests/audio, not this optional app-side schedule. Existing take schema is unchanged. Takes become part of normal portable project save and project history; undo never deletes their files.

## Bounds, failures and recovery

One to 32 selected stereo stems including master; up to 32 count-in quarter beats; start/punch within 0..1,000,000 beats; pre-roll 0..10 seconds; click gain 0..0.25; existing stereo session/block constraints apply. Duplicate/unknown arms, a track named `master` colliding with the selected master stem, empty selection, nonfinite controls, invalid ranges and future settings schemas fail before recording. GUI rejects dry/input-monitor capture without physical input enabled; library users may deliberately provide virtual PCM. Sources/channel maps and timings are fixed for one take. Input/output device support, drift compensation and overflow behavior remain those of the duplex backend and recorder. Missing frames are reported, never silently called successful audio.

The take directory must be new. Preparation or device errors are surfaced; a partially created take is evidence for inspection, never automatically overwritten. Finishing and file writes can block the control thread. No physical hardware, external DAW, Windows/macOS runtime or broad native plug-in acceptance was executed for this release. This is a reference studio recording workflow, not a claim of a production DAW.

## Startup correction in 0.70.0

Recording now calls `Document::compile_streamed_at` to prime disk pages at the planned project start before publishing audio. The new control-side API primes streaming pages for the caller's chosen starting sample; the caller must still supply that sample through transport/Controller::seek. Existing compile_streamed retains start-zero priming. Live seeks keep their existing asynchronous underrun policy. A regression test records the first callback of a disk source starting at frame 48,000 and verifies actual PCM rather than startup starvation.
