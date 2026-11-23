# Soniclaw 0.37.0 verification

Executed 2026-11-18 on Linux x86_64. 10 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Mixer tests cover constant-power balance, send gain, meter consumption, allocation-free processing, exact automation across partitions, invalid-event silence/rollback and group mute. A standalone mixer-studio example exports mix and send stems.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../MIXER.md).

Executed mixer-studio on Linux: 96,000 stereo frames exported as mix.wav and send.wav. The two asymmetric pan positions produced per-strip channel peaks 0.14782073 / 0.06122936 and the reverse. No physical audio device was used.
