# Soniclaw 0.32.0 verification

Executed 2026-11-17 on Linux x86_64. 84 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Typed mapping/quantization/enum/snapshot/ID-collision and allocation-free numeric operation tests passed. The real typed-effect CLAP export passed stepped/enum flags, physical cutoff text200.000Hz, parsing1000Hz to normalized.5663234, gain automation at frame128 and bypass at192: [report](native-typed/report.json). Direct shared-factory DSP recorded zero callback allocations. The native X11 editor passed size/scale/focus/close/reopen with the typed controls: [report](native-editor/report.json). No physical interface or external DAW acceptance is implied.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../TYPED_PARAMETERS.md).
