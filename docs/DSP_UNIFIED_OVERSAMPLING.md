# Unified oversampling

## Process a nonlinear effect at a higher rate

Construct `UnifiedOversampler::new(OversamplingConfig { ratio, mode, quality, integer_latency }, spec)` from `soniclaw::dsp::oversampling`. Prepare your inner processor using `high_rate_spec()`. In the audio callback call `process_with` and process the supplied interleaved block at the provided rate. Choose `Ratio::One` for direct processing, or Two through Sixteen for power-of-two conversion. A nonempty input invokes the closure once.

Each FIR stage is linear phase. Use Balanced for 65 taps per stage or High for 129 taps and a narrower transition. The IIR stages use cascaded allpass filters with six total sections in Balanced and eight in High. They run with a shorter nominal delay and frequency-dependent group delay. High uses a wider guaranteed reject region toward the transition than Balanced. Compare actual alias rejection with your nonlinear workload and sample rate.

```sh
cargo run -p soniclaw-dsp-suite -- unified-oversampling output/oversampling
```

The example renders a driven tone with increasing harmonics at each factor and reports latency and allocation counts.

## Report and compensate latency

`latency()` reports exact constant group delay for FIR and a low-frequency group-delay reference for IIR. Set `integer_latency` to add a fractional allpass after IIR decimation and bring the reference to the next whole base-rate sample. IIR phase remains frequency-dependent, so an aligned dry mix can still color the signal near the transition. This latency excludes the user callback and any filters it owns. Convert the callback's latency to base-rate samples when declaring host latency.

## Lifecycle, compatibility and bounds

Build or change the ratio, filter, or quality on the control thread. Use each prepared instance from one processing thread. A format change needs a fresh instance and new inner prepare. Reset on transport discontinuities. The provided callback must keep audio-thread guarantees. Scale transport and MIDI offsets when passing context to a high-rate callback; the legacy FIR `Oversampled` wrapper can still do this for its supported factors.

The maximum high-rate block is 4,194,304 interleaved samples and the common process spec allows rates up to 1,536,000 Hz. Bad input shape or nonfinite input returns an error before history advances. Nonfinite callback output is cleared and the converter resets. A failing construction leaves the old instance reusable.

Existing FIR `Oversampler`, `Config` and 2× `IirOversampler` entry points remain source-compatible. Serialized legacy FIR configurations and existing product state migrate automatically because their types are unchanged. Persist new mode/ratio controls in your own configuration and reconstruct filter history on reset, load or prepare.

[Executed verification](validation/v0_117_0/REPORT.md) checks impulse latency and DC moments, partition independence, rejection of above-Nyquist content and allocation-free processing.
