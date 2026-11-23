# Native mixdown and integrated DAW delivery — 0.85.0

Studio's Export centre can now render trusted native CLAP/VST3 sessions offline and capture routed track, submix and auxiliary stems in the same pass as the full mix. The accompanying example connects recording, editable MIDI, recovery, project generations, routing, cues, native processing and delivery through public APIs.

## Desktop workflow

Stop to capture native state, disable arrangement looping and live track inputs, open **Export centre**, and choose a new output folder, frame range and tail. Select **Full mix** and any **Same-pass routed stems**. Continue using the existing WAV/AIFF/FLAC formats, output-rate conversion, peak/LUFS normalization and integer dither controls. Available codecs still depend on the installed native format backend.

Native exports use the explicitly approved module paths from this Studio session. Project metadata and a cache do not grant permission. Each export worker constructs its own trust capability and native owners from the stopped project snapshot. It restores verified state and host parameters before processing. Missing, changed or unapproved modules fail with a native error; bypass or repair them before retrying. A plug-in that lacks opaque state requires an explicit parameter-only checkpoint, as described in the plug-in guide.

Progress remains stage-local, and Cancel is checked between bounded host operations. Output goes to an exclusively created folder; the report is written last. A render, codec, stale-media, native or cancellation error removes the new output folder. Existing folders are rejected. A native plug-in can block indefinitely inside its own code; this offline worker does not provide process isolation or a forced native-call timeout.

## What a routed stem contains

| Source | Captured signal | Compensation |
|---|---|---|
| `TrackPost(id)` | Track instrument/audio and inserts, after strip gain/pan/mute/solo | Common aligned track latency |
| `Bus(id)` | Full routed bus input, bus inserts and bus level/pan/mute/solo | That bus's compiled latency |
| `Aux(index)` | Shared auxiliary effect return and return gain | Common aligned auxiliary latency |
| Full mix | Entire graph including master inserts and master controls | Compiled master latency |

Routed stems are captured before master inserts. Detectors, other tracks and shared returns continue running in their full session context. No track is soloed and re-rendered to obtain these files. Native nonlinear master processing is applied to the full mix once. A stem selection that overlaps nested buses or includes both a track and its bus is not a disjoint mix decomposition; those files need not sum to the mix. Each selected source is trimmed by its own latency, so time zero refers to the requested source range.

Legacy `Settings.stems` retains its pre-mixer per-track freeze semantics for stock/custom projects. The native export helper rejects that isolated legacy mode when active native descriptors are present: use `routed_stems` for native sessions. The GUI labels both choices explicitly. Tail length is explicit, not inferred from native tail claims. Native MIDI output is not written as a MIDI file by the audio export.

## Native ownership and processing API

`soniclaw_host::studio::Mode::{Live,Offline}` is now explicit in `NativeFactory` struct literals. Live preserves the musical VST3 worker bridge. Offline uses `soniclaw_vst3::musical_offline::OfflineProcessor` and has only the native processor's latency, without the 1024-frame live bridge. CLAP retains direct continuous processing and prepared parameter events. Existing `Instance::with_offline` and `process_offline` APIs remain available with their documented contracts.

`Instance::start_offline_musical` creates a continuous offline lease: setup and `start_processing` happen once, blocks carry prepared timeline context and note identities, and stopping the owner ends processing. Tempo, meter/bar/loop context, MIDI 1 notes, explicit note identities and supported expression are forwarded across blocks. MIDI voice identity capacity is 128; event and normalized-change ingress is 1024 per submitted block. Values and IDs are validated. Structural native reconfiguration aborts the render for reconstruction.

The lease checks the constructing thread before native processing and uses a weak reference to the owner. An obsolete or moved lease clears output and reports an error; it cannot extend native object destruction onto another thread. It may be `Send` to satisfy `AudioProcessor`, but this API is for blocking offline work on its original worker, never device callbacks. Native processing can lock or allocate. Prepared musical time and event bounds do not establish native real-time safety.

