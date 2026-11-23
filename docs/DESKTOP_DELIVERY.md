# Desktop audio delivery: 0.16–0.23

Each requested feature was implemented as a separate minor release, with its own release note, detailed feature guide and executed verification report. The latest source includes all eight and preserves earlier evidence.

| Release | Feature | Detailed guide | Verification |
|---|---|---|---|
| [0.16.0](releases/v0.16.0.md) | Direct/buffered CLAP hosting and latency diagnostics | [Low latency](LOW_LATENCY.md) | [Report](validation/v016/REPORT.md) |
| [0.17.0](releases/v0.17.0.md) | Opaque hosted sessions, bypass and missing-product recovery | [Host sessions](HOST_SESSIONS.md) | [Report](validation/v017/REPORT.md) |
| [0.18.0](releases/v0.18.0.md) | Named buses, auxiliary routes and native sidechains | [Flexible buses](FLEXIBLE_BUSES.md) | [Report](validation/v018/REPORT.md) |
| [0.19.0](releases/v0.19.0.md) | Seekable WAV/AIFF/FLAC format manager | [Audio formats](AUDIO_FORMATS.md) | [Report](validation/v019/REPORT.md) |
| [0.20.0](releases/v0.20.0.md) | MIDI wire packets, clock mapping, scheduling, output and learning | [MIDI I/O](MIDI_IO.md) | [Report](validation/v020/REPORT.md) |
| [0.21.0](releases/v0.21.0.md) | Tempo maps, musical positions, loops/punch and automation | [Musical transport](MUSICAL_TRANSPORT.md) | [Report](validation/v021/REPORT.md) |
| [0.22.0](releases/v0.22.0.md) | Embedded editors, native windows, scaling and timer/FD services | [Hosted editors](HOSTED_EDITORS.md) | [Report](validation/v022/REPORT.md) |
| [0.23.0](releases/v0.23.0.md) | Native VST3 hosting, common controls and workbench integration | [VST3 hosting](VST3_HOSTING.md) | [Report](validation/v023/REPORT.md) |

## Current verified scope

The final workspace passes 202 Rust tests, strict Clippy/format/documentation checks and 11 Python tool tests on Linux. Native reference products exercise CLAP and VST3 processing, state, callback ownership and Linux editor rendering. Regression demos cover the earlier desktop additions after the new backend dependencies were integrated.

Direct CLAP hosting and buffered VST3 hosting have different callback/latency models. VST3 adds a fixed 1024-frame bridge delay; direct CLAP adds no worker bridge. Debug correctness/impulse checks are not physical round-trip latency or a fresh JUCE performance benchmark.

## Integration boundaries

- Flexible CLAP multi-bus processing is scoped; the workbench serial rack and new VST3 convenience adapter remain stereo.
- Native AIFF/FLAC support uses runtime libsndfile, which applications must supply.
- CLAP sessions and VST3 state snapshots are separate persistence models; mixed-format rack sessions remain future work.
- CLAP musical transport carries the richer map/meter/loop context. The VST3 live adapter currently uses constant-tempo PPQ derived from sample position.
- Linux X11/XCB editors were exercised under Xvfb/software GL. Native Wayland parenting and Windows/macOS runtime acceptance remain pending.
- Hardware audio/MIDI, external DAW sessions and broad third-party plug-in acceptance require target-machine testing. Native modules are trusted, in-process code; scan child supervision is not live isolation or a security sandbox.

These are explicit delivery boundaries, not a numerical claim of JUCE parity. Each guide records API/lifecycle, persistence/migration, bounds, errors and runnable examples for its feature.
