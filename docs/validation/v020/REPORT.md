# Soniclaw 0.20.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64, local execution. **194 workspace/all-feature tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied and documentation with warnings denied passed. checks.json/logs record commands and runtime results.

Packet parser, timestamp mapping, stable sample scheduling, overflow note recovery, controller learning and output-worker tests passed. The hardware-free demo verified Program Change/Clock/SysEx/CC packets, offsets, zero scheduler allocations and mock scheduled output. Native/virtual port APIs compiled; physical MIDI was not opened.

Native images when present were captured under Xvfb/software OpenGL and visually inspected. No physical device was opened. Mock tests/offline processing do not establish hardware latency, reconnect reliability, commercial plug-in acceptance, real DAW sessions or Windows/macOS runtime. Those remain target-machine checks. Debug timing is not a performance guarantee or a fresh JUCE comparison. Historical results retain their original scope.

Read the [feature guide](../../MIDI_IO.md) for workflow, API, examples, lifecycle, persistence, migration, limits and recovery. This minor release was packaged separately before subsequent feature work.
