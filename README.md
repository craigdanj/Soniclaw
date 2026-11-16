# Soniclaw

Formerly Sonora. This is the rename-only update of v0.3.0: crates and imports now use `soniclaw`, and the CLI is `cargo soniclaw`. See [migration notes](docs/RENAMING.md) for compatibility and the historical validation data.

**A modular Rust framework for native audio applications, instruments and effects.**

See the [release notes](docs/releases/README.md) for each major and minor release,
or the [changelog](CHANGELOG.md) for the chronological change history.

Soniclaw connects processing, devices, routing, DSP, MIDI, recording, state, UI and project tools. **v0.3.0 extends all six requested ecosystem areas.** It is a developer release with executable code, tests and Linux format validation. It does not claim JUCE feature parity or verified runtime support on platforms that have not been tested. Project/package names remain provisional and crates are not published.

## Added in v0.3.0

| Area | Working addition |
|---|---|
| Performance | Settled gain specialization, cached low-pass coefficients, streamlined CLAP buffer conversion; frozen v0.2 baseline and reproducible before/after batch CPU measurements |
| Embedded editors | Shared egui controls inside a host-parented window, resize/scale/focus handling and bounded UI-to-host gestures with backpressure retention |
| Compatibility | Exported-ABI host for state, buses, real sidechain audio, automation, in-place/constant buffers, callback stress and heap instrumentation; optional real X11 mouse interaction |
| Effects | Parametric EQ, compressor, peak limiter, gate, chorus, Schroeder reverb, partitioned FFT convolution and 2× oversampled drive |
| Instruments | Immutable multisample banks, key/velocity zones, round robin, sustain, channel pitch bend and crossfaded stealing; separate bounded background WAV transport |
| Formats and tools | Statically wrapped VST3, macOS AUv2 recipe, app/effect/sampler templates, command-line checks, license collection, archive/installer staging, documentation and platform CI configuration |

**New examples:** `soniclaw-reference` exports Gain + Filter, Sidechain Ducker, Multisample Drums and Effects Rack. `soniclaw-reference-studio` hosts the same drum factory standalone, with a hardware-free render mode. See [formats](docs/FORMATS.md), [DSP](docs/DSP.md), [sampling/streaming](docs/SAMPLER.md), [release tools](docs/RELEASE.md) and the [validation report](docs/VALIDATION.md).

Linux CLAP/VST3 builds and embedded X11 interaction are checked in this release. Real DAW project round trips, audio-device loopback/unplug tests, native macOS/Windows runtime acceptance and AUv2 compilation remain pending. The AUv2/installer recipes and CI jobs are supplied, but unexecuted targets are not presented as verified products.

## Earlier additions in v0.2.0


| Addition | Implementation |
|---|---|
| Audio input and duplex processing | Native capture/playback, selected mono/stereo input channels, stereo output mapping, muted-by-default monitoring, bypass, bounded input bridge with prefill and adaptive clock-drift correction |
| Device/session management | Supported configuration enumeration, requested rate/buffer validation, device settings UI, JSON configuration, control-thread device-loss detection and retry |
| Recording | Dry input or processed output to stereo PCM16 WAV on a worker thread; bounded enqueue, counted loss, silence inserted for dropped frames, explicit finalization and protection against overwriting takes |
| Automation and buses | Sample-offset parameter events, stable numeric IDs, coherent same-frame groups, separate input/output and sidechain layouts, sidechain compressor, graph latency compensation, suspend/resume |
| CLAP export | Real exported entry/factory and plug-in binaries; gain-effect and synth examples; parameter automation, state, audio/MIDI ports, transport context, latency/tail/render extensions; project template and bundling helper |
| Audio UI toolkit | Parameter knobs/sliders, XY pad, waveform zoom/pan, piano keyboard, envelope editor, low-pass response editor and meter ballistics/peak hold |

**Examples:** native synthesizer, Live FX & Recorder, UI component gallery, offline WAV rendering and a CLAP library containing two plug-ins.

Current checks and limitations are recorded in [validation](docs/VALIDATION.md); original v0.2 reports are retained as historical evidence.

## Build and run

Use stable Rust **1.100+** (validated with 1.100.0). Open a terminal in this extracted `soniclaw` directory.

| Platform | Prerequisites |
|---|---|
| macOS | Xcode command-line tools (`xcode-select --install`) and Rust |
| Windows | Rust MSVC toolchain and Visual Studio C++ Build Tools with the Windows SDK |
| Debian/Ubuntu Linux | `sudo apt-get install build-essential pkg-config libasound2-dev libx11-dev libx11-xcb-dev libxcb1-dev libxcursor-dev libxi-dev libxrandr-dev libxinerama-dev libgl1-mesa-dev libxcb-dri2-0-dev libxcb-icccm4-dev libwayland-dev libxkbcommon-dev libegl1-mesa-dev` |

