# Soniclaw 0.145.0 verification

Executed 2026-11-21 on Linux x86_64. 108 selected Rust tests passed. Formatting, all-target/all-feature Clippy and rustdoc passed with warnings denied within the scope recorded below. [Exact commands and timings](checks.json).

The [float64 WAV and Studio fixture](example.json) preserved 2^-40 after compensated mixing and file round-trip. The [native plug-in fixture](native-precision.json) ran owned headless CLAP and VST3 binaries on Linux and preserved the expected sub-f32 difference. Both negotiated f32 compatibility and measured zero Rust host-side callback allocations. The example did not exercise a physical device, GUI editor, AUv2 bundle, or a proprietary DAW.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../DSP_ENGINE_PRECISION.md).
