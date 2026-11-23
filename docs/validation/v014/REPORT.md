# Soniclaw 0.14.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64, local execution. **177 workspace/all-feature tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied and documentation with warnings denied passed. checks.json/logs record commands and runtime results.

Two-node native ABI fixture verified exact bridge delay and serial gain, live output events, main-thread callbacks, parameter flush and floating-editor lifecycle. The independent host contract and native Host panel capture passed. Fixture GUI is a mock; no physical device or third-party editor window was tested.

Native images when present were captured under Xvfb/software OpenGL and visually inspected. No physical device was opened. Mock tests/offline processing do not establish hardware latency, reconnect reliability, commercial plug-in acceptance, real DAW sessions or Windows/macOS runtime. Those remain target-machine checks. Debug timing is not a performance guarantee or a fresh JUCE comparison. Historical results retain their original scope.

Read the [feature guide](../../LIVE_CLAP_RACK.md) for workflow, API, examples, lifecycle, persistence, migration, limits and recovery. This minor release was packaged separately before subsequent feature work.
