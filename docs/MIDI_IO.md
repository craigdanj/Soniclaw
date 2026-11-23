# MIDI I/O, packet scheduling and controller learning (0.20)

The new MIDI layer adds complete bounded **MIDI 1 wire packets**, timestamp-to-sample mapping, a prepared scheduler, hardware output on a separate worker, virtual ports where supported and reusable/persisted absolute-CC learning. Existing `midi::Parser`, `native::connect_input`, core MidiMessage and Standard MIDI File APIs remain source-compatible with their previous subset contracts.

## Workflow and runnable demo

```sh
cargo run -p soniclaw-sdk-tools -- --midi-io-demo /tmp/new-midi-io-demo
cargo test -p soniclaw-midi --all-features
```

The demo parses Program Change, Clock, SysEx and CC, schedules packets at known sample offsets with no callback allocations, learns a parameter mapping, persists its JSON and sends through a mock output worker. It writes packet/offset/worker statistics and explicitly marks hardware untested. Native port functions compile with `soniclaw/midi-input` or `soniclaw-midi/native`; opening physical/virtual ports requires a running native MIDI backend and target-machine permission/device availability. No physical MIDI timing was measured in the demo.

For an application:

1. Open native packet input on a control thread; keep the returned connection alive.
2. Calibrate a TimestampMapper with a driver timestamp and matching audio sample position. Pass complete packets/times through a bounded queue to your audio owner.
3. Enqueue absolute sample positions into a prepared Scheduler and drain each audio block. Each output contains its local frame and complete wire packet.
4. Convert supported channel packets with `Packet::to_core()` for the existing engine, or handle raw packets explicitly. SysEx/Program Change/system messages remain raw because core MidiMessage does not represent every wire message.
5. For external output, construct an output Clock anchored to the audio timeline and a worker/Sender pair. Feed ordered sample positions to Sender; the worker performs blocking driver sends. Keep the worker owner on a control thread and drop/stop it there.
6. Learn a CC/parameter pair, save its LearnMap in your application's settings, compile target Arcs before playback and apply CC changes at their chosen engine sample offsets.

## Complete bounded wire packets

`midi::wire::Packet::new(bytes)` validates exactly one complete MIDI 1 message: channel voice messages (including Program Change), supported system-common/realtime messages, or complete F0…F7 SysEx. Channel/system data bytes must be seven-bit. Undefined/reserved statuses are rejected. The maximum packet is 1,024 bytes including status/terminator. Packet owns a fixed array and is Copy; `bytes()` exposes only the actual length. `all_notes_off(channel)` constructs CC123 for channels 0–15.

`wire::Parser::feed(byte)` handles running status, channel lengths, system messages and bounded SysEx. Interspersed realtime messages are emitted immediately without breaking a partial message/SysEx. Oversized SysEx is discarded until termination and increments dropped; a later valid message still parses. Running status is cleared by system-common/SysEx status. This is a separate parser from the legacy core-message parser, so old users do not suddenly receive new event types.

`to_core()` maps supported note, CC, pressure and bend messages to the current core enum, including NoteOn velocity zero as NoteOff. It returns None for unsupported core representations. It does not silently convert SysEx into notes or add Program Change to old DSP APIs. CLAP/VST adapter support for raw SysEx remains format-specific; a complete hardware packet API is not a claim that every exported/hosted format now routes every message.

```rust
use soniclaw::midi::wire::{Packet, Parser};
let program = Packet::new(&[0xC0, 9])?;
assert_eq!(program.bytes(), &[0xC0, 9]);
let mut parser = Parser::default();
let packets: Vec<_> = [0xF0, 1, 0xF8, 2, 0xF7]
    .into_iter().filter_map(|b| parser.feed(b)).collect();
assert_eq!(packets[0].bytes(), &[0xF8]);
assert_eq!(packets[1].bytes(), &[0xF0, 1, 2, 0xF7]);
# Ok::<(), soniclaw::core::Error>(())
```

## Timestamp mapping and sample scheduling

`TimestampMapper::new(driver_micros, audio_sample, sample_rate)` defines an explicit anchor. `sample(driver_timestamp)` rounds to the nearest corresponding audio sample, rejects negative/overflow results and timestamps more than 24 hours from the anchor. Re-anchor after driver reconnect, sample-rate changes or timeline discontinuities. Backend driver timestamp origins differ; do not assume they equal wall time or `Instant`. This mapping does not estimate unknown hardware latency or clock drift automatically.

`Scheduler::new(capacity)` allocates a fixed heap for 1–4,096 packets. `enqueue(absolute_sample, packet)` permits out-of-order arrival and maintains stable order for equal timestamps. `drain(block_start, frames, output)` fills caller-owned Scheduled records; use its returned count. Block end is exclusive, late events clamp to frame zero and increment late, future events remain queued. Frames must be 1–8,192 and end arithmetic must not overflow. A short output slice leaves due events queued; subsequent calls may report them late. Choose capacity/output budgets for your application.

