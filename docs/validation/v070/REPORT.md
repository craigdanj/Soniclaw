# v0.7.0 fresh Linux verification — 2026-11-17

Recovered source rebuilt with Rust 1.100.0 on Linux x86_64. Previous transient runtime results are not used as evidence here.

- Workspace tests: **121 passed**, zero failures, all features.
- Formatting, all-target/all-feature Clippy with warnings denied, and documentation with warnings denied passed.
- CLAP ABI smoke tests passed for legacy, project, DSP lab and four workbench products.
- 36 rate/block combinations across multiband, polyphonic modulation and memory sampler passed finite-output, callback heap and stopped-state checks. Debug timings include overruns and are not real-time performance guarantees.
- Independent MIDI-only CLAP fixture passed separate main/audio OS thread lifecycle, timed gain, 19 output events, main callback, transactional bounds rejection and 16-byte state round trip. Host callback allocation count was zero.
- Hardware-free SDK walkthrough produced six preset renders, per-voice/reference audio, MIDI file round trip, quantization, sample-map slices and prepared live-graph replacement.
- Six native workbench panels captured under Xvfb/software OpenGL; graph, sample-map and voice panels visually inspected. No physical sound device opened.
- App, CLAP and sampler templates checked with default features under a path containing spaces; see templates.log.

Raw JSON and logs accompany this report. Render artifacts live in examples/sdk-tools/rendered and host-rendered. Commands are in checks.json and SDK_070.md. Existing v0.6 CPU data remain historical; no new CPU comparison or JUCE parity claim is made.

Physical audio-device latency, commercial plug-in compatibility, real DAW sessions and Windows/macOS runtime acceptance remain untested here. Hosting is an in-process scoped rendering foundation with no hosted GUI, crash sandbox or persistent live device adapter.