```sh
# No audio hardware needed: six-second synth render and parameter preset
cargo run -p soniclaw-render -- soniclaw-demo.wav

# Native synth: on-screen piano and computer keyboard
cargo run -p soniclaw-desktop --release

# Microphone/guitar processing, device settings, monitoring and WAV recording
cargo run -p soniclaw-live-fx --release

# New native multisample drums (starts muted)
cargo run -p soniclaw-reference-studio --release
# Same factory rendered without audio hardware
cargo run -p soniclaw-reference-studio -- --render soniclaw-drums.wav

# Interactive controls without opening an audio device
cargo run -p soniclaw-ui-gallery --release
```

A generated six-second `soniclaw-demo.wav` and preset are included. For external MIDI in the synth demo, enumerate with `cargo run -p soniclaw-desktop -- --list-devices`, then launch with `-- --midi 0` and optionally `--device 0`.

## Live FX & Recorder

1. Expand **Audio devices and session**, choose input/output devices and channels, and select a supported rate/buffer. Channel indices start at zero; `[0,0]` duplicates a mono input across stereo processing.
2. Click **Apply & start / restart**. A changed setting takes effect on restart. Unsupported combinations return an error; there is no silent rate fallback.
3. Enable **Monitor output** when ready. Monitoring starts muted. Use headphones with a live microphone. Meters display captured input and processed output before the monitor mute.
4. Shape the sound with the cutoff/gain knobs, XY pad or filter-response display. Bypass sends dry input through the output path.
5. Choose a new WAV filename, select dry input or processed output, and click **Record**. Recording works while monitoring is muted. **Stop & finalize take** finishes the file and shows a waveform preview for takes up to 20 MiB.
6. Save/load `soniclaw-session.json` with the settings controls. Stream errors or a missing named device stop the session and, when enabled, trigger one-second retries. Recovery creates a fresh engine, finalizes the current recording and mutes monitoring. Recording does not automatically resume.

Recording always uses the **active stream's** rate, even if pending settings have been edited. Overflow produces counted silent gaps rather than shortening the take. Capture/write/loss counters and the last finalized take's summary remain available. A write/finalization error is reported; a recording with a disk failure must not be treated as a successful take.

The requested hardware buffer is distinct from the internal 256-frame processing maximum. The duplex bridge adds a software prefill and can correct small clock differences (bounded to ±0.2%) using linear interpolation. It is not a general high-quality sample-rate converter or a promise of measured round-trip latency. Start with input/output on the same audio interface. See [native session details](docs/SESSIONS.md).

## Build CLAP plug-ins

```sh
cargo build -p soniclaw-clap-demo --release
```

Package the library built for the current platform:

```sh
# Linux
python3 tools/bundle-clap.py --library target/release/libsoniclaw_clap_demo.so

# Windows
python tools/bundle-clap.py --library target/release/soniclaw_clap_demo.dll

# macOS
python3 tools/bundle-clap.py --library target/release/libsoniclaw_clap_demo.dylib
```

This produces `dist/SoniclawDemo.clap`, a file on Windows/Linux and a bundle on macOS. The library contains **Soniclaw Gain** and **Soniclaw Synth**. Open the packaged plug-in through a CLAP-compatible host's plug-in scanner. No plug-in is installed into system directories by these commands.

The CLAP adapter supports stereo f32 main audio, an optional separate stereo sidechain key, MIDI 1, parameters/state/transport and optional embedded UI. Reference plug-ins enable the editor; the original gain/synth demo remains headless by default. Float64, native Wayland GUI handles, full MPE/note-expression routing and arbitrary bus layouts are not implemented. See [CLAP](docs/CLAP.md) and [multi-format builds](docs/FORMATS.md).

```sh
cargo build -p soniclaw-reference --release
python3 tools/bundle-clap.py --library target/release/libsoniclaw_reference.so --name SoniclawReference
# Explicitly download pinned format dependencies, then build VST3:
python3 tools/build-formats.py --fetch
# macOS only, also build AUv2 (not validated on Linux):
# python3 tools/build-formats.py --fetch --au
```

For an ABI integration check, build debug and run the bundled independent host:

```sh
cargo build -p soniclaw-clap-demo
cargo run -p soniclaw-clap-smoke
```

See [CLAP adapter](docs/CLAP.md) for the export API, safety boundary and validation instructions.

## Create a project

```sh
cargo install --path tools/cargo-soniclaw
cargo soniclaw new my-instrument --framework /absolute/path/to/soniclaw
cargo soniclaw new my-effect --kind clap --framework /absolute/path/to/soniclaw
cargo soniclaw new my-sampler --kind sampler --framework /absolute/path/to/soniclaw
cargo soniclaw doctor --framework /absolute/path/to/soniclaw
```

The app template is a native tone generator that starts muted. The CLAP template exports a gain effect with an editor; the sampler template exports a mapped sample instrument. Each generated project has an independent workspace boundary and local dependency paths. Keep the checkout available, quote paths containing spaces, and change the provisional plug-in namespace before distributing a product. Existing directories are refused.

## New module APIs

