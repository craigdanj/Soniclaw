# Soniclaw 0.34.0 verification

Executed 2026-11-17 on Linux x86_64. 80 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Waveform cache tests cover channel/partial-bucket extrema, corruption repair, same-timestamp content invalidation, cancellation/latest publication, empty files, hostile counts, worker shutdown and owned-cache eviction. The60second stereo streaming demo passed: 2880000frames, 23040000bytes source PCM vs360064bytes peaks,15 levels, cold build then disk hit and latest asynchronous hit, plus256 detail samples: [report](streaming-demo.json). The explicitly ignored native libsndfile backend test was executed separately on Linux and passed: [log](native-formats.log). An actual native viewer loaded the cache and rendered both channels: [GPU capture](native-overview.png), [metadata](native-overview.json). The optional-codec test remains ignored in the general suite when codec availability is unknown.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../WAVEFORM_CACHE.md).
