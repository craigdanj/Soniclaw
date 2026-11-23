# Soniclaw 0.31.0 verification

Executed 2026-11-17 on Linux x86_64. 87 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Fifteen Python tests passed: [log](python-tests.log). The native v0.31 Linux VST3 bundle and387-dependency supplied attribution were staged into630 inventoried files; architecture/hash/license inventory audit passed: [report](native-package-audit.json). The opt-in pending-acceptance gate correctly returned2: [exit evidence](pending-exit.json). Source/release links and three-platform workflow YAML parsing passed. Remote workflow execution, signing and platform/hardware acceptance remain unrun.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../PLATFORM_RELEASES.md).
