# FIR and Butterworth filter design

Use `soniclaw_dsp::filter_design` to design low-pass and high-pass filters at the target sample rate. Call `kaiser(rate, cutoff_hz, transition_hz, attenuation_db, response)` for an odd symmetric impulse response. The transition spans cutoff ± half the requested width. The attenuation and order come from the Kaiser approximation. Measure the response before relying on a specific rejection target. Call `butterworth(rate, cutoff_hz, order, response)` for an IIR cascade with a nominal −3.01 dB cutoff. Each section exposes normalized numerator and denominator coefficients.

Construct `Filter::fir(rate, taps)` or `Filter::iir(rate, sections)`, then prepare with the matching rate. Design and prepare belong on the control thread. Each audio owner has its own channel histories. Process and reset use prepared storage. FIR latency is half the odd tap length rounded down. IIR reports zero buffering latency and has frequency-dependent phase delay.

`fir_at` and `iir_at` evaluate a response at cycles per sample. The complex result gives magnitude and wrapped phase. `group_delay` estimates local delay in samples and returns None near a response zero. Inspect CSV output in a spreadsheet or plot it against frequency.

Run:
```sh
cargo run -p soniclaw-dsp-suite --locked --offline -- filter-design NEW_DIRECTORY
```
The example produces FIR and IIR WAV files, a response CSV, and JSON metrics. The tests verify endpoint gain, Butterworth cutoff and FIR delay. This version provides low-pass/high-pass designs and direct FIR execution. Very long filters can use the existing convolution processor with coefficient conversion to f32.

Persist the recipe (rate, cutoff, transition, attenuation, response or order) in your application state and redesign when a device rate changes. There is no change to earlier preset schemas. Design supports 8–384 kHz, frequency strictly inside the guarded Nyquist range, 21–120 dB estimated attenuation, at most 4095 FIR taps and IIR order 1–20. Invalid recipes return InvalidParameter. Invalid preparation preserves the previous configuration. Callback dimension errors clear the output. Build replacements before switching processors and crossfade when continuous audio is required. See [executed checks](validation/v0_101_0/REPORT.md).