Queue overflow returns InvalidEvents, increments overflow, clears stale queued packets and requests CC123 on all 16 channels. `panic()` does the same without overflow accounting. Panic is emitted before newly queued data; if output capacity is smaller than 16, remaining panic channels carry into subsequent drains. It is note-release recovery, not CC120 all-sound-off, and hardware sustain behavior may affect the audible result. No callback heap allocation/locks are used after preparation, including overflow errors. Fixed packet copies/heap operations are bounded but still cost CPU; measure large SysEx bursts.

## Native input and virtual ports

`packets_native::connect(index, callback)` uses the existing input inventory indices, disables MIDI filtering and invokes `callback(driver_microseconds, Packet)` for every parsed packet. The driver callback owns a bounded wire parser. User callback work must be bounded; bridge rather than touching DSP from the MIDI thread. Port indices are enumeration positions, not stable hardware serial IDs; enumerate again after reconnect.

On Linux/macOS, `packets_native::create_virtual(name, callback)` and `output::native::Destination::Virtual(name)` expose midir's native virtual-port support. Availability depends on ALSA/CoreMIDI and the running session; failure is returned. These APIs are not exposed on Windows because that backend does not supply equivalent built-in virtual-port creation. A separately installed Windows virtual driver can appear as an ordinary enumerated Port.

`output::native::ports()` returns output names. `connect(Destination::Port(index), capacity, clock)` creates the native connection inside the output worker, reports initialization errors before returning and drops it on that worker. Native driver calls do not run on the audio callback.

## Output worker and timing limits

```rust,ignore
use soniclaw::midi::output::{Clock, native};
let clock = Clock::new(48_000.0, audio_sample_now, audio_instant_now)?;
let (mut worker, mut sender) = native::connect(native::Destination::Port(0), 256, clock)?;
sender.send(audio_sample_now + 480, packet)?;
let diagnostics = sender.stats();
worker.stop();
```

The Sender uses a fixed 1–4,096-capacity SPSC queue and requires nondecreasing sample positions. Out-of-order timestamps fail without changing the queue. A full queue increments dropped, requests note panic and clears the Sender's ordering anchor. The worker discards stale queued/pending packets when servicing panic, sends CC123 on all channels, then resumes. Dropping the Sender or owner requests stop; stopping joins off the callback, cancels pending work and sends shutdown note panic. Reconstruct the worker for a new clock/device. The clock accepts sample rates 1–768,000 Hz and deadlines within 24 hours of its anchor.

`OutputWorker::start(capacity, clock, factory)` accepts an application Sink factory for testing/custom backends. The Sink is created, used and destroyed on the output worker; it may block/allocate. Sender success/failure paths use fixed storage and typed core errors. `stats` exposes sent, dropped, send errors and messages more than 1 ms late. The worker waits in at most 1 ms increments so stop/panic can be observed between waits. OS wakeup, driver buffering and hardware introduce jitter: this is deadline scheduling, **not sample-accurate physical MIDI transmission**. A blocked native driver/Sink can delay stop. Worker scheduling thread priority is not automatically elevated.

## Controller learning and persistence

`learning::LearnMap::learn(channel, cc, parameter_id)` creates/replaces the mapping for that stable target. Mapping has exact channel/controller and a normalized `[low, high]` output range. Up to 128 targets are supported, channel 0–15, CC 0–127, IDs ≤256 bytes, finite ordered normalized bounds 0–1. A source may intentionally control several distinct targets. The first implementation supports absolute seven-bit CC, not relative encoders, takeover modes or NRPN learning.

Serialize LearnMap with serde in your application's versioned settings. `compile(parameters)` validates every ID and resolves Arc<Parameter> targets before playback. `LearnedControls::apply(MidiMessage::ControlChange)` maps the value into the parameter's physical range and atomically sets it. The caller controls timing: invoke at the correct sample offset or explicitly choose block-boundary updates. Applying a whole batch before processing a block is not sample-timed learning. Project automation can also own those parameters; define precedence rather than letting competing sources fight.

No ProjectState schema migration occurs; old MIDI/parser/file APIs keep their contracts. Learn maps and packet scheduler state are not automatically inserted into existing project JSON. Persist maps in a schema/versioned application envelope; rebuild bindings after graph edits. Queues, clocks, native connection handles, running parser bytes and active note recovery are runtime state and must not be serialized as reusable device identities.

See [release notes](releases/v0.20.0.md), [verification](validation/v020/REPORT.md) and [desktop roadmap](DESKTOP_ROADMAP.md). Hardware MIDI loopback/jitter, virtual-port availability and native Windows/macOS execution remain pending where not run. MIDI 2/UMP and MIDI-CI are future extensions.
