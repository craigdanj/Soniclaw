# VST3 hosting and common product controls

Version: 0.23.0. Enable `soniclaw` feature `vst3-hosting`, or depend on `soniclaw-vst3`. This adds native VST3 **hosting** alongside the existing VST3 export tools. It is a stereo desktop host foundation, with explicit limits below.

## Workflow and runnable examples

The workbench VST3 tab accepts a trusted `.vst3` bundle, inspects its factory, lets you select a specific product/class, starts/stops worker-backed audio, edits normalized parameters, opens/closes a native editor and saves/loads stopped plug-in state. The Audio tab still selects devices. Stop captures a state snapshot before releasing the owner. Select a matching class when loading a state file. Opening an editor/controls creates a control owner without requiring an audio device; starting audio reconstructs that product from its saved state.

To build a native reference bundle and exercise it on Linux:

```sh
# Requires Rust, CMake/C++17 and the native dependencies in README.md and docs/FORMATS.md.
python3 tools/build-formats.py --fetch --jobs 2
cargo build -p soniclaw-sdk-tools --features vst3-tools
python3 tools/probe-vst3.py \
  build/formats/Release/SoniclawReference.vst3 /tmp/vst3-probe.json
# Output directories must be new.
target/debug/soniclaw-sdk-tools --vst3-demo \
  build/formats/Release/SoniclawReference.vst3 /tmp/vst3-check
# Requires an X11 display, also supported under Xvfb/software GL:
target/debug/soniclaw-sdk-tools --vst3-editor-demo \
  build/formats/Release/SoniclawReference.vst3 /tmp/vst3-editor-check
cargo run -p soniclaw-workbench -- --tab 13
```

The export build uses the separately pinned CLAP-wrapper/VST3 SDK sources in `formats/dependencies.json`; no system plug-in folder is modified. Bundles and dependencies are not included as prebuilt binaries in the source archive. For a third-party plug-in, use its actual bundle/class rather than the framework-specific assertion demo.

## Common product/control API

`soniclaw::host::formats` exposes `PluginFormat::{Clap,Vst3}`, `Product`, `products(format,path)` and `Instance::load(product,spec)`. Discovery and loading are unsafe because they execute selected native code, even when their purpose is only inspection. Pick trusted files. The common owner returns `NormalizedParameter` metadata, accepts `set_normalized(id,value)` and reports latency. Variant access supplies the backend-specific processing, bus, state and editor APIs.

CLAP normalization in this common surface is linear interpolation between its advertised min/max. VST3 normalized values are native 0–1 values. Do not use that mapping as an engineering-unit conversion or assume the two formats expose matching parameter IDs or curves. Persist stable IDs plus format/product identity. Common CLAP parameter updates use the stopped-instance restore API; live CLAP rack updates keep their existing API.

```rust,no_run
use soniclaw::{host::formats::{self, PluginFormat}, prelude::*};
let spec=ProcessSpec{sample_rate:48_000.0,channels:2,max_block_frames:256};
let path=std::path::Path::new("/trusted/MyEffect.vst3");
// SAFETY: this application explicitly trusts this native module.
let products=unsafe { formats::products(PluginFormat::Vst3,path) }?;
let product=products.first().ok_or("no products")?;
let owner=unsafe { formats::Instance::load(product,spec) }?;
for p in owner.parameters()? { println!("{} = {}",p.name,p.value); }
# Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
```

## Native VST3 owner

`soniclaw::vst3::Instance::load(path,class_id,spec)` selects a 32-hex-character class ID, or the first product if omitted. Specs require stereo, a validated sample rate and a maximum of 1–8192 frames. Supported products have exactly one stereo output and zero or one stereo input. Additional buses are rejected explicitly; this convenience adapter does not flatten or silently drop sidechains. The lower-level CLAP multi-bus API remains available independently.

`info()`, `parameters()`, `set_parameter()`, `latency_frames()`, `stats()` and `drain_output_events()` serve the control side. Parameter values must be finite 0–1 and IDs writable. Stopped changes are committed with a zero-sample native process/flush cycle so a subsequent opaque state snapshot captures the new processor values. A plug-in which rejects that cycle returns an error. Live changes are queued for the next worker block. Native GUI edits also flow through the backend's processor parameter queue. Poll parameter metadata again when your UI needs fresh values; the workbench's slider list is refreshed when creating an owner, not a full automation recorder.

`process_offline(interleaved,changes,position,bpm)` is a blocking, allocating, isolated-block convenience call. Changes are sorted `(sample_offset, parameter_id, normalized_value)` tuples, maximum 128, with offsets inside the block. Blocks are 1–prepared maximum frames. It starts and stops native processing for each call; it is unsuitable for preserving sustained instruments/tails across a series of blocks. Use `start_buffered()` for continuous processing. Parameter smoothing and a plug-in's handling of multiple points remain its own behavior. The bundled CLAP-wrapper reference consumes the last point per parameter per block; the host does not claim to change that plug-in behavior.

## Device bridge, timing and MIDI