| Crate/API | Scope |
|---|---|
| `soniclaw-editor` | Host-parented native editor, shared widgets, bounded parameter-edit producer |
| `soniclaw-sampler` | Bank/zone validation and fixed-voice multisampling |
| `soniclaw-stream` | Worker-fed forward WAV player and underrun/error telemetry |
| `soniclaw-dsp` additions | Effects, FFT convolution and oversampled drive |
| `soniclaw-plugin` additions | Sidechain processing and `HostedProcessor` for shared standalone factories |

## Existing modules

| Crate | Responsibility |
|---|---|
| `soniclaw-core` | Borrowed buffers, processor lifecycle, MIDI/events, transport, atomic parameters, smoothing and automation events |
| `soniclaw-dsp` | Oscillators, ADSR, fixed-voice synth and preloaded sampler, gain, low-pass and delay |
| `soniclaw-engine` | Serial chains, automation splitting, event validation, transport and offline rendering |
| `soniclaw-graph` | Prepared DAG routing, parallel branches, fan-in/output latency alignment |
| `soniclaw-bus` | Explicit main-input/output/sidechain layouts, in-place adapter and sidechain compressor |
| `soniclaw-midi` | MIDI 1 streaming parser and optional native input |
| `soniclaw-device` | CPAL output/duplex hosts, channel maps, clock bridge, queues and telemetry |
| `soniclaw-session` | Portable persisted configuration; optional native controller/recovery |
| `soniclaw-record` | Audio-thread recording tap and background WAV worker |
| `soniclaw-ui` | Reusable egui audio controls and displays |
| `soniclaw-io` | WAV import/export |
| `soniclaw-state` | Versioned parameter presets and validated restoration |
| `soniclaw-plugin` | Format-independent lifecycle and stable parameter-ID binding |
| `soniclaw-clap` | CLAP ABI/host-buffer/event/state translation |
| `soniclaw` | Facade/prelude; native devices, native MIDI and UI remain optional |

`cargo-soniclaw`, `soniclaw-clap-smoke` and the Python bundle helper provide developer tooling. Libraries depend on mature CPAL, egui/eframe, midir, rtrb, hound, serde and clap-sys implementations. The core/DSP/engine/device/portable UI code forbids unsafe code; raw ABI handling is confined to `soniclaw-clap`, its export entry point, the native-handle boundary in `soniclaw-editor`, and independent host tools.

## Processing and automation

`AudioProcessor::prepare` can allocate; `process` must not allocate, lock, log or perform I/O. An in-place effect modifies input; an instrument replaces it. Prepared storage supports varying block lengths. Edit graphs and recreate devices on a control thread.

```rust
use soniclaw::prelude::*;
use std::sync::Arc;

fn automate_gain() -> Result<Vec<f32>, Error> {
    let gain = Arc::new(Parameter::new(ParameterInfo {
        id: "master.gain", name: "Gain", min: 0.0, max: 1.0,
        default: 0.5, unit: "",
    })?);
    let mut engine = Engine::default();
    engine.push(PolySynth::<16>::default());
    engine.push(Gain::new(gain.clone()));
    engine.bind_parameter(7, gain)?;
    engine.prepare(ProcessSpec {
        sample_rate: 48_000.0, max_block_frames: 256, channels: 2,
    })?;
    let notes = [Event { frame: 0, message: MidiMessage::NoteOn {
        channel: 0, note: 69, velocity: 100,
    }}];
    let automation = [AutomationEvent { frame: 12_000,
        parameter: 7, value: 0.2,
    }];
    soniclaw::engine::render_automated(&mut engine, 48_000, &notes, &automation)
}
```

Use `process_automated` with block-relative offsets in real time; use `render_automated` with absolute render-relative positions offline. Events must be ordered and parameters registered. Same-frame parameter groups are applied before rendering that sample. Gain still ramps over 10 ms: automation timing and DSP smoothing are distinct. `PluginInstance` binds parameters using stable FNV-1a IDs derived from their persistent string IDs and rejects collisions.

`soniclaw-bus::BusEngine` accepts separate main input/output and sidechain slices. `InPlaceBus` adapts an engine with mono/stereo mapping; `SidechainDucker` demonstrates a linked-channel compressor. The graph compensates declared processor latencies at fan-in and output sums; an intentional echo is not declared as processing latency. Compensation changes require preparation and are bounded to 262,144 frames per delay line.

## Checks and status

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

Full workspace checks require native prerequisites. Portable checks can use `cargo test -p soniclaw` without native features. The test suite includes processing allocation checks, MIDI/automation partition invariance, sidechain behavior, latency alignment, record loss/duration, concurrent stop/restart and UI rendering without a window.

[VALIDATION.md](docs/VALIDATION.md) records what actually ran. CI is configured for native Linux/macOS/Windows, but those remote jobs have not been executed for this archive. The source is not a signed installer and the project is not yet a production-equivalent replacement for JUCE.

Read [architecture](docs/ARCHITECTURE.md), [real-time contract](docs/REALTIME.md), [roadmap](docs/ROADMAP.md) and [contributing](CONTRIBUTING.md). Soniclaw is **MIT OR Apache-2.0**; dependencies keep their own licenses.
