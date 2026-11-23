# Eight-feature desktop application series

Requested follow-up to v0.43. Each feature is a separate minor source release with detailed documentation, runnable examples and executed evidence. Planned rows are not delivery or hardware acceptance claims.

| Release | Feature | Status |
|---|---|---|
| 0.44.0 | [Unified live session engine](LIVE_SESSIONS.md) | Complete |
| 0.45.0 | [Disk-streamed arrangement clips](STREAMED_CLIPS.md) | Complete |
| 0.46.0 | [Multitrack recording and takes](MULTITRACK_RECORDING.md) | Complete |
| 0.47.0 | [Track freeze and bounce-in-place](TRACK_FREEZE.md) | Complete |
| 0.48.0 | [Multicore graph processing](PARALLEL_GRAPH.md) | Complete |
| 0.49.0 | [Broader plug-in isolation](EXPANDED_ISOLATION.md) | Complete |
| 0.50.0 | [Accessible audio controls](ACCESSIBILITY.md) | Complete |
| 0.51.0 | [Native PipeWire integration](PIPEWIRE.md) | Complete |

Target-machine hardware, desktop accessibility and external DAW acceptance remain separately recorded.

## Delivery and acceptance

All eight minor source releases include their own guide, release note, example/test workflow and validation report. The final all-feature workspace run passed 278 Rust tests, formatting, Clippy and documentation checks. Native CLAP multi-port, deliberate crash and hung-child recovery runs passed. PipeWire native compilation and callback tests passed, while the private daemon probe was blocked by local native protocol socket setup returning `Operation not permitted`; server routing and physical audio remain pending. Desktop screen-reader certification and external DAW/hardware acceptance also remain pending. Consult the per-feature guides for API scope and bounds.
