# Half-band filters and custom oversampling

Use `soniclaw_dsp::half_band_design` when a nonlinear processor needs a specific reconstruction and anti-alias filter budget. A stage doubles the rate on the way in and halves it on the way out. Set the two filter specs separately to spend more rejection where the processor needs it. Stack one to four stages in order from base rate upward for 2×, 4×, 8× or 16× conversion.

- `FilterSpec` specifies full transition width in cycles per sample at the **higher rate of that stage**. For width 0.10, the passband ends at 0.20 and the stopband begins at 0.30. Attenuation is positive dB.
- `HalfBand::design` exposes the symmetric FIR taps or the even and odd IIR allpass coefficients, with `magnitude_at` and `measured_attenuation_db` for inspection. FIR uses constrained Remez exchange. IIR uses elliptic coefficients with coupled allpass branches.
- `CustomOversampler::new(base_spec, &stages)` prepares coefficients, histories and conversion buffers. Prepare the nested processor using `high_rate_spec()`. Call `process_with` on each base-rate block, and process the borrowed higher-rate buffer inside the closure.
- `dc_delay_samples()` reports round-trip base-rate delay. FIR-only cascades have linear phase. An IIR stage has frequency-dependent group delay, so its reported DC delay is nominal. A host that needs integer delay can choose a suitable FIR design or provide its own fractional compensation.

```sh
cargo run -p soniclaw-dsp-suite --locked -- half-band-design /tmp/soniclaw-half-band
```

The example processes a soft-clipped tone and writes FIR and IIR WAVs. Its report includes separate up/down rejection, callback allocations and nominal delay. Use a fresh output directory.

## Lifecycle, inputs and recovery

Design and prepare on the control thread. Move the instance to the callback owner and reuse it across blocks. The conversion path uses prepared storage. The closure must follow the same real-time rules. On playback discontinuity, call `reset`. At a sample-rate or quality change, prepare a replacement off the audio thread.

Widths span 0.02–0.24 and attenuation requests span 20–120 dB. A FIR design has at most 255 taps. A design that cannot meet a request within its numerical and order budget returns `InvalidParameter`. The high-rate spec follows the usual Soniclaw spec limits and each scratch buffer is capped at 2,097,152 samples. A mismatched or nonfinite input is rejected before advancing history. A nonfinite high-rate result or final output clears the destination and resets history.

The design checks attenuation on a dense grid with a margin. Verify unusually tight targets on the production signal chain. Additional nonlinear aliasing depends on the nested processor and sample rate. Save family and numeric stage specifications in application state, then rebuild on load. Existing preset oversampling APIs and state formats continue to work.

[Release validation](validation/v0_138_0/REPORT.md) · [FIR half-band design paper](https://scispace.com/pdf/a-trick-for-the-design-of-fir-half-band-filters-4pzn30v77t.pdf) · [Coupled allpass model](https://www.mathworks.com/help/dsp/ref/designhalfbandiir.html)
