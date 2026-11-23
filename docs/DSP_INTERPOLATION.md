# Fractional-delay interpolation modes

`soniclaw::dsp::fractional_delay::DelayLine::with_mode` adds three choices to the existing programmable delay line. Set an initial delay in samples, choose a maximum delay, and choose how many samples per frame the delay may slew. On each frame call `tick` with an interleaved input and output of exactly the prepared channel count. `set_delay` changes the target without allocating.

| Mode | Use |
|---|---|
| Integer | Tap the nearest stored frame. Prefer for fixed sample alignment. Changing the selected sample can click. |
| Linear | Low-cost fractional delay for modulation. The fractional weights attenuate high frequencies. |
| Cubic | Existing cubic interpolation for smoother sweeps. |
| Lagrange3 | A third-order polynomial interpolator using four history samples. Use it to reduce interpolation error in modulated delays. |
| Thiran | A first-order allpass with unity magnitude for a fixed delay. Useful in feedback paths and for low-frequency phase alignment. Phase delay varies with frequency and rapid modulation can introduce transients. |

Use `tap` to read additional historical samples in stateless modes. For a stateful tap, keep one `ThiranTap` per channel and tap, and call `tap_thiran` once per input frame before `tick`. This API is available on a line using any main interpolation mode. Reusing one state across taps couples their history. `tap` returns `InvalidParameter` when the main mode is Thiran.

```sh
cargo run -p soniclaw-dsp-suite -- interpolation output/interpolation
```

The example renders a five-way flanger comparison using the same mono input and modulation. Use a fresh output directory.

## Lifecycle and migration

Construct on the control thread and process from a single owner. Each instance accepts 1–64 channels, up to 262,144 frames of configured maximum delay and 4,194,304 samples of storage. The usable delay starts at two frames. Input and output shape errors leave history untouched. Nonfinite input samples are stored as zero. Reset both the line and any independent tap states after seeking. A different maximum, interpolation mode, or channel layout calls for rebuilding the line.

Existing `DelayLine::new` and its two-variant `Interpolation` remain source-compatible. Persist the new mode and delay controls in your own data model. History and allpass state are reconstructed on load; no Soniclaw saved project changes are required.

[Executed verification](validation/v0_119_0/REPORT.md) covers polynomial interpolation, integer rounding, fixed-delay allpass magnitude and delay, channel isolation, reset and allocations.
