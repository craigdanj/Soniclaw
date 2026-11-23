# Soniclaw 0.27.0 verification

Executed 2026-11-17 on Linux x86_64. 86 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Independent Linux VST3 protocol fixture verified tempo ramp/PPQ, 3/4 meter, loop [0,.02], recording/playing/cycle flags, stable note ID42 with tuning/volume/pressure, SysEx and program changes, and automation at frame128. Zero device-callback/ingress allocations and zero bridge errors/drops/underruns: [native musical report](native-musical/report.json). Existing native VST3 state/audio/MIDI/revocation regression also passed: [report](native-regression/report.json). Eleven Python tooling tests passed: [log](python-tests.log).

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../VST3_MUSICAL.md).
