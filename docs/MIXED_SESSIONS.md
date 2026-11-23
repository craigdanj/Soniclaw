# Unified native, CLAP and VST3 graphs and sessions

Version 0.25.0. Enable `soniclaw/vst3-hosting`, then use `soniclaw::hosted`. A schema-1 mixed document now reconstructs native project processors and foreign plug-ins into the existing compensated acyclic graph. This is a reusable framework API and SDK demonstration; the earlier workbench CLAP/VST3 panels remain separate.

## Workflow and runnable demonstration

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-host-fixture --features soniclaw-sdk-tools/vst3-tools
python3 tools/build-formats.py --fetch
# Choose a NEW destination:
target/debug/soniclaw-sdk-tools --mixed-demo \
  target/debug/libsoniclaw_host_fixture.so \
  build/formats/Release/SoniclawReference.vst3 /tmp/mixed-graph
```

The demonstration runs native gain 0.5 → independent CLAP fixture gain 0.5 → VST3 gain 0.4 on an actual audio worker thread, while the owner pumps GUI-thread services. Output should be 0.1 after the VST3 bridge delay. It captures opaque state, reloads the graph, checks latency-aligned VST3 bypass output at 0.25 and preserves a missing CLAP slot. JSON, WAV and diagnostic evidence are written to the destination.

## Document and public API

`Session` contains schema/name, up to eight `Node`s, up to 64 `Connection`s, input gains and output gains. IDs are stable 1–64-character ASCII alphanumeric/period/underscore/hyphen strings. Graphs reject cycles, duplicates, unknown endpoints and nonfinite gains before loading any module. Connections are whole stereo buses in this release, not arbitrary per-port channel routes.

Each node has a `Source`:

- `Native { project, root }`: an existing schema-2 `ProjectState` and asset root.
- `Clap { slot }`: the existing opaque-state/parameter/editor `Slot` payload.
- `Vst3 { product, state, parameters, editor }`: explicit factory class/path, optional schema-1 opaque state, normalized writable parameters and editor preferences.

Source JSON is externally tagged (`native`, `clap`, `vst3`). This keeps numeric parameter-map IDs round-trippable through serde without losing their key representation. Do not substitute a different tagged representation in saved documents. Boxed Rust fields are transparent in JSON.

`Session::validate`, `from_json`, `to_json`, `read` and `write` operate without executing modules. JSON input/output is bounded to 96 MiB; component state retains its own earlier limits. File writes create a unique same-directory temporary file, sync and rename it. A rename failure returns an error and retains the previous destination; Windows replacement behavior is not runtime-verified.

## Explicit native-code authorization

Parsing a session does not authorize its paths. Construct `host::formats::TrustedLoader` with an explicit list of trusted existing module files/bundles, on the GUI thread. Its constructor is unsafe because native code and dependencies must remain trusted, ABI-conforming and unmodified. It is bound to that thread and checks canonical paths before loading. `TrustedLoader::default()` permits no foreign modules and is useful for pure native graphs.

```rust,ignore
let document=soniclaw::hosted::Session::read(session_path)?;
// SAFETY: the application/user explicitly selected these trusted native modules.
let loader=unsafe { soniclaw::host::formats::TrustedLoader::new(&trusted_paths)? };
let (mut owner,mut processor)=document.build(&loader,spec,MissingPolicy::Placeholder)?;
processor.prepare(spec)?;
// Give processor to the device callback; keep owner on this GUI thread.
owner.poll()?;
let saved=owner.capture()?; // revokes/stops processing before state capture
saved.write(session_path)?;
```

The built-in `build` uses product identity `org.soniclaw.mixed.native` for native documents. `build_with` accepts a control-thread factory returning your `PluginInstance`, allowing your own product identity and registered DSP factories. No unsafe code is added to the facade; native loading is confined to the existing FFI crates and authorized through the explicit capability.

## Runtime ownership and controls

`Owner` retains foreign module/control lifetimes; `Processor` owns the prepared graph and implements `AudioProcessor`. `prepare` accepts the same rate/channels and a maximum no larger than the prepared graph. Audio must run on the audio thread, separately from the creating GUI thread; the independent CLAP fixture checks this distinction. `Owner::poll` services both host backends. Applications must keep calling it while playing.

`set_parameter(node,id,normalized)` dispatches native atomic controls or the selected host's control queue. Native parameter IDs use `plugin::parameter_id`. CLAP normalization is linear between its native advertised bounds. VST3 uses its own 0–1 normalized mapping. `set_bypass` is atomic, while each node's dry line continuously tracks its declared latency. Bypass is an immediate aligned switch, not a crossfade. Native DSP still runs while bypassed.

`diagnostics()` returns per-node latency, bypass, missing status/native diagnostics or native parameter values. `open_editor(node)` opens a Linux embedded CLAP editor or the VST3 platform window; native nodes use the application's parameter UI. Remembered open preferences are attempted on reconstruction, with failures retained as warnings. VST3 size/scale metadata is preserved but not yet fully applied here.

CLAP nodes use direct processing; VST3 nodes use the existing buffered worker path (1024 bridge frames). The graph compensates parallel fan-in/output paths using each processor's declared latency. This is not a direct-mode VST3 graph yet. Stateful continuous offline rendering should not run a buffered graph as fast as possible; use a native/offline host path instead.

## Stop, capture and recovery

Stopping first revokes a global gate and waits off the callback for an in-flight graph call. It then stops native endpoints before reading their states. A retained graph processor becomes silent. Capture updates native parameter documents, CLAP opaque state/parameter maps, VST3 opaque state/parameter fallback, editor-open settings, outer bypass flags and saved latencies. DSP delay/voice history is not serialized. Capture disrupts playback; stop the device, drop the old owner/processor and rebuild to resume. `reset` can revoke underlying endpoints; reconstruct rather than expecting an in-place native restart.

`MissingPolicy::Fail` returns construction errors. `Placeholder` preserves the entire original node payload and connections, records a warning and inserts a silent processor with the saved latency. Turning its outer bypass on explicitly gives aligned dry passthrough. No native state is discarded merely because a product is missing. Resolve the path/product, extend explicit trust if needed and rebuild. A native crash/hang is not contained by this in-process layer.

## Persistence and migration

The older CLAP `Session` and VST3 `State` remain unchanged. `Session::from_clap` converts a serial CLAP document without executing it, preserving original slot identities and states inside node payloads. It generates graph IDs `slot-0`, `slot-1`, etc., preserves bypass and connects the serial chain. Native state continues using its earlier project schema and asset-resolution rules. VST3 snapshots verify class identity/checksum before restoring. Saved product versions inform application migration policy; they do not guarantee vendor state compatibility.

## Validation and scope

Tests cover native graph audio, allocation-free processing, owner revocation, invalid/cyclic routing and numeric parameter-ID persistence. The native three-format demonstration checks real processing, opaque-state reload, aligned bypass and missing-slot preservation. See the [release report](validation/v025/REPORT.md). Physical device, external DAW, broad third-party plug-in and Windows/macOS runtime acceptance remain pending. No claim of arbitrary multichannel ports, process isolation or seamless capture during audio playback is made.
