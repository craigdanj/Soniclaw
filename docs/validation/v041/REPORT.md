# Soniclaw 0.41.0 verification

Executed 2026-11-18 on Linux x86_64. 44 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

All supported Linux kernels matched scalar for sizes 0–259, unaligned tails, dot/interleave/gain/mix and invalid-argument non-mutation, with no processing allocations. ARM64 kernel/all-target Clippy cross-check passed; NEON runtime was not executed. Linux release microbenchmark results are included in benchmark.json and are operation-specific, not whole-application speedups.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../DSP_KERNELS.md).

## Executed kernel evidence

ARM64/all-target Clippy cross-check passed; this compiled NEON and did not execute it. [Log](arm64-cross-clippy.log). Linux release microbenchmark: 4096 samples x 1000 calls, median of seven rounds after warmup. [All timings](benchmark.json). The benchmark ran the same kernel implementation before the release metadata bump.

| Backend | Gain ns | Mix ns | Dot ns | Interleave ns |
|---|---:|---:|---:|---:|
| Scalar | 499980 | 412723 | 2805029 | 933923 |
| Sse2 | 189319 | 324509 | 2789625 | 1124006 |
| Avx2 | 317403 | 355026 | 3160232 | 1056292 |

These are local kernel timings, not a whole-application/DAW comparison. Wider SIMD was not faster in every case.
