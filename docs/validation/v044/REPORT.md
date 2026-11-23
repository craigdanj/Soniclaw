# Soniclaw 0.44.0 verification

Executed 2026-11-18 on Linux x86_64. 4 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Unified-session tests exercise loops, automation/MIDI timing, live-input validation, zero callback allocations, real recorder finalization and prepared replacement with off-callback destruction. The studio-demo renders a real two-track synth/PCM/mixer/aux session across unequal callback sizes and records through the worker-backed WAV path. Physical device opening was not executed.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../LIVE_SESSIONS.md).

Executed studio-demo: 144000 stereo frames, exact 64/127-frame partition equivalence, zero callback allocations, 16384 finalized recorded frames, and applied replacement revision 2. [Report](studio-demo.json). The optional physical-device path was compiled by all-feature Clippy but not opened.
