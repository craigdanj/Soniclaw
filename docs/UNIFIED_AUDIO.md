# Unified input/output management — 0.12.0

## What this release changes

DeviceManager now coordinates stereo DSP, selected input capture, output routing, monitoring, MIDI delivery and recording through the same settings/reconnect policy. Its default remains the previous output-only mode. Existing OutputHost, DuplexHost and SessionController APIs remain available.

Enable `DevicePreferences.duplex = Some(DuplexRoute::default())` to use unified mode. The route chooses a default or named input, maps hardware input channels into stereo DSP, and maps stereo DSP back to selected hardware output channels. Input may also be disabled for output-only instrument/effect recording.

## Workbench walkthrough

1. Run `cargo run -p soniclaw-workbench -- --tab 9 --unified`. The flag changes the settings draft; it does not open a device or apply it automatically.
2. Refresh devices, choose the backend and output, and enable Unified input/output and recording. Select a named input, OS default input, or Input disabled.
3. Set input L/R and output L/R hardware channel indices (zero based). `[0,0]` input duplicates one microphone/instrument channel to stereo. Output destinations must be distinct. A map outside the selected device's capabilities is rejected.
4. Choose a shared sample rate and optional device buffer size. Unified DSP always has two channels; the channel-count preference controls the hardware output layout. Apply validates preferences and restarts an already wanted stream; Play opens a stopped stream.
5. Monitoring starts **muted** after every unified start/reconnect. Once a stream runs, Monitor through selected output enables its sound. Recording does not require monitoring.
6. Load an effect project to process the mapped input, or an instrument project to generate notes. In unified mode the workbench no longer replaces effect input with its demonstration sine signal. Its existing audition chord and keyboard events reach the duplex engine through the bounded event queue.
7. Set a new WAV take path, choose dry input or processed output, then Start new take. Existing files are refused. Finalize take stops the writer and makes the completed WAV available. The settings panel shows the last take's captured/written/dropped-frame counts and finalization error, if any.
8. Save preferences to retain routing. Stop disables retries. On device loss or stream error, both streams are closed/reconstructed together; the current take finalizes before the restart. Recording never silently resumes into the same file: start a new take explicitly.

## Library setup

```rust,ignore
use soniclaw::device::*;
let preferences = DevicePreferences {
    sample_rate: Some(48000),
    duplex: Some(DuplexRoute {
        input: None, use_default_input: true,
        input_channels: [0, 0], output_channels: [0, 1],
    }),
    ..Default::default()
};
let mut manager = DeviceManager::new(NativeOutputBackend, preferences)?;
manager.start(clock.elapsed(), &mut |plan, telemetry| {
    // Construct your engine off the callback. Use plan.spec() for its stereo spec.
    // Event receivers must share this telemetry; retain their senders in your app.
    Ok(OutputEngine { engine: make_engine(plan)?, receivers: vec![] })
})?;
manager.set_monitor(true)?;
manager.record(std::path::Path::new("/new/take.wav"),
    soniclaw::record::RecordSource::Processed)?;
```

The snippet's make_engine and clock are application-provided. Call `poll` regularly with a monotonically increasing Duration; it performs control-thread enumeration/recovery. `set_bypass` selects mapped dry input while continuing DSP/event processing, so note-offs are not discarded while bypassed. `stop_recording` finalizes a take and publishes its summary. Explicit `stop` closes streams and disables restart; read `status().last_recording` before dropping the manager to retain/report finalization failures. Drop finalizes as a fallback but cannot return an error to a caller that discarded the manager.

## Negotiation and routing

`OutputInventory` now includes inputs and a default_input selector using the existing capability structure. A unified plan resolves default/named input and output selectors together. It checks a common requested/default-output rate, input/output maps and buffer constraints before constructing a stream. If the default output rate is unsupported by the input, choose an explicit common rate; no hidden sample-rate conversion is negotiated.

`OutputPlan.channels` is the DSP channel count (2 in unified mode). `hardware_channels` is the selected physical output layout. The manager uses the smallest matching output layout fitting the map unless a channel-count preference is explicit. Only mapped output channels receive sound; others are zeroed. Input is converted from f32/i16/u16 and routed into the existing bounded stereo clock-drift bridge.

