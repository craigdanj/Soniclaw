# Soniclaw 0.66.0 verification

Executed 2026-11-19 on Linux x86_64. 22 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

The routing example rendered actual pre/post-fader paths and verified expected RMS ratios, stale draft rejection, undo and portable reopening. Unit tests cover removed-send automation/value remapping and failed preparation rollback. See routing.json.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../STUDIO_ROUTING.md).
