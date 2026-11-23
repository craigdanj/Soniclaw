# Soniclaw 0.71.0 verification

Executed 2026-11-19 on Linux x86_64. 314 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Full-workspace all-feature checks. MIDI example verified exact input offsets across 64/127-frame partitions, zero callback allocations, raw/SMF controller preservation and prepared quantized clip undo/redo/reopening. Headless studio test exercises actual finish/save/append/history actions without hardware. See midi-example.json. Physical input timing/device/native-window acceptance remains unrun.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../STUDIO_MIDI_RECORDING.md).

[Executed MIDI example command and report](midi-example-check.json). The studio headless test includes actual MIDI finish/save/prepared-insert/undo/redo actions; no native device was opened.
