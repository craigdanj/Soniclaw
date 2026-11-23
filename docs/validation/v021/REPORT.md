# Soniclaw 0.21.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64, local execution. **198 workspace/all-feature tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied and documentation with warnings denied passed. checks.json/logs record commands and runtime results.

Analytic tempo/meter/loop/punch and partition invariance tests passed. The demo rendered six seconds of mapped project automation, compared the final parameter with an independent clock and exercised native CLAP PPQ/meter/loop metadata. The native Transport tab opened under Xvfb and was inspected.

Native images when present were captured under Xvfb/software OpenGL and visually inspected. No physical device was opened. Mock tests/offline processing do not establish hardware latency, reconnect reliability, commercial plug-in acceptance, real DAW sessions or Windows/macOS runtime. Those remain target-machine checks. Debug timing is not a performance guarantee or a fresh JUCE comparison. Historical results retain their original scope.

Read the [feature guide](../../MUSICAL_TRANSPORT.md) for workflow, API, examples, lifecycle, persistence, migration, limits and recovery. This minor release was packaged separately before subsequent feature work.
