# Selectable panning laws

Use `soniclaw_dsp::stereo_panner::StereoPanner` to place a mono source in a stereo field or balance a stereo source. Position runs from −1 at left to +1 at right, with 0 at center. Each curve reaches unity on the selected side and silence on the other at its endpoints.

| `PanLaw` | Center gain per channel | Behavior |
|---|---:|---|
| `Balanced` | 0 dB | Attenuates the far channel. The near channel stays at unity. |
| `Linear` | −6.02 dB | Linear complementary channel gains. |
| `Sine3Db` | −3.01 dB | Equal power with sine-shaped gain. |
| `Sine4_5Db` | −4.52 dB | Sine curve with a stronger center reduction. |
| `Sine6Db` | −6.02 dB | Squared sine gain, with a different shape from `Linear`. |
| `SquareRoot3Db` | −3.01 dB | Equal power with square-root-shaped gain. |
| `SquareRoot4_5Db` | −4.52 dB | Square-root family curve with a stronger center reduction. |

`PanLaw::gains` provides static gains for meters, visualization or custom render code. For moving sources, construct `StereoPanner` with a law, starting position and number of smoothing frames. `set_pan` retargets continuously. Zero smoothing frames switches position immediately. `set_law` changes the law immediately. For seamless law changes on playing content, crossfade processor outputs or change the law during silence.

`process_mono` writes a mono input slice to a separate interleaved stereo output. `process_stereo` and the in-place `AudioProcessor` path apply the gain for each existing channel independently. They do no crossfeed or stereo-image rotation. The `Balanced` law keeps a centered stereo source at its original level.

```sh
cargo run -p soniclaw-dsp-suite --locked -- stereo-panner /tmp/soniclaw-panner
```

The example renders seven mono-to-stereo sweeps and reports the center attenuation and callback allocations for each law.

## Lifecycle, errors and persistence

Configure under exclusive control and move the processor to the render owner. Positions must be finite and within −1 to +1. Mismatched channel counts and mono-output lengths return `InvalidBuffer` before writing samples or advancing the pan state. Through the infallible `AudioProcessor` interface, an incompatible block is silenced. Invalid target positions leave the ramp intact. Reset snaps to the current target.

Save the law, target and smoothing time in application state, and derive the frame count at the next sample rate. Existing mixer and balance behavior and plug-in IDs are preserved. Gain normalization here is unity at the endpoints. Validate center and endpoint gain when migrating from a host or engine with a different normalization convention. Equal-power gains describe a single source distributed over two outputs. Correlated stereo signals summed to mono may still vary in level.

[Release validation](validation/v0_144_0/REPORT.md) · [JUCE panner rule reference](https://docs.juce.com/master/juce__Panner_8h.html)
