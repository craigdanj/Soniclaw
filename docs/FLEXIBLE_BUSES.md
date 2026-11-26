# Flexible audio buses (0.18)

Soniclaw gains named mono/stereo/multichannel layouts, explicit auxiliary/sidechain channel routing, exact layout negotiation and scoped multi-port CLAP hosting. The existing simple stereo rack remains available; multi-port processing uses the new bus API rather than silently flattening sidechains into main audio.

## Workflow and executable demonstration

For an application, describe the bus layout, negotiate it while stopped, prepare channel routes, then process borrowed buffers. For a hosted CLAP product, inspect audio ports, optionally select an advertised configuration, and acquire a scoped bus-audio lease.

```sh
cargo build -p soniclaw-reference -p soniclaw-sdk-tools
cargo run -p soniclaw-sdk-tools -- --bus-demo \
  target/debug/libsoniclaw_reference.so /tmp/new-bus-demo
cargo test -p soniclaw-bus
```

The demo loads the actual exported Sidechain Ducker (`org.soniclaw.ducker`), supplies separate main/key inputs, checks gain reduction and release, and verifies invalid buffers are silenced. It measures callback allocation count for the tested path. It also routes mono input to stereo and a separate auxiliary output with the reusable patch bay. Layout JSON/report output is written to a new directory. This is synthetic hosted processing, not physical multichannel hardware acceptance. Platform library filenames differ.

## Named layouts and explicit routes

`soniclaw::bus::flex::{Bus, Role, Layout, Route, PatchBay}` is the safe reusable contract. A Bus has stable string `id`, display name, channels and Role (Main/Auxiliary/Sidechain). Layout has ordered input/output vectors. IDs are unique within each direction; matching input/output IDs are allowed. Role labels describe intent and do not mix, duplicate or activate any channels implicitly.

```rust
use soniclaw::bus::flex::*;
let layout = Layout {
    inputs: vec![Bus { id: "mic".into(), name: "Microphone".into(), channels: 1, role: Role::Main }],
    outputs: vec![Bus { id: "main".into(), name: "Speakers".into(), channels: 2, role: Role::Main }],
};
let routes = vec![
    Route { input_bus: 0, input_channel: 0, output_bus: 0, output_channel: 0, gain: 1.0 },
    Route { input_bus: 0, input_channel: 0, output_bus: 0, output_channel: 1, gain: 1.0 },
];
let mut patch = PatchBay::new(layout, routes, 256)?;
let mut stereo = [0.0; 4];
patch.process(&[&[0.2, 0.4]], &mut [&mut stereo])?;
assert_eq!(stereo, [0.2, 0.2, 0.4, 0.4]);
# Ok::<(), soniclaw::core::Error>(())
```

`Layout::negotiate(supported)` returns the index of an exact ordered ID/channel/role match after validation; display names do not affect matching. It never chooses an implicit downmix or discards a sidechain. Configure alternatives explicitly. `validate_buffers` checks buffer counts, complete interleaved frames and preparation maximum. `PatchBay::new` validates and stores numeric routes off-thread. `process` clears outputs and sums only those routes; unconnected outputs remain silent and sidechain signals only reach main if the application explicitly routes them there. Fan-out and channel-level sums are supported. There is no gain smoothing or clipping/limiting built into this utility.

Bounds: up to eight buses and 64 aggregate channels **per direction**; at least one output; each bus 1–64 channels; IDs 1–128 bytes; names ≤256 bytes; up to 4,096 routes; finite gains with absolute value ≤16; maximum 8,192 frames. Invalid prepared routes fail construction. Invalid process buffers clear every supplied output before returning an error. The patch bay uses borrowed interleaved buffers and performs no callback allocation or string lookup. Bus role/layout edits require a new prepared patch bay.

## Hosted CLAP buses

`ClapInstance::audio_ports()` returns cached ordered input/output Port metadata (native ID, name, channels, main flag). `bus_layout()` converts it to the common named Layout; native IDs become strings and non-main ports are Auxiliary. Applications explicitly mark known key inputs Sidechain rather than guessing from a vendor's display name.

`audio_configurations()` queries the optional `clap.audio-ports-config` extension while deactivated. `select_audio_configuration(id)` validates the requested advertised ID, asks the plug-in to select it, then refreshes ports/parameters. Missing extension yields an empty list; an unknown/refused configuration is an error. Configuration selection may change parameter/port identities; reconstruct saved routing and validate it before activation. No configuration selection runs on a callback.

```rust,ignore
let (inputs, outputs) = instance.audio_ports();
println!("{inputs:?} -> {outputs:?}");
instance.with_bus_audio(48_000.0, 256, |audio| {
    let main = [0.5; 512];
    let key = [1.0; 512];
    let mut output = [0.0; 512];
    audio.process(&[&main, &key], &mut [&mut output], &[], &[], transport)?;
    Ok::<_, soniclaw::core::Error>(())
})??;
```

`with_bus_audio(rate, max_frames, closure)` activates once, moves an exclusive BusAudio lease into a separate scoped audio OS thread, starts processing, runs the closure, stops, joins and deactivates. The instance/library/context remain alive until join. State/editor/main-thread operations cannot be performed concurrently through this API. It is suitable for offline processing or application-controlled scoped audio integration; the workbench's asynchronous/direct **serial rack still uses its stereo contract**. There is no new multibus routing GUI or automatic device-to-hosted-bus adapter in this release.

BusAudio accepts one borrowed interleaved slice per input/output port in native order. It deinterleaves into prepared planar storage, calls the actual native process ABI with every audio port, then interleaves outputs. All ports use the same frame count. Bounds are 8 ports / 64 aggregate channels per direction and maximum 8,192 frames. At least one output is required. Unsupported stereo-rack layouts now give a clear instruction to use the bus API. Fixed-layout existing plug-ins and old mono/stereo host APIs retain their contracts.

Sample-offset MIDI/native-note and parameter events use the existing bounded event subset, with at most 1,024 native events after MIDI1 panic expansion. Inputs must be sorted; parameters must name writable known IDs with finite physical values in range. Native output events remain available through `output_events()`. Transport currently supplies block-start sample position/tempo/playing with a default 4/4 meter; full tempo-map scheduling is a separate release. Multiple note input buses, float64 audio and arbitrary mid-process layout changes are not implemented.

## Threading, persistence and recovery

Layout construction, configuration selection, native activation and storage allocation belong off-thread. The callback path uses bounded prepared storage, no locks/files or allocation in Soniclaw. A malicious/nonconforming native module can still violate those guarantees. Invalid input shapes/events or native process errors clear supplied outputs and return an Error. Nonfinite native output is rejected. A scoped Rust panic stops processing through the lease destructor, joins and returns an error; a native crash is not contained.

Named Layout/Route use serde and can be persisted in an application's versioned session. This release does not change ProjectState schema or automatically add these fields to old projects. Native CLAP port/configuration IDs are vendor identities; re-check them when a plug-in version changes. Never restore a route by display name alone. The earlier BusLayout/BusEngine/SidechainCompressor API remains source-compatible. A failed layout change happens while stopped; reselect a supported configuration or reconstruct the old instance before resuming.

The framework's own CLAP exports still expose their documented fixed stereo main/optional key layout; this release expands **hosting and reusable bus contracts**, not every format wrapper's exported dynamic-layout ABI. VST3/AUv2 wrapper acceptance, real DAW channel negotiation, physical multichannel interfaces and macOS/Windows runtime require separate target-machine evidence.

See [release notes](releases/v0.18.0.md), [verification](validation/v018/REPORT.md) and [desktop roadmap](DESKTOP_ROADMAP.md).
