# Optimized DSP kernels (0.41)

## API and workflow

`soniclaw::kernels` reexports the small standalone `soniclaw-kernels` crate. Prepare a `Kernel::best()` on a control thread, or choose `Kernel::new(Backend)` explicitly. The automatic policy selects AVX2, SSE2, ARM64 NEON, then scalar according to CPU availability; it selects the widest implemented instruction set, not a runtime benchmark winner. `backend()` reports the actual selection. Unsupported explicit choices return `UnsupportedBackend`, never execute instructions speculatively.

```rust
use soniclaw::kernels::Kernel;
let kernel = Kernel::best();
let mut samples = [1.0, -1.0, 0.5];
kernel.gain(&mut samples, 0.25).unwrap();
assert_eq!(samples, [0.25, -0.25, 0.125]);
```

Operations are in-place gain, scaled mix/add, dot product and stereo interleave. Slice lengths are validated before writes; failures leave destinations untouched. Gain must be finite. Dot products round each product to f32, then accumulate in source order in f64 across all backends. Mix uses separate multiply/add, without FMA. Tails of any length use scalar code. Unaligned slices work; safe Rust's borrowing enforces mutable alias restrictions. Empty slices are valid. There is no alignment, minimum block or padding requirement.

The existing `dsp::Gain` selects its kernel in `prepare` and uses it when gain smoothing is settled; the changing-gain path retains sample-by-sample smoothing. Gain zero/unity still use clear/bypass. This makes the optimization available to existing graph/project users without changing their parameter or output contracts.

## Lifecycle, safety and numerical scope

Kernel is Copy and has no heap state, locks or I/O. CPU feature detection occurs during explicit selection, not per process call. Public methods remain safe; checked target-feature functions and their unaligned pointer accesses are isolated in this crate. Other workspace crates retain the unsafe-code prohibition. Source comments state every vector access proof; loops prove full vectors fit and scalar tails avoid out-of-bounds accesses.

PCM values are not scanned for finiteness or clipped. NaN/infinity/overflow propagate through ordinary floating-point arithmetic. Subnormal handling follows the process/CPU environment; this crate does not change floating-point control registers. Fast-math flags supplied externally can invalidate strict numerical expectations. Thread migration assumes the OS exposes a consistent supported CPU feature set. Prepared kernels can be shared/copied between processing owners without synchronization.

No new project schema is needed: kernels are execution policy, not sound state. Scalar remains available for reproducible investigations. Persisting a preferred Backend is application configuration; validate it again on each machine and fall back explicitly if unavailable.

## Benchmark and validation

`cargo run --release -p soniclaw-kernel-bench -- NEW_DIRECTORY` performs gain/mix/dot/interleave measurements on 4096 samples with 1000 calls, one warmup and seven recorded rounds per supported backend. It writes all timings, medians, platform and selected backend to JSON. The scalar compiler may auto-vectorize, so SIMD does not imply a speedup for every operation/size. These measurements do not predict overall plug-in/DAW CPU use. Do not compare debug timings with release runs.

Tests cover every supported backend at sizes 0–259, deliberately unaligned source/destination subslices, tails/sentinels, scalar parity for all operations, zero processing allocations and invalid-shape/gain non-mutation. ARM64 code is cross-checked separately when the target is installed; this does not execute NEON. See the verification report for actual Linux benchmark results and cross-check scope. No AVX-512, ARM32 NEON, GPU kernels or automatic per-CPU benchmarking is included.
