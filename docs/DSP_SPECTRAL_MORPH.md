# Two-input spectral morphing

Construct `soniclaw::dsp::spectral_morph::SpectralMorph` with a prepared process
spec, FFT window size, envelope smoothing radius in bins, and two mix values.
Pass a mutable interleaved carrier and an equally shaped modulator to
`process`. Both streams must begin on the same sample. The processor replaces
the carrier with the result and reports one FFT window of latency.

`Morph::envelope` blends the broad log-amplitude envelope. Set it to one and
keep `fine_structure` at zero to transfer the modulator's broad spectral
shape while maintaining the carrier's relative peaks. Increasing
`fine_structure` blends the normalized bin amplitudes as well. Resynthesis
uses carrier phase where meaningful, or zero phase for newly introduced bins.
It is an effect for synchronized sources. There is no pitch tracking or
time stretching in the morph core.

```sh
cargo run -p soniclaw-dsp-suite -- spectral-morph output/morph
```

The example renders a harmonically rich carrier, full envelope transfer and
a blended fine structure using a pulsed two-tone modulator. Blend off keeps
the original spectrum, delayed by the window. Smooth control changes or
crossfade complete processors for preset transitions. Controls sampled at
hop boundaries can produce audible changes when jumped.

Use a power-of-two FFT from 256 to 8192, hop size one quarter of the window,
and 2–128 envelope bins subject to the FFT bound. Process spec allows 8–192 kHz
and 1–8 channels. Each channel has independent FFT history. Allocate and plan
off-thread, then use a single audio owner. Shape mismatch fails before state
advances; nonfinite samples become silence. Reset clears overlap history.
Persist window/radius/mix settings and the application references for both
sources. Existing spectral effects and sessions require no migration.

[Executed checks](validation/v0_125_0/REPORT.md).
