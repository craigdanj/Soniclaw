# Disk-streamed arrangement clips (0.45)

## Workflow and runnable example

`cargo run -p soniclaw-streamed-session -- NEW_DIRECTORY` writes a sixty-second original float WAV without constructing its full PCM in memory, then plays three seconds through the unified session engine alongside a MIDI instrument, automation and the mixer. Its report compares source file size with fixed PCM cache size, reports page/underrun/error counters and confirms no callback allocations. The driver paces callbacks to let the actual asynchronous file worker run; it is not a hardware/deadline benchmark.

Add `Track::streamed` DiskClips and call `studio::Document::compile_streamed(&preloaded_assets,&disk_catalog,&mut factory)`. The DiskAssets catalog maps stable asset IDs to paths on the control thread. Serialized clips contain IDs, timeline/source-frame bounds, looping, gain and fades; they do not cause arbitrary paths to open by themselves. `compile` without a disk catalog still supports old sessions and returns a preparation error if enabled streamed references cannot resolve.

## Standalone API and integration

`soniclaw::stream::clips::{DiskClip,DiskAssets,StreamedClips}` also works outside the studio. `StreamedClips::new(clips,catalog,spec,cache_budget)` opens metadata/initial heads, prepares existing seekable WAV players and starts bounded page workers off the callback. `mix_into(stereo_pcm,transport)` adds regions to an existing signal; the AudioProcessor implementation replaces output instead. Regions honor timeline offsets, source trims, loop periods, overlaps and linear region fades. Mono source audio maps to both channels. Source/project rates must match; resample offline deliberately when importing another rate.

The session renderer adds streamed/preloaded PCM before instrument latency alignment and inserts, so track controls, automation, PDC, sends and prepared project replacement use the same path. No full-file decode occurs. One seekable worker/cache is prepared per enabled clip, up to sixteen per session, including overlapping references to the same file. This favors simple independent seeks over a global shared page scheduler; thread/file-handle count is bounded but not optimized for hundreds of regions.

## Prefetch, seeking and telemetry

`prime_at(sample,timeout)` is an explicit blocking control-thread operation before publication or while stopped. It seeks to relevant source positions, drains ready pages and retries bounded prefetch requests that could have been refused behind older seek generations. Preparation primes initial positions; subsequent live seeks/loop boundaries can produce silence until pages arrive. Never wait/prime from a callback. New `SeekablePlayer::prefetch` retries requests without I/O; `ready_frames` observes contiguous cached coverage after response drainage.

Audio processing only reads immutable heads/fixed page caches, copies/mixes PCM and uses bounded SPSC requests/responses. Late pages carry generations and cannot restore old seek audio. Disk misses are silence at the correct timeline position; the worker never blocks the callback. `Controller::streaming_stats` and PreparedSession telemetry expose each clip's cache bytes and existing DiskStats: underruns, I/O errors, request overflow, pages read and stale responses. Inspect these in UI/tests rather than assuming every asynchronous run is glitch-free. Preparation fails on metadata/initial-prefetch errors; later worker faults remain visible in stream telemetry and can yield silence.

## Persistence, ownership, limits and recovery

The studio schema remains 1 with an additive optional `streamed` field; old JSON defaults to an empty list. Earlier framework builds with strict unknown-field rejection cannot read new streamed sessions. Catalog paths and relocation policy remain application-owned; reconnect catalogs after load. Keep source files immutable during playback and retain a previous valid prepared session when a path/rate/bounds check fails. This release does not hash/monitor changes to an open source file or repair a failed device/disk automatically.

Construction, metadata reads, prefetch waits and worker joins/destruction are control-thread operations. Stop playback before final teardown. For live replacements, use the studio Controller and regularly collect retired graphs so obsolete stream workers join outside the callback. Never drop the last streamed renderer from its process method.

Limits: stereo output, mono/stereo PCM/float WAV supported by the existing paged decoder, exact sample-rate match, <=1024 callback frames, <=16 clip workers per session, source-frame bounds within RIFF/WAV u32 frame addressing and timeline positions <=2^48. Clip gains are -4 to +4; fade sums cannot exceed region length. Standalone cache budget per clip is 128 KiB–2 MiB; the studio uses a 1 MiB ceiling per clip, while actual PCM page accounting is smaller and reported. Budgets cover heads/pages/queues, not path/decoder/thread metadata or whole process RSS. No MP3/FLAC streaming, sample-rate conversion, time warping or guarantee of uninterrupted playback from a slow disk is included.

## Validation

Tests cover offsets/fades/looping/overlap with exact cached PCM, callback allocations, a distant seek followed by return to the original generation, initial-prefetch retry and rate/bounds rejection. Existing disk sampler/seek tests still run. The example executes real worker-backed long-file playback through the shared studio path and records all stream counters. Hardware and hostile/slow storage acceptance remain pending.
