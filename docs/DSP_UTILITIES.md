# Reusable DSP utilities

## Estimate level with independent attack and release

Construct `EnvelopeFollower::new(spec, mode, attack_ms, release_ms)` from `soniclaw::dsp::ballistics`. `Peak` follows absolute amplitude with exponential smoothing. `Rms` smooths squared amplitude and returns its square root. Each channel owns one state. At a time constant the amplitude or power covers approximately 63% of a step. A time constant of zero reaches its target immediately.

Use `process_sample(channel, sample)` inside a compressor or gate. Use `process(&mut buffer)` to replace a full block with envelope output. `set_times` changes coefficients without clearing state. The mode and layout are fixed at construction. Sample rates and channels use `ProcessSpec` bounds, and time constants accept 0–60,000 ms. Nonfinite samples are taken as zero. Invalid indices and blocks return an error without changing state.

## Work with audio blocks

Choose a backend once using `BlockMath::best()` or `BlockMath::new(backend)` from `soniclaw::dsp::block_dsp`. Build borrowed `Block` and `BlockMut` views over contiguous data. Copy, add or multiply views with the same frame count, channel count and layout. Use `fill` for a constant or `clamp` with a finite `ClampRange`. The dispatched multiply and clamp loops handle unaligned starts and scalar tails on AVX2, SSE2 and NEON. Scalar and SIMD paths preserve in-range signed zeros for clamp; NaNs remain NaNs. `peak_rms` measures a whole block using f64 accumulation and treats invalid samples as silent.

## Avoid subnormal tails

For an explicitly scoped callback, call `soniclaw::dsp::denormals::with_denormals_flushed(|| render())`. The guard enables the calling thread's flush-to-zero mode and restores the original flush bit when the scope ends, including on unwind. Use nested scopes in stack order. The x86 path enables SSE output flushing; it leaves the separate input DAZ bit untouched. AArch64 uses the FPCR flush bit. The guard is a no-op on other targets. For input values passed in from other threads, use an explicit silence threshold if the application requires it.

```sh
cargo run -p soniclaw-dsp-suite -- dsp-utilities output/dsp-utilities
```

The example gates and clips a signal, then writes its two envelope tracks and reports the selected backend. Run each command with a fresh output directory.

## Ownership, persistence and verification

Allocate the follower and choose a kernel off the audio thread. Process with a single owner for follower state. Rebuild for a new format, and reset on an audio stream discontinuity. Block math is stateless. Layout/shape errors return before writing; adjust the views or choose the scalar backend if a requested CPU feature is unavailable. Persist attack, release and detector mode in the application. Runtime state is transient; no saved sessions or compatibility migration are required.

[Executed verification](validation/v0_120_0/REPORT.md) checks reference step responses, CPU backend equivalence where available, callback allocations, and nested restoration of the flush bit.
