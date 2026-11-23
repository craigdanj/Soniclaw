# Double-precision processing

## Use a native f64 signal path

Import `AudioBuffer64` and `AudioProcessor64` from `soniclaw::core` or the facade prelude. Wrap an interleaved mutable f64 slice with the explicit channel count. Construction accepts complete frames, including a zero-frame block. Samples remain f64 between processors; the double-precision classes share the existing filter and linked dynamics core.

```rust
use soniclaw::{
    core::{AudioBuffer64, AudioProcessor64, ProcessContext, ProcessSpec},
    dsp::precision::Gain64,
};

let mut gain = Gain64::new(0.5)?;
gain.prepare(ProcessSpec { sample_rate: 48_000., channels: 2, max_block_frames: 128 })?;
let mut pcm = [1. + 2f64.powi(-40), -1., 0.25, -0.25];
let mut buffer = AudioBuffer64::new(&mut pcm, 2)?;
let context = ProcessContext { events: &[], transport: Default::default() };
gain.process(&mut buffer, &context);
assert!(pcm[0] > 0.5);
# Ok::<(), soniclaw::core::Error>(())
```

## Choose a processor

- `Gain64` accepts linear amplitudes from -64 through 64 and can ramp across up to 16,777,216 frames. Each frame shares one gain across all channels.
- `Filter64::fir` accepts the same symmetric odd-length f64 taps as the existing FIR. `Filter64::iir` accepts existing second-order sections. Construct and prepare at the same sample rate. FIR reports its exact integer delay. IIR has frequency-dependent phase and reports zero fixed frames.
- `Dynamics64` accepts the existing linked mono/stereo `dynamics::Config`. Its f64 audio path includes lookahead storage and keyed processing with an f64 mono or stereo sidechain. Controls and UI telemetry retain their existing f32 configuration types. The bounds remain 8–192 kHz and up to 8,192 frames per block, with up to 20 ms lookahead. Audio samples are bounded to ±16 for dynamics as before.

```sh
cargo run -p soniclaw-dsp-suite -- double-precision output/double-precision
```

The example runs gain, FIR, IIR and dynamics in series, then writes the output and residual from two inputs separated by 2^-40 at unit amplitude. It uses a CSV to keep the f64 result intact.

## Lifecycle, errors and migration

Construct and prepare off the audio thread. Call process with one owner per prepared instance and reset on a discontinuity. Unprepared or incompatible blocks are cleared by the processor trait. The keyed dynamics method returns an error before changing history if the key shape or samples are invalid. After a rate or channel change, build rate-sensitive coefficients again and prepare every stage.

Existing f32 API calls and project state remain source-compatible. The original filter and dynamics APIs retain their previous coefficients and processing behavior for f32 inputs. Application-owned double-precision settings can use the existing dynamics configuration and the same FIR/IIR coefficient designs. Runtime state stays transient. This release provides application-owned DSP; the existing CLAP/VST3 bridges, devices, engine and Studio do not negotiate native f64 callback buffers.

[Executed verification](validation/v0_121_0/REPORT.md) includes sub-f32 precision preservation, native FIR/IIR and lookahead behavior, keyed partition equivalence and zero-allocation processing.
