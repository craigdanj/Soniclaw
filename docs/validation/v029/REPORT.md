# Soniclaw 0.29.0 verification

Executed 2026-11-17 on Linux x86_64. 87 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Invalid isolation policy is rejected without native execution. Linux native acceptance passed external helper termination, injected native abort, injected2second DSP stall with200ms ordinary-response watchdog, application survival, silence after faults, zero callback allocations, fresh helper PID, restored gain0.4, stale callback revocation and recovery-budget quarantine. Hang-induced queue drops/underruns are expected and recorded: [report](native-isolation/report.json), [run log](native-isolation.log), [checkpoint](native-isolation/checkpoint.json).

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../PLUGIN_ISOLATION.md).