`start_buffered()` returns a Soniclaw `AudioProcessor`. Prepare it with the same sample rate and stereo layout. It adapts arbitrary callback lengths into fixed 256-frame packets through two capacity-eight SPSC queues. Native calls and all native/backend mutexes run on a worker/control thread. The callback uses fixed storage, bounded queue operations and atomics; it does not load modules, allocate buffers, lock a mutex or call the VST3 processor.

Output is scheduled four packets (1024 frames) after its input. At 48 kHz this bridge adds about 21.33 ms, before device latency and plug-in latency. Missing/late output is silent and stale packets are discarded. `latency_frames()` adds the native reported latency; counters expose underruns, input/output drops, native errors and restart notifications. This initial VST3 integration offers buffered hosting only. Do not infer a low-latency/direct VST3 path from the earlier CLAP direct mode.

The packet carries core MIDI 1 note on/off, CC, pitch bend, channel pressure, poly pressure and AllNotesOff/panic, with sample offsets. Each packet holds 128 events. Overflow or an input packet drop requests worker-side MIDI panic; unsupported note-ID/expression messages increment error counters. MIDI controller behavior depends on the plug-in's VST3 mapping. Program change, SysEx, note-expression/MPE translation and a sample-clock-scheduled output-port bridge are not implemented here. `drain_output_events()` exposes bounded backend-captured native output events on the control thread; it does not automatically route them to hardware.

Transport sends playing/sample position and tempo. PPQ in the live adapter is derived from sample position at the current constant tempo; it does not carry the full v0.21 tempo-map/meter/loop model into VST3. The offline API accepts an explicit quarter-note position. These limits are separate from the richer existing CLAP transport API.

## GUI, restart handling and lifetime

The owner is deliberately `!Send`/`!Sync` and stays on the GUI/main thread. `open_editor()`, `close_editor()`, `editor_open()` and regular `poll()` service native windows and VST3 run-loop callbacks. Native parent ownership comes from the backend's platform window adapter. This release exercised Linux XCB/X11. Other platform code being present is not runtime acceptance.

The worker serializes native processing with control calls through a mutex; a slow plug-in/UI can therefore cause worker underruns, while the device callback stays nonblocking. This is not process isolation. Dropping/stopping the owner sets its revocation flag, joins the worker, stops native processing and closes the editor before destroying the module on the owner thread. A retained callback processor becomes silent. Each new run has an independent flag, so restarting cannot revive an old callback.

`poll()` services restart requests off the device callback. An actual I/O-layout/latency change or reload request during a run revokes the processor and returns an error requiring stop/reconstruction. Notification-only updates with unchanged layout/latency are counted and accepted. Rebuild rather than continuing with stale delay compensation. Native errors produce silent packets and diagnostic counts; stop/recreate the owner to recover. No bound can guarantee completion if trusted native code hangs inside a call or destructor.

## State, bounds and migration

VST3 `State` schema 1 records bundle path, class ID, product version, opaque bytes and SHA256. States are limited to 4 MiB; JSON input is limited before deserialization. `State::read`, `from_json`, `to_json` and `write_atomic` validate shape and digest. Atomic replacement uses a uniquely created same-directory temporary file, syncs it and renames it. Replacement behavior is verified on Linux; a failed rename retains the previous destination and returns an error.

`save_state()` and `restore_state()` require the buffered worker to be stopped. Snapshot flushes pending stopped edits with a zero-sample cycle. Restore checks the class identity and checksum before calling native code. A version difference is not proof of state compatibility; inspect recorded `version` and apply your application's upgrade policy. A failure leaves the owner available for error reporting/reload, but third-party state restoration is not transactional. Never load the saved path automatically from untrusted JSON.

These states are independent of CLAP rack `Session` schema 1: that document has not been silently converted to a mixed-format session. Existing CLAP sessions remain readable. A unified mixed-format rack/session graph is future work. Workspace ring buffers advance from rtrb 0.3.2 to pinned 0.3.5 to match the hosting backend; existing bridge regressions are rerun.

## Backend source and attribution

Native ABI hosting uses MIT `vst3-host` 0.11.1, copyright 2026 Helge Sverre, checked in under `third-party/vst3-host` with its original license. Default CPAL and process-isolation features are disabled; Soniclaw supplies its own callback bridge. See [integration patch notes](../third-party/vst3-host/SONICLAW_PATCHES.md). The local integration fixes latency-only restart handling to avoid needless deactivate/reactivate cycles and queries latency while active. This prevents repeated notifications and instrument resets with the reference adapter. It is not an independently rewritten VST3 ABI engine.

`tools/probe-vst3.py` runs inspection in a child with an explicit timeout (0.1–120 seconds), captures exit/error metadata and contains crashes/hangs during scanning. It is not a security sandbox and does not isolate live processing. Its default timeout is 10 seconds. Direct `probe()` remains in process.

## Validation and scope

See the [release report](validation/v023/REPORT.md) for executed commands, native product/class enumeration, sample-offset gain, opaque-state round trip, zero-allocation callback instrumentation, fixed bridge onset, retained-owner revocation, sustained drum MIDI and native editor reopen/rendering. GUI evidence comes from Xvfb/software GL. No physical device, commercial VST3 collection, real DAW session, Windows or macOS runtime acceptance is implied. This expands the ecosystem; it does not establish JUCE feature parity.
