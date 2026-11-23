# Audio device manager — v0.9.0

`soniclaw-device::DeviceManager` adds a reusable **output** device manager over CPAL. The facade exposes it under `soniclaw::device` with `native`; `device-ui` also enables the reusable egui settings panel. Existing `OutputHost` and `DuplexHost` APIs remain available. Input monitoring/recording and duplex clock drift continue to use DuplexHost; they are not managed/reconnected by this output manager.

## Workbench

Run `cargo run -p soniclaw-workbench -- --tab 9`. The new Audio tab exposes:

- Backend and output device selection, including OS-default selection.
- Default or requested sample rate, channel count and device buffer size.
- A separate maximum internal DSP chunk size.
- Retry policy/interval and explicit permission to use the OS default when a preferred output is absent.
- Refresh, Apply, Save/Load preferences, Play/retry and Stop.
- Current state, negotiated/requested configuration, last error and reconnect count.

Refresh after changing backend selection. Apply updates the manager; applying during playback stops/reconstructs the stream. Save records the displayed preferences in `audio-settings.json` under the startup workbench root. Startup reads a valid existing file without opening an audio device; absent/invalid files use defaults. Explicit Load reports parse errors and changes the draft; Apply activates it. The settings path remains independent of subsequently loaded projects. Preferences are not part of a portable project bundle.

No device opens at startup. Only Play/retry opens a stream. An explicit Stop disables automatic restart. Reconnection creates fresh DSP/stream state and the workbench reseeds its audition chord; held-key/voice history and elapsed playback position are not transferred. It does not promise a seamless or click-free physical-device transition.

## Library use

Construct `DeviceManager::new(backend, preferences)` or its native default. Supply a control-thread factory to `start`, `apply` and `poll`; it receives the `OutputPlan` and a fresh `Arc<Telemetry>` and returns `OutputEngine { engine, receivers }`. Event queues must use that same telemetry. The native backend prepares the engine with the negotiated spec before opening the stream. Processing callbacks reuse fixed storage and record stream errors atomically.

Poll with monotonically increasing `Duration` from an Instant while your application is running. Default polling/retry interval is 1 second (allowed 250 ms–30 seconds). Device enumeration, stream destruction/recreation, parameter construction and DSP preparation happen on the caller's control thread. Enumeration/opening can block that thread; do not run these methods or drop streams on an audio callback. The policy is factored through `OutputBackend`, allowing deterministic mock-device testing.

The manager checks output disappearance/default selection changes and stream error counters. It waits/retries after failure when enabled; otherwise a manual Play/retry is required. An enumeration error alone retains a functioning stream and exposes the error. Fallback is opt-in and never relaxes requested rate/channel/buffer constraints. When the preferred output returns, the next check can switch back. After explicit Stop, polling performs no enumeration or restart until the caller explicitly starts again; manual Refresh remains available.

## Configuration and persistence bounds

Preferences use JSON schema 1 and reject unknown fields, unsupported schema and files over 16 KiB. Explicit save validates before truncating/writing the settings file and syncs it; this is not a crash-recoverable settings transaction. Applications may wrap persistence using their own storage policy.

- Sample rate: default or requested 8–384 kHz; individual processors/devices can impose narrower bounds.
- Channels: default or requested 1–64.
- Fixed device buffer request: 16–65,536 frames; a known reported range is checked before opening.
- Internal DSP chunks: 1–8,192 frames; callback buffers can be split into these chunks.
- Formats: f32, i16 or u16; f32 is preferred among matching configurations.
- Native catalog: up to 256 output devices and 512 configuration ranges per device, with up to 16 per-device warnings.

When a driver reports an unknown buffer range, a fixed size remains a request and stream creation can reject it. Requested frames/rate are not measured round-trip latency. Device selection is backend name + device name + duplicate-name occurrence because this CPAL version exposes no universal persistent hardware identity. Reordered identically named devices can refer to different hardware. Refresh and verify selections when hardware changes.

Device-loss/restart behavior has deterministic mock tests; physical interface disconnect/reconnect, actual latency, and Windows/macOS runtime remain target-machine acceptance work. See [validation](validation/v090/REPORT.md). A configured backend/feature is not evidence of a tested physical device.
