# Soniclaw 0.15.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64, local execution. **179 workspace/all-feature tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied and documentation with warnings denied passed. checks.json/logs record commands and runtime results.

The structured suite passed all 18 executed checks, including three exported workbench ABI reopen cycles, catalog/sampler/automation demos, independent and live-rack host contracts, processor audits and a child-process native metadata probe. Eight Python tooling tests passed in addition to the Rust suite. All three platform acceptance records are explicitly skipped/pending; no real device/DAW or other-platform acceptance was fabricated. Audit CPU timing is descriptive debug-build evidence and is not a callback deadline guarantee.

Native images when present were captured under Xvfb/software OpenGL and visually inspected. No physical device was opened. Mock tests/offline processing do not establish hardware latency, reconnect reliability, commercial plug-in acceptance, real DAW sessions or Windows/macOS runtime. Those remain target-machine checks. Debug timing is not a performance guarantee or a fresh JUCE comparison. Historical results retain their original scope.

Read the [feature guide](../../COMPATIBILITY_SUITE.md) for workflow, API, examples, lifecycle, persistence, migration, limits and recovery. This minor release was packaged separately before subsequent feature work.
