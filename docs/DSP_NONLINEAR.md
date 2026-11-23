# Anti-aliased nonlinear processors — 0.90.0

## Workflow

Use `soniclaw_dsp::nonlinear::{Shape, Adaa1, Config, Waveshaper}` in effects and instruments independently of Studio. Choose hard clipping, tanh saturation or bounded cubic soft clipping. Prepare on the control thread, then process normal interleaved audio. This adds a reusable antiderivative path; existing drive/distortion processors keep their original sound.

Run `cargo run -p soniclaw-dsp-nonlinear --offline -- NEW_DIRECTORY` for coherent-tone spectral comparisons, a WAV for each algorithm and measured callback allocations. The example compares direct waveshaping with the same three-tap output filtering used for the small-signal reference, standalone ADAA and oversampled ADAA. Results are signal-specific and include the selected bin, FFT length and drive; they are not universal alias-rejection specifications.

## API, algorithm and latency

`Adaa1::tick(f64)` uses the difference of first antiderivatives divided by the difference of adjacent inputs. Near-equal inputs use the curve at the midpoint to avoid cancellation. Tanh uses a stable log-cosh expression. `Shape::sample` exposes the corresponding unfiltered curve for comparison. The primitive has half a sample of small-signal delay and is intentionally not an `AudioProcessor` with an integer latency claim.

`Waveshaper` adds a two-point average to its antiderivative output, giving one sample of nominal small-signal delay; its dry path is delayed one sample too. This also changes the high-frequency response. `Config.latency_multiple` may be 1, 2, 4 or 8 and adds fixed output padding. For `Oversampled<Waveshaper>` use a latency_multiple equal to the oversampling factor, so inner latency converts to an exact base-rate frame. The full wrapper reports FIR conversion latency plus one base-rate frame.

The shaper supports `drive` 0–32, `bias` −2–2, `mix` 0–1 and 0–500 ms parameter smoothing. `set_targets` validates all controls before changing any, allocates nothing, and runs on the audio owner (or via an application command queue). It updates gain/bias/mix without rebuilding. `config()` returns current targets for persistence. Shape and latency padding are structural: prepare a replacement off-thread to change them. The biased curve subtracts its zero-input value; startup and changing bias still have a settling transient.

```rust
let config = soniclaw_dsp::nonlinear::Config {
    shape: soniclaw_dsp::nonlinear::Shape::HardClip,
    drive: 4.0,
    ..Default::default()
};
let mut effect = soniclaw_dsp::nonlinear::Waveshaper::new(config)?;
// effect.prepare(spec)?; effect.process(&mut buffer, &context);
```

## Lifecycle, state and bounds

`prepare` allocates one state and small delay storage per channel; callbacks and reset allocate nothing. This is a single-owner processor with no global atomics or UI access. `process_checked` returns shape/size/nonfinite input errors before advancing states. The `AudioProcessor` adapter clears output and records `last_error`. The scalar primitive resets/silences nonfinite input. Call reset for a new signal history; inner histories are not serialized.

Schema-1 serde `Config` rejects unknown fields and invalid control ranges. Persist configuration only, then construct/prepare on reopen. Old products, presets and parameter IDs are unchanged. Core `ProcessSpec` rate/channel limits apply; padding is at most seven samples per channel. ADAA reduces aliasing for the tested signals but is not alias-free and is not a calibrated circuit/amp model. The first-order method and added filter have audible frequency-response and transient tradeoffs; compare at matched levels. Oversampling increases processing cost and adds FIR latency.

## Validation and reference

Tests verify analytic antiderivatives, near-equal/static inputs, integer impulse-peak latency, partition equivalence, channel isolation, zero allocations, invalid-control transactions and spectral improvement for the owned coherent-tone fixture. Reports and executed commands are in `docs/validation/v0_90_0/`. External DAW, hardware and platform listening acceptance remain pending.

Algorithm background: Parker, Zavalishin and Le Bivic, [Reducing the Aliasing of Nonlinear Waveshaping Using Continuous-Time Convolution, DAFx 2016](https://www.dafx.de/paper-archive/2016/dafxpapers/20-DAFx-16_paper_41-PN.pdf). This is an independent implementation; no paper/repository source code is bundled.