`capture_output_events(true)` enables a bounded 1024-event offline output buffer; drain it with `take_output_events` between blocks. Its sample timestamps are absolute. Overflow reports an error. The default audio-render path discards native output MIDI explicitly. `Instance::offline_errors` and the native automation failure counters are checked by `studio::render_export` before accepting the result.

`studio::render_export(project, settings, new_directory, loader, context)` constructs owners and retires them on the same worker. Supply a loader constructed there from explicitly approved native modules. Stop and snapshot live owners before starting the worker; never move live GUI owners into it. The desktop's native freeze path also chooses Offline mode. Direct use of the generic `studio::export::render` still requires an appropriately owned factory; a live buffered native factory is unsuitable for a fast offline loop.

## Routed-tap API, persistence and bounds

`studio::delivery::{Source,Tap}` and `OfflineSession::routed_taps` expose prepared stereo publication banks. Taps are disabled by default; enable selected taps before rendering, then read their banks after each `render_block`. The processor owns sub-block origins, so automation subdivisions publish a complete contiguous outer block. Banks are atomic exchanges for sequenced write-then-read use, not coherent concurrent snapshots.

At most 32 export files (mix, legacy and routed stems combined) can be selected; duplicate/unknown routed sources reject settings. The graph prepares at most 32 track + 8 bus + 4 auxiliary stereo banks, up to the project's 8192-frame block bound (about 2.75 MiB maximum sample storage). Disabled taps skip publication. Existing graph compensation, native state, project media, export duration/RIFF and codec limits continue to apply. Native code can allocate its own additional memory beyond host bounds.

`Settings.routed_stems` is optional and defaults empty for older schema-1 export settings. Projects retain their existing outer schema, native resource format, stable processor identities and parameter IDs. No native binary is bundled in projects. The new explicit factory mode is a pre-1.0 Rust source API change. File reports retain the existing role/normalization fields; routed role filenames include their source type and stable ID/index. Mix and stems retain the session's two-channel layout; cue hardware feeds are monitoring signals, not additional export files.

## Integrated runnable example

```sh
python3 tools/build-formats.py --fetch --no-editor --protocol-fixture
cargo run -p soniclaw-daw-delivery -- NEW_FOLDER build/formats/fixtures/HostProtocolFixture.vst3
```

Use the owned protocol fixture only: its gain ID is known, and the example never selects its fault-injection control. It starts with a blank project, records two virtual audio tracks with distinct mono/stereo channels, places the take, records and recovers MIDI/controller data, adds a stock instrument, creates submix/sidechain/cue routing, checkpoints native state, saves two generations, removes original raw take folders, reopens, and renders a mix plus five routed stems. It checks frame counts and the expected relationship between the pre-master bus/return and the native gain master. Separate native calls verify a note identity and expression survive across blocks, and an obsolete lease fails safely after owner destruction.

This is executed software integration on Linux, with no physical device. See [verification](validation/v0_85_0/REPORT.md) for exact commands, evidence and the full workspace test result. Physical recording/playback, native editor interaction, third-party product compatibility, Windows/macOS runtime and release-installation acceptance remain pending. The three requested workflow milestones have bounded framework/reference-application implementations; this does not establish commercial DAW completeness or JUCE feature parity.

## State and native-editor boundaries

Selected native host parameters are authoritative when snapshotting: the last host scalar is retained even if an update had not reached the next callback. Native-editor parameter feedback remains informational and does not automatically become a Touch/Latch write gesture. Set selected values through the live host faders when those values must be checkpointed; native editor changes to selected parameters can be superseded by the saved host values. Opaque hidden state is still captured where supported. Native editor gesture reconciliation remains future work.

As of 0.85.0, path repair preserves the selected native automation IDs and requires affected tracks to be unfrozen first. Native resource aggregate sizes and parameter counts are checked before loading their raw files. Custom descriptors can continue to use an unrelated `native_state` key without being treated as native plug-ins.
