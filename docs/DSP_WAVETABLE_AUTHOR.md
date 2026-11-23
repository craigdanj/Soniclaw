# Wavetable authoring

Decode a WAV off the audio thread, choose a mono channel, and mark the start and exclusive end of up to 16 cycles. The Rust API accepts PCM with frame ranges so the caller controls decoding and cycle selection. compile removes a linear boundary trend when requested, resamples each selected region periodically to a common size, removes DC, optionally normalizes each cycle, and finds a circular alignment against the first frame using FFT correlation. The returned bank uses the existing harmonic mipmaps.

Review the aligned cycles before using the bank. Seam repair trades a simple boundary correction for a change in the cycle's low harmonics. Phase alignment rotates whole cycles and can be ambiguous for strongly different shapes or repeated subcycles. This release takes explicit cycle marks. A pitch scanner or GUI can propose those ranges separately. Use looped periodic input for the most natural result. The example generates and opens a mono float WAV, compiles two cycles and renders a continuous morph.

Compile and drop banks off the audio thread. Clone an Arc into each prepared oscillator. Persist source asset ID, selected ranges and authoring settings or save the reviewed cycle arrays. Previous wavetable banks and patches retain their shape. Frame size must be a power of two from 64–4096, selected source ranges must contain 16 to 32× target-size samples, and source values must be finite and bounded by ±16. Bad ranges or effectively silent cycles fail without publishing a bank. The phase regression uses a known circular offset.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- wavetable-author NEW_DIRECTORY`. [Executed checks](validation/v0_110_0/REPORT.md).
