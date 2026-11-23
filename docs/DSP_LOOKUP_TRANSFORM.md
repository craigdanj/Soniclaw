# Lookup-table transforms

Use `soniclaw_dsp::lookup_transform::LookupTransform` to turn a pure DSP function into a reusable table, for example a soft clipper, saturation curve or perceptual transfer function. Construct it from finite endpoints, a resolution and a function on the control thread.

```rust
use soniclaw_dsp::lookup_transform::{LookupTransform, LookupWaveshaper};
let curve = |x: f32| (3.0 * x).tanh();
let table = LookupTransform::prepare(-2.0, 2.0, 257, curve)?;
let error = table.measure_error(20_003, 1e-3, curve)?;
assert!(error.maximum_absolute < 0.00025);
let waveshaper = LookupWaveshaper::new(table);
# Ok::<(), soniclaw_core::Error>(())
```

`evaluate` rejects input outside the interval. `evaluate_clamped` returns the nearest endpoint for finite out-of-range input. Both reject NaN and infinities. `LookupWaveshaper` applies the clamped rule in place through `AudioProcessor` or `process_in_place`, and silences nonfinite samples.

```sh
cargo run -p soniclaw-dsp-suite --locked -- lookup-transform /tmp/soniclaw-lookup
```

The example renders soft clipping and records maximum absolute, RMS absolute and maximum relative error. Grid-based measurements sample the reference off the callback and use a caller-selected lower bound for the relative-error denominator. For sharp corners or discontinuities, increase resolution, narrow the interval or choose direct evaluation if the recorded error is unsuitable.

## Lifecycle and bounds

A table accepts 2–1,048,576 points. Preparation rejects unordered or nonfinite endpoints and any nonfinite sampled value. The immutable table data can be shared via a cheap clone created off the audio thread. Evaluating and processing reuse existing storage. Build and replace tables away from the callback if controls change. This is a memoryless operation and produces new harmonics when used as a waveshaper. Prepare oversampling independently if the audible aliasing budget requires it.

Persist the curve identifier and its settings, then rebuild the table. Arbitrary Rust closures and measured error reports are application data. This feature introduces no preset migration or dependencies.

[Release validation](validation/v0_140_0/REPORT.md)
