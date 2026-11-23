# End-to-end float64 audio

Soniclaw 0.145 adds an opt-in double-precision audio path. Use `Engine64` and `Graph64` with `AudioProcessor64` processors. Audio remains in f64 through serial stages, graph inputs and edge gains, summation, latency compensation and output. A new file path reads and writes IEEE float64 WAV. In native plug-ins the host must select the double-precision format to preserve all source and destination bits.

## Engine, graph and offline I/O

```rust
use soniclaw::{dsp::precision::Gain64, prelude::*};

fn process() -> Result<[f64; 2], Error> {
    let mut engine = Engine64::default();
    engine.push(Gain64::new(0.5)?);
    engine.prepare(ProcessSpec {
        sample_rate: 48_000.,
        max_block_frames: 128,
        channels: 2,
    })?;
    let mut frame = [1. + f64::EPSILON, -1.];
    engine.process(&mut frame, &[])?;
    Ok(frame)
}
```

Use `Graph64::add`, `connect`, `set_input` and `set_output` as with the f32 graph, using f64 edge gains. Wrap it as a stage in `Engine64` or process it directly with an `AudioBuffer64`. The prepared graph rejects cycles and compensates fan-in and output delays. Engine64 supports sample-offset events, parameter automation and reset through the same engine contract. `engine::render64` renders a previously prepared engine into a complete in-memory f64 buffer. `io::read_wav64_with_limit` and `write_wav64` support finite IEEE float64 samples in ordinary little-endian RIFF WAV, with up to 64 channels. The reader's limit counts decoded bytes. The writer creates a fresh destination and retains values outside ±1 without clipping. On a write error, discard the partial output. RF64 and WAVE_FORMAT_EXTENSIBLE are outside this reader's scope.

`device::OutputHost::start64` uses `Engine64` for standalone output and converts once to the hardware-selected output type, if needed. The device layer clamps to its normal ±1 output range. A native f64 stream keeps f64 samples to the device API. Physical device format negotiation and audio-loopback accuracy depend on the installed driver. The existing duplex and recording adapters use their existing f32 audio path.

## Studio audio sessions

`studio::precision::PreparedSession64::new` takes a Studio `Document`, resident `Asset64` data keyed by clip asset IDs, and a factory for f64 inserts. This path handles stereo sessions with resident mono/stereo audio clips, clip gain/fades/looped regions, track inputs, stereo panning, group gain and mute/solo. It uses the same clip edit metadata and graph latency alignment. Supply transport position and playback state explicitly when calling `render_block`, or drive the prepared session from `Engine64`. Reset or seek clears processing history. An asset's sample rate must match the session. For missing assets, unsupported descriptors or an unsupported document, correct the source configuration and prepare again.

This is the f64 session API for supported documents. The existing desktop mixing/export path, instruments/MIDI clips, disk streaming, cue/aux/bus routing, and session automation have their existing APIs. The f64 constructor rejects their unsupported document features at compile time of the session graph with `InvalidRouting`. UI and automation control values retain their existing scalar formats. Audio samples, gains used inside the f64 graph and delay buffers retain f64 precision.

## Plug-in integration

- Export a `PluginInstance::new_64` product through the existing CLAP macro. The adapter advertises `CLAP_AUDIO_PORT_SUPPORTS_64BITS` and common sample size, accepts native `data64` on supported stereo ports, and handles the normal f32 mode by converting at the product boundary. Both modes use one f64 processor history. State continues to use the regular preset envelope and stable parameter IDs. This constructor supports main stereo input/output or a stereo instrument output. Existing sidechain and project-backed product constructors keep their f32 contract.
- For a trusted CLAP in-process host, activate the instance and use `HostedAudio::process_block64` inside the existing scoped audio lease. Inspect `ClapInstance::supports_f64` first. A product without advertised wide ports returns `InvalidSpec` and clears the supplied output. This narrow host path supports main mono/stereo I/O according to the existing scoped layout rules.
- The pinned VST3 wrapper forwards f64 planes for products whose CLAP ports all advertise f64. The CMake build applies the tracked patch to a build-local copy. `vst3::Instance::start_direct64` negotiates native `kSample64` for trusted in-process stereo plug-ins, prepares native VST3 planes, and uses the existing revocable direct lease. Unsupported products fail negotiation. Direct VST3 uses the existing audio and normalized-control contract, with MIDI/editor/offline/multibus hosting provided by separate APIs. AUv2 follows its existing wrapper capabilities.

Keep file operations, graph construction, format negotiation, activation, and device creation off the callback. CLAP/VST3 modules must be trusted and must obey their host ABI. Retire prepared VST3 and plug-in owners from the control thread. Block-size and format changes require preparation. Engine and graph processing use the configured maximum block size. Existing product identities and project data continue to load. The precision reference effect has its own product ID and refactors no saved preset data.

## Run the examples

```sh
cargo run -p soniclaw-dsp-suite --locked -- engine-precision /tmp/soniclaw-f64
python3 tools/build-formats.py --fetch --no-editor
cargo run -p soniclaw-sdk-tools --features vst3-tools --locked -- \
  --precision-host-demo /tmp/soniclaw-native-f64.json \
  clap target/release/libsoniclaw_reference.so \
  vst3 build/formats/Release/SoniclawReference.vst3
```

Use fresh output paths and the reference plug-ins built from the same source release. The session example renders two nearly cancelling sources through a latency-compensated graph, writes an IEEE float64 file, and reopens it bit-exactly. The native example presents a sub-f32 input difference via the CLAP and VST3 ABI and checks for it after processing, checks the f32 compatibility path, and reports allocations observed by the Rust host. [Executed Linux verification](validation/v0_145_0/REPORT.md).
