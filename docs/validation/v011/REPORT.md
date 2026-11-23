# Soniclaw 0.11.0 verification — 2026-11-17

Rust 1.100.0, Linux x86_64; local execution only.

Final workspace/all-feature suite: **162 tests passed**, zero failures. Formatting, all-target/all-feature Clippy with warnings denied, and documentation with warnings denied passed. checks.json records the exact build/runtime sequence; corresponding logs and generated walkthrough reports accompany this release.

Velocity endpoint/layer tests, allocation-free release and sustain/note-ID/choke tests, SFZ inheritance/ranges/rejection tests, and onset/slice tests passed. The generated SFZ instrument rendered, persisted and produced a transient report. Existing four-product workbench CLAP smoke checks passed. The expanded sample map was captured and inspected.

Native images, when present, were captured under Xvfb/software OpenGL and visually inspected; no physical audio device was opened. Offline rendering and mock tests do not establish actual device latency, physical reconnect reliability, commercial plug-in compatibility or Windows/macOS runtime acceptance. Real DAW and hardware checks remain pending unless a separate target-machine evidence record says otherwise. Debug timing on shared compute is not a performance guarantee or a new JUCE comparison.

Read the [feature guide](../../ADVANCED_SAMPLER.md) for workflow/API, lifecycle, bounds, migration and errors. Historical reports are preserved with their original scope. This feature was packaged separately from subsequent features.
