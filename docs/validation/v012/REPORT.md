# Soniclaw 0.12.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64, local execution. **166 workspace/all-feature tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied and documentation with warnings denied passed. checks.json/logs record commands and runtime results.

Input/output negotiation, shared-rate and map tests, input-loss close/retry and retained finalization-error tests passed, alongside existing output/device/drift tests. The common bounded MIDI/panic drain was checked. The synthetic duplex walkthrough produced a clean 25,600-frame processed take and persisted settings. The unified Audio panel was captured and inspected.

Native images when present were captured under Xvfb/software OpenGL and visually inspected. No physical device was opened. Mock tests/offline processing do not establish hardware latency, reconnect reliability, commercial plug-in acceptance, real DAW sessions or Windows/macOS runtime. Those remain target-machine checks. Debug timing is not a performance guarantee or a fresh JUCE comparison. Historical results retain their original scope.

Read the [feature guide](../../UNIFIED_AUDIO.md) for workflow, API, examples, lifecycle, persistence, migration, limits and recovery. This minor release was packaged separately before subsequent feature work.
