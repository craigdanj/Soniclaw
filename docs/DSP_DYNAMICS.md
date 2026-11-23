# Flexible linked and keyed dynamics — 0.98.0

## Workflow and API

`soniclaw_dsp::dynamics::Dynamics::new(Config)` creates a prepared compressor, downward expander or upward compressor. Config includes fixed detector topology (peak or RMS window), key highpass, lookahead and runtime Controls. Normal AudioProcessor processing detects the main input. `process_keyed(buffer, key, key_channels)` accepts a separate interleaved mono/stereo sidechain with exactly the same frame count. Detection links channels by the maximum absolute highpassed key sample, then optionally averages its squared value over the RMS window. Both main channels receive the same gain, preserving their instantaneous balance. This is linked maximum-channel RMS, not independent stereo RMS averaging.

Controls expose threshold, ratio, quadratic soft knee, attack/release, hold, maximum gain change, makeup and parallel mix. `set_controls` validates before mutation; call on the audio owner or use an application command queue. `telemetry()` returns the latest detector level and gain including makeup; it is not thread-safe shared metering. Poll through a suitable snapshot channel.

Compressor reduces levels above threshold by slope 1/ratio; expander attenuates below threshold by slope ratio; upward mode raises below-threshold levels with slope 1/ratio. Knee blends the gain curve continuously around threshold. Desired gain is bounded before makeup. Attack follows increasing magnitude of gain change; release follows relaxation. Hold retains the previous target for its duration when the requested gain-change magnitude decreases. It is not a dedicated threshold-crossing gate hold. A silence floor of −120 dB bounds upward noise amplification; use a gate when needed.

## Timing, bounds and errors

Mono/stereo 8–192 kHz, max block 8192. Lookahead 0–20 ms rounds upward to frames and is the declared latency. Main dry and wet are delayed equally; the key stays undelayed. RMS window 1–4096 adds detector averaging memory, not another output delay. Detector highpass is zero (off) or 20–2000 Hz. Key dimensions/range/finite checks happen before state advancement; invalid keyed calls return InvalidBuffer and leave input/state intact, allowing a valid retry. The AudioProcessor form silences invalid/unprepared dimensions. Main nonfinite input becomes zero and finite input is limited to ±16; keyed input must already be finite and within ±16.

Controls: threshold −80–0 dB, ratio 1–20, knee 0–24 dB, attack 0–500 ms, release 0–5000 ms, hold 0–1000 ms, maximum change 0–36 dB, makeup ±24 dB, mix 0–1. Zero attack/release updates immediately. Invalid settings retain old controls. No true-peak limiting, automatic loudness matching, adaptive multiband splitting, or guaranteed output ceiling is supplied. Use existing multiband/limiter processors where required. Upward mode may increase noise and makeup can raise peaks substantially.

## Lifecycle, persistence and migration

Schema-1 Config/Controls/Detector serialize via serde and reject unknown fields/schema mismatches. Save config, not RMS/delay/envelope histories. Construct and prepare off-thread; delay/RMS rings allocate there. Processing, keyed processing, telemetry, controls and reset use prepared storage without locks. One audio owner mutates each processor. Topology/window/lookahead changes require a replacement prepared off-thread; apply compensation/crossfade at the host level. Reset clears detector, gain, hold and output delay. A failed prepare requires a new valid prepare; preserve the prior instance in your host. Existing compressor, limiter, multiband and dynamic-EQ presets/APIs remain unchanged.

## Runnable example and validation

`cargo run -p soniclaw-dsp-suite --offline -- dynamics NEW_DIRECTORY` writes downward compression, expansion, upward processing and a separately keyed ducking fixture with finite-sample/latency/allocation reports. Unit tests check transfer curves and knee continuity, linked stereo, exact delay startup, rejected key retry and block-size independence with RMS detection. Callback fixtures measure zero allocations. See docs/validation/v0_98_0/. Results are signal-specific; no perceptual transparency or commercial-compressor equivalence is implied.
