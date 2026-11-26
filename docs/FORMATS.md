# CLAP, VST3 and AUv2

Soniclaw keeps one Rust CLAP processing/state/editor implementation. The pinned MIT-licensed `free-audio/clap-wrapper` translates it to VST3 and, on macOS, AUv2. The wrapper is C++; the processor and editor remain Rust. This is a real wrapper integration, not a native Rust implementation of each format ABI.

## CLAP reference bundle

```sh
cargo build --release -p soniclaw-reference
python3 tools/bundle-clap.py --library target/release/libsoniclaw_reference.so --name SoniclawReference
```

Use `soniclaw_reference.dll` on Windows or `libsoniclaw_reference.dylib` on macOS. The library exports these stable IDs:

| ID | Product |
|---|---|
| `org.soniclaw.reference` | Gain + Filter: Mode 0 pass-through, 1 gain, 2 low-pass + gain |
| `org.soniclaw.ducker` | Main stereo input + separate stereo sidechain key, with gain |
| `org.soniclaw.drums` | Multisample kick/snare/hat instrument (MIDI notes 36/38/42) |
| `org.soniclaw.fx-rack` | High-pass/peaking EQ, compressor, chorus, reverb, gain and peak limiter |
| `org.soniclaw.precision-gain` | Precision Gain with native f64 and f32 compatibility |

Reference defaults include an embedded editor. `--no-default-features` builds headless. The original `soniclaw-clap-demo` keeps the existing gain/synth IDs and defaults to a host-generated parameter UI; enable its adapter editor feature via Cargo's dependency-feature syntax if needed.

## VST3 / AUv2

Install a C++17 toolchain, CMake 3.21+, Git and Rust. Linux editor builds need X11/GL development libraries; see the main README. No JUCE dependency is required.

```sh
# Explicitly fetch pinned sources, build Rust + a self-contained VST3 bundle
python3 tools/build-formats.py --fetch

# macOS only: also build a native AUv2 .component using Apple's AudioUnitSDK
python3 tools/build-formats.py --fetch --au
```

Upstreams are pinned in `formats/dependencies.json`, with recursive VST3 SDK submodule revisions defined by the pinned SDK. Fetching is opt-in. The CMake integration links the Rust static archive and references its CLAP entry directly; it does not require a separately installed CLAP file. No plug-in is automatically installed. Default VST3 output is `build/formats/Release/SoniclawReference.vst3` on Linux; macOS/Windows output layouts depend on the generator.

The VST3 exposes the five reference products. Only Precision Gain advertises native f64 sample ports. See the [precision guide](DSP_ENGINE_PRECISION.md) for negotiation and the output bit-preservation test. The AUv2 recipe intentionally registers the first product, Gain + Filter, with type `aufx`, manufacturer `Sncl` and subtype `SnGF`; these provisional identities must be changed for an unrelated distributed product. AUv3/AAX are not integrated.

## Embedded editor contract

The CLAP GUI adapter negotiates X11, Win32 or Cocoa on the corresponding platform. It rejects floating windows and native Wayland handles (use an X11/XWayland host on Linux). `create` establishes GUI state; `set_parent` creates the native child with an already-valid parent. Size queries and resize requests work before attachment. Native window ownership and CLAP GUI calls stay on the host main thread. The X11 render thread schedules main-thread callbacks through the host. The backend is pinned to baseview 0.3.4 after exercising actual software-display rendering, rather than relying on an unverified newer repaint implementation. The opaque GL framebuffer does not require depth, stencil, alpha or sRGB visuals.

Controls use the shared Soniclaw UI widgets. Edits pass through a bounded queue; the audio consumer never locks. Every accepted adjustment emits a begin/value/end gesture. Partial host-output backpressure retains the current gesture stage rather than losing its end event. Restoring host state cancels outstanding GUI edits, acknowledges their sequences and completes any already-started gesture without replaying a stale value. Optimistic UI values are retained until the audio consumer acknowledges their sequence; subsequent host automation is read back from the shared atomics on repaint. UI edits are currently discrete adjustment gestures rather than a continuous touch gesture covering an entire drag. Keyboard capture is limited to the focused editor when egui requests it. Resize and host scale requests are supported; real multi-monitor DPI/focus testing on macOS/Windows remains pending.

The older reference products retain schema-1 parameter state and fixed factories. The new `soniclaw-project-plugin` exports `org.soniclaw.project-rack`, using schema-2 complete project state with deactivated structural reconstruction and stable parameter identities. See [projects](PROJECTS.md). Native wrappers still target the original reference factory unless explicitly adapted.

Linux builds and validation evidence are in `docs/VALIDATION.md`. macOS AUv2 and Windows runtime behavior are not certified by a Linux build or CI configuration.

## DSP Lab (v0.5)

`cargo build -p soniclaw-dsp-lab` exports `org.soniclaw.dsp-lab.effect` and
`org.soniclaw.dsp-lab.wavetable` as CLAP, with the shared editor feature enabled
by default. Use `--no-default-features` for headless exports. State is schema 2.
The independent smoke tool accepts the library path followed by `--dsp`.
The CMake native-format recipe still targets the original reference factory;
these new products are not automatically included in its VST3/AUv2 bundle.

## Hosting VST3 products

Version 0.23 adds trusted VST3 factory inspection, a common CLAP/VST3 control surface, native editors and a buffered stereo host. See [VST3 hosting](VST3_HOSTING.md) for API, workbench workflow, backend attribution, callback ownership and limitations. Export and hosting remain separate capabilities.
