# Soniclaw 0.87.0 verification

Executed 2026-11-20 on Linux x86_64. 71 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Owned VST3 two-input/two-output integration: muted key still ducks native main to0.0625; independent output0.25; zero callback allocations; 1024live/zerooffline bridge latency; native/project save-reopen and routing-cycle rejection. Frame-clock exchange tests. Extra layout/custom descriptor guards remain non-native-compatible. Physical/graphical/third-party acceptance pending.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../STUDIO_NATIVE_BUSES.md).
