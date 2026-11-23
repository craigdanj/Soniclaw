# Logarithmic ramps

Use `soniclaw_dsp::logarithmic_ramp::LogarithmicRamp` for continuous gain, effect mix or parameter transitions with different shapes for rising and falling values. Construct with a finite initial value and one curve for each direction.

`Curve::Linear` moves at a constant fraction of the total change per frame. `Logarithmic` makes more of the change near the start. `Exponential` makes more near the end. Both mapped curves accept a strength from 0 to 16. Strength zero behaves linearly. They apply to normalized time, so targets may be negative, zero or positive.

Call `set_target(value, frames)` from the render owner. Values move from the current value and arrive exactly after the given number of calls to `next_value`. Zero frames performs an immediate set. A new transition interrupts an active one continuously at its current value. `advance(frames)` skips ahead in constant time and `remaining_frames` reports when the destination will be reached. `apply_gain` advances once per interleaved frame.

```sh
cargo run -p soniclaw-dsp-suite --locked -- logarithmic-ramp /tmp/soniclaw-log-ramp
```

The example renders a tone with different rise and fall curves and records midpoint values, exact endpoint checks and allocations.

## Lifecycle and migration

Create and configure the ramp under exclusive ownership. The type uses scalar state and does not allocate. Route cross-thread control changes through the host's normal parameter path. Deliver sample-offset events by splitting blocks or by calling `next_value` at the correct frame. Durations are unsigned 32-bit frame counts. Invalid curves or nonfinite values return `InvalidParameter` and an invalid target leaves an active transition intact.

Set a value with zero frames after a transport discontinuity if that matches the application workflow. Save control targets and curve settings, and convert durations in seconds to frames after sample-rate changes. Existing linear smoothing APIs and state formats remain available. Logarithmic changes here refer to temporal interpolation. For dB controls, choose whether to ramp in the dB domain or ramp a converted amplitude.

[Release validation](validation/v0_143_0/REPORT.md)
