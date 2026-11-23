# Audio format manager (0.19)

Soniclaw now has a common control-thread API for WAV, AIFF and FLAC, including encoding, bounded whole-file decoding, seekable chunk reads and metadata. `soniclaw-formats` dynamically loads the native **libsndfile** backend. Soniclaw supplies the Rust API/lifetime/validation integration; it does not implement the FLAC codec in Rust. The existing `soniclaw::io` WAV API remains portable and unchanged.

## Setup and workflow

Enable `soniclaw`'s `formats` Cargo feature, or depend directly on `soniclaw-formats`. The native workbench GUI enables this integration. A libsndfile runtime must be installed through your platform's normal dependency setup: Linux `libsndfile.so.1`, macOS `libsndfile.1.dylib`/`libsndfile.dylib`, or Windows `libsndfile-1.dll`/`sndfile.dll`. The source ZIP does not bundle these binaries. macOS/Windows runtime acceptance is pending.

1. Create a FormatManager on a control/loader thread and inspect `backend_version` / `supports`.
2. Open a file to inspect MediaInfo (format, channels, rate, frame count, seekability, supported text tags).
3. Read bounded chunks, or call `read_all` with an explicit decoded PCM budget. Seek in **frames**, not individual interleaved samples.
4. For export, create a new destination with format/encoding/rate/channels, write supported metadata before audio, write chunks and call `finish` to observe finalization errors.
5. For a project sample, use `soniclaw::read_audio_with_limit`; WAV uses the existing portable reader, other formats use this backend when enabled. Project asset loading and bundle validation now use this helper. The workbench's Assets import also accepts these formats. Some browser searches/previews still retain their earlier WAV-specific workflow.

```sh
cargo run -p soniclaw-sdk-tools --features format-tools -- \
  --format-demo /tmp/new-codec-demo
cargo test -p soniclaw-formats
```

The runtime demo writes/reopens WAV, AIFF and FLAC PCM24, verifies titles/frame counts, seeks to a known frame, compares decoded samples, checks memory/seek bounds and exercises the facade asset decoder. It writes its backend version and per-format results. The pure unit test does not pretend to validate native codecs on a machine lacking the runtime. See [verification](validation/v019/REPORT.md) for actual execution.

## Public API example

```rust,ignore
use soniclaw::formats::{FormatManager, AudioFormat, Encoding};
let formats = FormatManager::native_default()?;
let pcm = formats.read_all(std::path::Path::new("sample.flac"), 64 * 1024 * 1024)?;
let mut reader = formats.open(std::path::Path::new("sample.aiff"))?;
println!("{:?}", reader.info());
reader.seek(48_000)?;
let mut block = vec![0.0; 256 * reader.info().channels as usize];
let frames = reader.read_frames(&mut block)?;
let mut writer = formats.create(std::path::Path::new("new.flac"),
    AudioFormat::Flac, Encoding::Pcm24, pcm.sample_rate, pcm.channels)?;
writer.set_metadata("title", "My sound")?;
writer.write_frames(&pcm.samples)?;
let written_frames = writer.finish()?;
```

`AudioFormat` is Wav/Aiff/Flac. `Encoding` is Pcm16/Pcm24/Float32; combinations are queried through `supports`. FLAC does not accept Float32. WAVEX/RF64 inputs are classified as WAV when the backend supports them, while new WAV outputs use the ordinary WAV container. Other native backend formats are deliberately not exposed as supported by this first manager API.

`FormatManager::native_default` searches platform runtime names and returns a clear missing-runtime/symbol error. `unsafe from_library(path)` selects a specific trusted library implementing the stable libsndfile ABI; it must not point to untrusted code. Keep the manager for multiple operations to avoid repeated loads. `backend_version` reports the actual loaded runtime. `supports` validates range bounds and asks the backend about the exact format/encoding/rate/channel combination before creating a writer.

