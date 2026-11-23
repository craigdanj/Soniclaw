# Soniclaw 0.38.0 verification

Executed 2026-11-18 on Linux x86_64. 70 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Authoring tests exercise pitch/crossing inference, normalized sample peak, native/SFZ zone and round-robin roundtrip, invalid maps and output overwrite refusal. Executed sample-author demo restored three generated SFZ zones through the actual project sampler and rendered preview.wav. A440 was estimated at 440.00350 Hz with correlation 0.99803; the export decoded 288,000 PCM bytes.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../SAMPLE_AUTHORING.md).
