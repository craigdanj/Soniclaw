# Seekable transport and per-voice disk sampling

The legacy `stream::open_wav` remains a forward-only player. New APIs are
`open_seekable_wav`, `open_disk_sampler::<N>`, `DiskAsset`, `DiskZone`,
`DiskHandle`, `DiskSampler<N>` and `SeekablePlayer`.

```rust
use soniclaw::{prelude::*, stream};
use std::time::Duration;
let (mut handle, mut player) = stream::open_seekable_wav("take.wav", 1024 * 1024)?;
player.prepare(ProcessSpec { sample_rate: 48000.0, max_block_frames: 256, channels: 2 })?;
player.seek(48000)?; // source frames, audio-owner operation; performs no file I/O
handle.wait_ready(1, Duration::from_secs(1)); // control thread only
// A process call drains ready pages; a zero-frame call can prime before playback.
player.set_loop(Some((48000, 96000)))?;
player.set_playing(true);
```

`SeekablePlayer` supports pause/resume, arbitrary source-frame seek, looping,
position queries, EOF silence and reset-to-start. Its output rate must match the
WAV rate. Seek increments a cache generation: stale worker responses cannot
restore pre-seek audio. A seek can produce silence until pages arrive. Paused
rendering drains completed pages without advancing position. Check loop bounds
against the actual file; empty/reversed/out-of-file loops are rejected.

## Disk-backed instruments

```rust
use soniclaw::stream::{open_disk_sampler, DiskAsset, DiskZone};
let (handle, instrument) = open_disk_sampler::<32>(
    vec![DiskAsset { path: "piano-c4.wav".into(), root_note: 60 }],
    vec![DiskZone { keys: [48, 72], ..DiskZone::single(0, 60) }],
    8 * 1024 * 1024,
)?;
```

Each voice has an independent source position, generation and fixed page cache.
Key/velocity zones, independent round-robin groups, layering, pitch shifting,
sustain, release, loops, explicit MPE zones and supported CLAP expressions are
implemented. `set_quality` chooses linear/sinc-fast/sinc-best interpolation;
`set_envelope` configures attack/release while stopped. The default is sinc-fast
with a 5 ms fade-in and 50 ms release. Oldest-voice stealing replaces the old
voice and fades in the new one; disk voices do not retain a retiring crossfade
voice. Preloaded `MultiSampler` still supplies its separate stealing crossfade.

Only the first 1024 stereo-expanded frames per asset are preloaded. A single
worker seeks/decodes mono/stereo PCM or float WAVs into 1024-frame pages. Audio
requests current/neighbor pages and reads only immutable heads or fixed caches.
No file opens, seeks, reads, locks, allocations or worker joins occur in note-on
or rendering. Fast offline rendering may outrun the worker; this is explicitly
reported rather than silently switching to full-file preload.

## Bounds and telemetry

- 1–64 voices, 1–128 assets and 1–1024 zones.
- Four cached pages and up to four outstanding requests per voice; global request
  capacity is `N * 4` and response capacity is `N * 2`.
- At most `N * 2` response pages are drained per process call. Catch-up and
  interpolation work are bounded. Read-ahead retries queue backpressure.
- The caller's `max_pcm_bytes` covers sample heads, voice/response caches and
  page scratch accounting. It excludes decoder buffers, thread stacks and small
  metadata allocations; it is not an overall process-memory limit.
- `DiskStats` exposes underrun frames (per voice), request overflows, I/O errors,
  pages read and stale responses. Missing interpolation data silences that voice
  frame while advancing its source position. Other voices continue normally.
- `wait_ready` waits for queued responses, not a promise that a particular seek
  or every voice is ready. Drain/prime and check underruns as appropriate.

Retain and drop `DiskHandle` on the control thread. Its destructor stops and joins
the worker, so it must never run in a callback. Stop rendering before teardown.
Files must remain stable. Runtime decode errors are counted and return silent
pages; automatic file replacement/reopening after an I/O failure is not supplied.
This release does not include FLAC/MP3 streaming, compressed sample banks,
time-stretching, or a seamless-playback guarantee for arbitrary disk stalls or
extreme transposition rates.