`Reader::info` is an immutable metadata view; callers cannot change its validated channel count. `read_frames` fills an interleaved f32 slice, zeroes unused/trailing output, returns frames read and advances `position`. EOF returns zero. `seek(frame)` checks support and the reported range, permits seeking to EOF, and updates position after a successful native seek. It does not resample. `read_all` checks decoded size before allocating and rejects truncation/length mismatch rather than returning a silently shortened asset.

Metadata keys are `title`, `artist`, `album`, `comment`, `date`, `genre`. A backend/container may refuse a tag; `set_metadata` returns that error. Metadata values are text, not arbitrary embedded artwork/chunks. Use a new versioned application schema for custom metadata. Unknown keys are rejected. Tag writing should happen before the first audio write because some containers restrict later changes.

## Limits, errors and lifecycle

Channels are 1–64 and rates 1–768,000 Hz, subject to backend format constraints. Read/write chunks are limited to 64 Mi interleaved samples. Whole-file decode respects the caller budget and an additional 512 MiB PCM ceiling. Size arithmetic is checked. Writer PCM16/24 input must be finite in [-1,1]; Float32 must be finite. Files are not resampled, normalized for loudness or automatically limited. Quantization is native PCM encoding; no separate dither API is supplied.

Reader shapes must contain complete frames. Invalid buffers or nonfinite/error output are cleared and rejected. Out-of-range/nonseekable requests fail before seeking. Existing output destinations are refused to avoid normal accidental overwrite. This path-based check is not an atomic whole-file transaction; applications needing transactional export should write a new temporary destination and rename after successful finish.

`finish` flushes and closes explicitly and reports errors; Drop closes best-effort and cannot report a late disk error. A failed write can leave a partial newly created file. Keep it for diagnostics or remove it deliberately; it is not presented as a completed export. File decoding, metadata, seeking, encoding, finalization and library unloading are blocking/allocating operations. Never use them in an audio callback. Perform chunk reads on a file worker and feed a bounded PCM queue for real-time streaming; this release adds seekable decoder primitives, not an automatic multi-codec real-time disk engine.

Managers can be cloned; each open Reader/Writer holds the backend alive until its native file closes. Reader/Writer raw native handles are not Send/Sync, so create/use/drop each on its owning loader/control thread. Independent handles can be used on separate workers by constructing them there. Native codec internals have their own memory use; the PCM budget is an output allocation limit, not a sandbox for malformed files or native allocations.

POSIX paths must be UTF-8 for this API. Windows uses libsndfile's wide-character open API. Native runtime discovery/loading and file error strings are platform-dependent. A missing codec runtime does not disable the old portable WAV reader. Core/offline Soniclaw users who do not enable formats retain their previous dependency behavior.

## Persistence, packaging and validation

MediaInfo is serializable descriptive metadata, not a replacement for the audio file. No ProjectState schema migration is needed: assets still have paths/IDs, and bundles preserve original file bytes/checksums. Older builds without the format backend cannot decode newly referenced AIFF/FLAC assets; export PCM WAV explicitly for those targets. The SFZ subset importer uses the common bounded decoder when this feature is enabled; all its other scope/path restrictions remain.

The backend is libsndfile, whose upstream license is LGPL-2.1-or-later; consult the exact distribution's license/notices when packaging its binaries. This ZIP includes Soniclaw's Rust adapter, not a bundled codec distribution. Cargo license collection alone does not collect dynamically installed native runtime licenses. Record the backend version and native notices in your application's packaging manifest. Primary ABI reference: https://libsndfile.github.io/libsndfile/api.html ; source/license: https://github.com/libsndfile/libsndfile .

Local results establish the exercised backend's WAV/AIFF/FLAC round trips, seek/tag behavior and bounds. They do not establish codec equivalence on every runtime version, real-time deadlines, arbitrary file robustness, third-party plug-in acceptance or macOS/Windows execution. See [release notes](releases/v0.19.0.md), [verification](validation/v019/REPORT.md) and [desktop roadmap](DESKTOP_ROADMAP.md).
