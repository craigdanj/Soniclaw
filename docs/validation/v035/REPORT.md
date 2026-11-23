# Soniclaw 0.35.0 verification

Executed 2026-11-17 on Linux x86_64. 12 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Five clip tests cover exact MIDI offsets, seek/stop ownership, loop/fade audio, partition invariance, allocation-free playback, latency compensation and transactional edits. The clip-studio executable renders a real ExpressiveSynth arrangement at 64/256-frame block sizes; native editor evidence is recorded separately.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../CLIPS.md).

## Executed native demo

Rootless Xvfb/Linux native OpenGL editor opened, rendered and closed through its screenshot event. [Captured editor](clip-studio.png). A five-second/240,000-frame stereo arrangement using ExpressiveSynth plus looped PCM rendered with peak 0.16483231 and exactly equal samples for 64/256-frame partitions. No physical audio device was used.
