# Soniclaw 0.25.0 verification

Executed 2026-11-17 on Linux x86_64. 79 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Native three-format processing, state, bypass and missing-node demonstration: native-mixed/report.json and native-mixed.log. The SDK keeps native processing on a separate audio thread while the owner pumps main-thread requests.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../MIXED_SESSIONS.md).
