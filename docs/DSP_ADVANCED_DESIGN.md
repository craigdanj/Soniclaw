# Advanced FIR and IIR filter design

## Build a filter from measurable requirements

Use `design_iir(IirSpec { ... })` from `soniclaw::dsp::filter_design`. At 48 kHz, start with a 6 kHz passband edge, 10 kHz stopband edge, 0.5 dB passband ripple and 60 dB stopband attenuation. Swap the order of the edges for a high-pass design. Choose `IirFamily::Butterworth` for a monotonic response, `ChebyshevI` for passband ripple, `ChebyshevII` for stopband ripple, or `Elliptic` for ripple on both sides and fewer sections for a given transition.

The design contains the selected order and stable second-order sections. Inspect their complex frequency response using `iir_at(&sections, frequency_hz / sample_rate)`. Give the sections to `Filter::iir` and prepare the processor at the design rate. `Section::new` accepts custom biquad coefficients, normalizes by a0, and rejects unstable denominators.

## Build a linear-phase FIR

Give `least_squares` an odd tap count and sorted, non-overlapping `FirBand` values. Frequencies run from 0 to 0.5 cycles per sample. Each band specifies a constant linear gain and relative error weight. Omit transition regions. The result is an odd symmetric kernel for `Filter::fir`; latency is one-half of the order. Use `half_band(taps, transition_width)` for a unity low-pass design with exact alternating zeros and a center tap of 0.5. Choose a tap count of 4k+3 and specify a total transition width centered at 0.25 cycles per sample.

```sh
cargo run -p soniclaw-dsp-suite -- advanced-design output/advanced-design
```

The example compares four IIR families, writes the learned FIR coefficients and renders measured elliptic and half-band impulse WAVs.

## Lifecycle, bounds and migration

Design filters and prepare processors on the control thread. The offline solver may allocate and performs bounded matrix work. The prepared f32 processor uses f64 state and supports allocation-free processing. Reset history when seeking or changing signal sources. Build new coefficients for a new sample rate and re-prepare before processing. For a seamless change, crossfade two prepared processors in the application.

The IIR utility accepts 8–384 kHz, orders up to 20, ripple of 0.01–6 dB and attenuation of 12–140 dB. It returns `InvalidParameter` for unusable edges or specifications it cannot meet. The FIR solver accepts 3–511 taps and up to 32 bands. It rejects ill-conditioned fits. For tight transitions, widen the gap, decrease the order, or use the existing Kaiser design. Least-squares shaping minimizes integrated error and users should independently check peak ripple constraints. FIR half-bands use constrained least squares. This release designs low-pass and high-pass IIR cascades.

Existing `butterworth`, `kaiser`, and `Filter` calls stay source-compatible. Persist the design specification or coefficient arrays in your own project schema and regenerate the state on load. No saved sessions or parameter IDs need migration.

[Executed verification](validation/v0_116_0/REPORT.md) covers dense response bounds, expected orders against SciPy, half-band structure, and allocation-free filtering.