Fallback remains opt-in. It may choose OS-default devices when a selected input/output is unavailable, but it never relaxes rate/map/buffer requirements. A returning preferred device or changing default input/output can cause a coordinated restart. Name plus duplicate-name occurrence remains the selector; it is not a universal hardware serial identity. Reordered identical names must be verified by the user.

The bridge uses bounded prefill and small adaptive interpolation to absorb normal independent-clock drift. It does not support unrelated configured sample rates or guarantee glitch-free recovery. Prefill adds software delay beyond hardware/OS latency. Device buffer preferences are requests; the UI is not a round-trip latency measurement.

## Stream and callback lifecycle

NativeOutputBackend returns ManagedStream::Output or ManagedStream::Duplex. Managed duplex owns the input/output host and a recorder. Both native stream owners remain on the caller's control thread. DSP/resource construction, CPAL stream creation/destruction, catalog enumeration and recording start/finalization are control-thread operations and can block; do not call them inside processing.

`DuplexHost::start_with_events` adds up to eight EventReceiver sources. The original `start` delegates with no event sources. The shared output/duplex drain consumes a bounded, fair event budget (128 events including a reserved panic slot) and clears stale queued note-ons when panic is requested. EventSender delivers messages at block boundaries; it is not a timestamped MIDI driver. Existing sample-offset Engine automation remains separate.

Prepared callbacks reuse dry/wet PCM and event storage. They sanitize capture values, process the engine, optionally select bypassed dry audio, send bounded dry/processed packets to the recorder, then apply monitoring and hardware output maps. Recording captures before monitor mute and before physical channel mapping. The writer owns file I/O; no callback writes to disk.

## Recording and recovery contract

The existing worker records stereo PCM16 WAV. Dry means mapped input; processed means the engine output (or dry when bypassed). Bounded queue loss is counted and represented by silence to preserve recording time. Captured/written/dropped/write-error counts are retained in RecordingReport. Start/finalization failures are surfaced as errors. A retained error is not cleared by a successful audio reconnect.

OutputBackend adds an overridable `close` hook returning an optional RecordingReport; mock/alternate backends may implement their own finalization. The default drops the stream. DeviceManager invokes the hook before retry/reconfiguration, explicit Stop and destruction. After a native stream loss, monitoring resets to muted, DSP/voice/transport history is rebuilt, and the finished take remains a separate file. There is no automatic take concatenation or recording resume across disconnects.

A full disk or filesystem failure can prevent a clean header/data finalization. The API preserves the error/counts but cannot recover unwritten data or promise a valid WAV after arbitrary power loss. Preserve the failed take and error for manual inspection. Physical unplug/clock/disk-full acceptance remains target-machine work; mock tests exercise policy rather than actual drivers.

## Persistence and migration

Device preference schema remains 1 with an optional `duplex` field, omitted in output-only mode. Old saved output preferences therefore keep their original behavior. Older readers reject settings files with the new field enabled. Projects and exported product/parameter IDs are unchanged; device routing stays application-local, outside portable project bundles.

DuplexOptions gains host, input_occurrence, output_occurrence and output_hardware_channels fields. Existing Rust literals must initialize them or use `..Default::default()`. Defaults preserve the previous platform-default host and uniquely named-device behavior. Direct NativeOutputBackend callers now receive ManagedStream instead of a bare OutputHost. Applications using DeviceManager need no separate stream-owner code.

The old output-only manager continues supporting mono/multichannel DSP. Unified mode intentionally exposes stereo processing with hardware channel maps, not arbitrary multibus/sidechain negotiation.

## Validation and runnable walkthrough

Run `cargo test -p soniclaw-device --all-features`. New tests cover shared-rate/maps, input loss closing a take, finalization-error retention, coordinated retry, input-disabled output recording, backward settings and bounded panic/event draining. Existing drift/prefill tests still run.

`cargo run -p soniclaw-sdk-tools --features device-tools -- --duplex-demo /new/demo` uses a synthetic capability catalog, processes generated input through Tape Colour, writes/finalizes a 25,600-frame WAV and emits settings/JSON evidence. It opens no physical device. Native UI captures and exact full-workspace checks are in the [validation report](validation/v012/REPORT.md).
