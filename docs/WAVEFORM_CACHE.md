# Background waveform thumbnail cache

`soniclaw::waveform` / `soniclaw-waveform` streams audio into a multiresolution
peak pyramid, persists a digest-checked cache and exposes cancelable background
requests. The UI can display long recordings without retaining their full PCM.

## Runnable demo and native viewer

```sh
cargo run -p soniclaw-waveform-cache-demo --locked -- new-waveform-demo
cargo run -p soniclaw-ui-gallery --locked -- --audio new-waveform-demo/minute-stereo.wav --cache new-waveform-demo/cache --waveform-only
# Add --snapshot new-waveform-demo/overview.png for a real native framebuffer capture.
```

The demo streams a deterministic60second stereo float WAV, builds a thumbnail,
reopens its disk cache, checks a latest-request background load and reads a
small PCM detail range. It writes `report.json` with source/peak memory sizes,
levels, hit flags and peak columns. Its output directory must be new. The
viewer uses the same background loader and cached egui view; it opens no audio
device. Input file, cache directory and screenshot path are explicit.

## Background API and lifecycle

```rust,no_run
use soniclaw_waveform::{Loader,Options};
let mut loader=Loader::new(std::path::PathBuf::from("app-waveform-cache"))?;
let selected=loader.request(std::path::PathBuf::from("recording.wav"),Options::default())?;
// In the UI loop: progress is a cheap control-side snapshot; poll never decodes audio.
if let Some(done)=loader.poll(){
    if done.ticket==selected {
        let loaded=done.result?;
        println!("{} frames, cache hit {}",loaded.thumbnail.frames(),loaded.cache_hit);
    }
}
loader.cancel();
# Ok::<(),Box<dyn std::error::Error>>(())
```

A loader owns one worker, at most one active job, one replaceable pending
request and one replaceable result. A new request cancels older work by ticket;
only the current ticket can publish. `cancel` clears pending/result/progress and
suppresses the old completion. Background errors arrive with the completion
rather than executing application callbacks on the worker. Source/cache paths
are made absolute when requests/loaders are constructed, preserving meaning if
the application later changes its working directory.

`progress` reports hashing/decoding/verification/completion, done and total
counts. Counts are bytes for fingerprint passes and frames for decoding. Poll
and request use short control-side mutex sections; native file/codec work runs
outside that lock. Drop signals the worker while holding its condition mutex,
wakes it and joins. Cancellation is checked between I/O chunks/pyramid levels;
it cannot interrupt an already blocked operating-system read. Drop can therefore
wait for I/O and belongs on a control/shutdown thread, never an audio callback.

## Peaks, zoom and detail

`Thumbnail` is immutable and shared as `Arc`. Accessors expose rate, channels,
frames, base bucket, levels, source SHA256 and peak memory. Base buckets default
to256 frames; higher levels combine adjacent peak pairs independently per
channel. Partial final buckets preserve their actual extrema. Empty audio has a
valid empty pyramid and queries as silence.

`query(channel,start_frame,end_frame,&mut[Peak])` fills up to8192 visible columns
without file access or allocation. It chooses a useful pyramid level and
includes whole buckets overlapping a pixel. This is a conservative overview:
zooming below the base bucket does not recover sample detail. `read_detail`
provides a bounded seek/read for such zooms on a background/control thread.

`ui::ThumbnailView::show(ui,&thumbnail,channel)` reuses peak scratch storage,
provides zoom and drag-to-pan, and returns `ThumbnailResponse` containing the
egui response, visible frame range and `detail_recommended` flag. An application
can schedule `read_detail` when that flag is true and use its normal PCM view.
The thumbnail widget itself never opens files or decodes samples. It resets
navigation for a different source fingerprint and clamps offsets to valid
frames. Initial application-specified zoom/offset is honored.

## Codecs, disk cache and invalidation

The default backend streams PCM8/16/24/32 or float32 RIFF WAV through `hound` on
all supported targets without a dynamically installed codec library. Enable
`soniclaw-waveform/formats` (or facade `formats`) and select `Backend::Native`
for the existing seekable libsndfile codec manager, including its documented
WAV/AIFF/FLAC support. That optional backend requires the native codec library;
missing/unsupported codecs return a recoverable decode error.

Each request fingerprints all source bytes, so content changes invalidate even
when size and timestamp are unchanged. Cache hits still require this read pass;
reopening a very large file is not constant-time. A new build verifies the
source fingerprint again after decoding and rejects a changed file. Keep source
content stable during analysis; this is not a filesystem snapshot/locking API.

Cache keys include source digest, schema, base bucket and decoder backend.
Identical file bytes can share the same cache across paths. `.swave` files use
fixed little-endian schema1 headers, per-level bounds and a SHA256 payload
trailer. Reads validate length, channels, spans, counts, finite extrema, complete
pyramid and digest before accepting data. Corrupt caches are rebuilt from source
and marked `rebuilt_corrupt_cache`. New cache files use unique temporary files,
sync and atomic rename; failed/cancelled writes remove their temporary file.
Original audio is never rewritten.

`prune(cache_directory,max_bytes)` is an explicit off-audio eviction policy. It
removes oldest canonical64-hex`.swave` files until owned-cache bytes fit the
budget, preserving unrelated filenames/symlinks and in-progress `.tmp` writes.
Use an application-owned cache folder. Loaded thumbnails remain valid in RAM;
evicted data can be rebuilt. There is no hidden global cache, eviction timer or
unbounded worker pool. The application owns in-memory thumbnail lifetimes.

## Bounds, persistence and errors

Base bucket must be a power of two64–8192, channels1–64, at most32 pyramid levels
and8Mi peak pairs (64MiB peak payload). File/cache buffers and serialized peak
copies can increase peak memory during validation/write; they remain bounded
by the cache payload plus fixed decode buffers. Increase the base bucket for a
recording that exceeds the budget. Query widths cap8192; detail reads cap65536
frames and1Mi samples. Invalid ranges/channels clear query output and return an
error. Nonfinite source samples and partial/truncated frames fail analysis.

`Error` distinguishes cancellation, invalid bounds, decode errors and filesystem
I/O. Handle an error on the UI side by retaining a prior thumbnail or showing an
appropriate file error; retry with a new request after correcting the source.
Corrupt-cache repair never bypasses source validation. Cache keys/files are
reproducible acceleration data, not project state. Persist asset identity and
optional view preferences in the application's document; do not serialize
worker threads, tickets or texture/UI scratch state. Rebuild caches when the
schema/backend/base resolution changes. Existing project schemas are unchanged.

## Actual validation

`cargo test -p soniclaw-waveform --all-features --locked` checks multichannel
extrema, partial buckets, cache hits and corruption repair, same-size/timestamp
content changes, empty files, hostile counts, cancellation/latest-result
publication, idle worker shutdown and scoped eviction. The optional native codec
test is explicitly ignored in the general suite; run it on a configured machine:

```sh
cargo test -p soniclaw-waveform --all-features native_formats_backend -- --ignored
```

The release report records the executed60second streaming/cache/detail demo,
its memory reduction, Linux native codec check and actual cached native viewer
capture. No hardware or external DAW acceptance is implied. Full source hashing
and peak analysis are background/control work, not real-time DSP.
