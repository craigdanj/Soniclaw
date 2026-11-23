# Multitrack recording and takes (0.46)

`soniclaw::record::multitrack` writes synchronized float32 WAV stems on a worker thread. The live studio can capture any prepared track after its inserts and track latency alignment, the master output, and dry device input. Existing stereo `RecordTap` applications continue to work.

## Runnable workflow

```sh
cargo run -p soniclaw-multitrack-session -- new-takes
```

The example records bass, synth and master, saves two immutable takes, punches the second take with the synth disarmed, verifies silence in that punch section and reloads the selected take. Inspect `take-a/take.json`, `take-b/take.json`, `takes.json`, the WAVs and `report.json`. The example uses generated session audio, not a physical input device.

On the control thread:

1. Compile a studio document, create `live_pair`, and let the callback apply its initial revision.
2. Build `(file_id, RecordingSource)` pairs. `Controller::recording_specs` supplies stereo channel counts and latency metadata. Create `multitrack::pair(Config)` with these specifications.
3. Call `attach_multitrack(tap, sources)`. Run callbacks before starting a take to accumulate pre-roll. `Recorder::stats().latest_sample` reports the acquisition clock.
4. `Recorder::start(new_directory, TakePlan)` opens fresh files. `set_armed(index, false)` writes silence for that track while all files retain the same duration. Start refuses an existing directory.
5. `stop()` drains published packets to the captured stop boundary, truncates racing future samples, fixes WAV headers and writes the manifest. Check `complete`, `missing_frames`, `queue_drops` and each track's error.
6. Stop first, then `detach_multitrack()`, allow an audio callback to acknowledge it and call `collect_recorders()` or `collect_retired()` off the callback. Project replacement is rejected until that retirement completes.

Attach consumes the tap; validation or a busy command queue can return an error, destroying that rejected tap on the caller's control thread. Only one multitrack binding may be attached. Tagged detach commands cannot detach a later attachment.

## Acquisition time, punch and pre-roll

The recording clock starts at the first attached capture, increases once per outer callback, and does not rewind when the musical playhead seeks or loops. Punch boundaries are a half-open `[start, end)` interval on this clock. They are not beats or transport samples. A future punch is supported. Stopping before its start produces a valid empty take.

Configured pre-roll precedes the take/punch start in the file. `acquisition_start`, `file_start`, `pre_roll_frames` and `punch` identify its placement; retained history outside the armed period can contain previously armed audio. Missing history and dropped queue packets become synchronized zeros, with explicit counters. A manifest is complete only when finalization succeeded; completeness does not imply there were no missing samples.

`latency_frames` is metadata: input is zero, studio track stems use the common track delay compensation, and master uses the live graph's fixed latency budget. This does not measure device round-trip delay or automatically compensate converters. Align imported stems using these values and the recorded acquisition interval.

## Standalone API and callback contract

`pair(Config)` also accepts any producer supplying `Tap::capture(sample, &[&[f32]])`. Inputs are interleaved per track with an identical frame count. The tap validates every input before publishing any. Time must be monotonic; gaps over ten seconds and backward positions are rejected. Nonfinite or excessive samples are rejected. Valid values above full scale are preserved by float WAV encoding.

The callback copies fixed-size packets to a bounded SPSC queue, never waits for disk and never allocates. Arming uses atomics. The worker owns files, commands, history and manifests. `start`, `stop`, file selection, joins and destruction belong on a control thread. The studio stages per-track samples into prepared atomic storage so per-sample automation does not create per-sample disk packets; capture is once per outer callback. Do not destroy the entire processor on an audio callback.

Bounds: 8–192 kHz, 1–32 tracks, 1–8 channels per standalone track, 64 total channels, blocks up to 1024 frames, 4–1024 queue packets, at most ten seconds of pre-roll and 16 Mi samples of history. Each packet holds 128 frames and up to 64 channels. Storage cost includes unused packet channels, history timestamps, files and staging; size the queue deliberately. Conventional RIFF WAV has a four GiB per-file ceiling; finalize and start another take before reaching it. Missing-range bookkeeping is bounded to 4096 ranges and reports an error if exhausted. Observed peaks may include a racing tail subsequently trimmed at stop.

## Persistence and recovery

Schema-1 `Take` records identities, file names, sample rate, channel counts, timing, latency, peaks and errors. `Take::read(directory)` limits manifest size, rejects incomplete manifests, unexpected paths and mismatching WAV headers. An error leaves evidence in the take directory; choose a new destination rather than overwriting it. This is file validation, not a cryptographic integrity check.

`TakeSelection` schema 1 stores up to 256 directories plus a selected take ID. `to_json`, `from_json` and `resolve` validate the referenced manifests and selected identity. Selection changes metadata without rewriting recordings. This release does not split or crossfade comp regions, automatically create loop-cycle takes, or import the selected take into the arrangement. Applications can build `DiskClip`s from the validated float WAVs.

No existing project schema changes are required. Take paths are caller-controlled resources, not executable plug-in identities. Move recordings together and update selection directories when relocating a project.

## Verification

Unit tests cover independent arming, stereo/mono synchronization, pre-roll, punch, float headroom, immutable destinations, selected-take reload, invalid buffers, timestamp gaps, zero-filled loss, studio tap lifecycle, replacement exclusion and zero callback allocations. The executable produces real WAVs and checks the selected punch. See [release checks](validation/v046/REPORT.md). Hardware recording, device disconnect and long-duration disk exhaustion require product acceptance testing.
