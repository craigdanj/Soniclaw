# Soniclaw 0.28.0 verification

Executed 2026-11-17 on Linux x86_64. 93 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Graph tests verify compensated impulse alignment across variable blocks with zero callback allocations, independent auxiliary/sidechain routing and malformed buffer silence. Native Linux VST3 Sidechain Ducker plus compensated dry path passed 160 packets: baseline2.0, key-active1.17783165, released2.0; latency1024; zero callback allocations/errors/drops/underruns. [Native report](native-sidechain/report.json), [blueprint](native-sidechain/graph.json). The unchanged reference binary was built during v0.27.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../HOST_CHANNELS.md).
