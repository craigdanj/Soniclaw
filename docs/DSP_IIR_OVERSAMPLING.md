# Polyphase IIR oversampling

Construct IirOversampler with the base ProcessSpec and prepare the inner effect using high_rate_spec. process_with runs the inner effect on an interleaved 2× signal, then decimates in place. Use a single owner for process and reset. The two cascaded allpass branches per stage have a frequency-dependent phase response. dc_delay_samples gives a low-frequency estimate. For a phase-aligned wet/dry blend, use the FIR path or route dry through an identical IIR converter. There is no exact broadband integer compensation for this filter.

Construction allocates up to 2,097,152 high-rate samples. Process allocates no storage and blocks may vary within the declared maximum. Invalid dimensions and nonfinite inputs reject before state advancement. Nonfinite high-rate results clear output and reset history. Persist the selected converter externally and rebuild when rate or topology changes. Existing FIR preset selection is unchanged. Only factor 2 is exposed here.

The example compares direct, IIR and FIR tanh runs using a coherent tone and a residual metric that excludes the fundamental and third harmonic. Residual combines alias products and other distortion above the retained harmonics. It is a fixture comparison. The tests cover DC gain, an image rejection tone and block partition equivalence.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- iir-oversampling NEW_DIRECTORY`. [Executed checks](validation/v0_103_0/REPORT.md).
