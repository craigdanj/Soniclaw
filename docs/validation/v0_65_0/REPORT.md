# Soniclaw 0.65.0 verification

Executed 2026-11-19 on Linux x86_64. 21 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

The export example rendered mix/stems, encoded and decoded WAV/AIFF/FLAC, checked exact frame counts and normalized peaks, and exercised cancellation. See export.json. Native external plug-in exports remain unrun.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../STUDIO_EXPORT.md).
