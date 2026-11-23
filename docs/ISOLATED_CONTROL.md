# Isolated plug-in control and events (0.58)

The CLAP child-process bridge now accepts live UI parameter changes and sample-offset automation, returns MIDI/parameter/gesture events and keeps the child's main-thread services running while its audio lease remains active. The audio-facing adapter exchanges fixed packets; pipe I/O and JSON stay on the worker. This remains a process crash boundary for explicitly trusted native code, with the existing bounded timeout/recovery policy.

## Run the native fixture

```
cargo build -p soniclaw-isolated-control -p soniclaw-clap-isolation-helper -p soniclaw-host-fixture
./target/debug/soniclaw-isolated-control ./target/debug/soniclaw-clap-isolation-helper ./target/debug/libsoniclaw_host_fixture.so NEW_DIRECTORY
```

This Linux software example loads the independent CLAP ABI fixture, queues a UI gain change, automates gain at frame 32, checks both halves of the delayed output, sends/receives MIDI and verifies render/source timestamps while the source clock loops. It measures zero callback allocations, checks main-thread timer/FD dispatch during playback, verifies silence after owner destruction and saves a checkpoint. Separate fixture crash and hang runs exercise recovery into a fresh helper. Reports are included with the release evidence. No native editor window or audio device is opened.

## Owner and callback APIs

`Isolated::start_buffered(pipeline_frames)` now returns `(BufferedOwner, isolation::Processor)`. The latter replaces the previously exposed raw graph Bridge and implements `AudioProcessor`. `Processor::parameters()` and `BufferedOwner::parameters()` return initial remote CLAP parameter metadata: native IDs, names, bounds, flags, default and initial value. These values describe the handshake snapshot; update application UI state from returned events, rather than treating the metadata's initial value as a live meter.

Call `BufferedOwner::set_parameter(native_id, plain_value)` from the UI/control thread. It validates bounds/read-only flags and queues at most 64 commands; exhaustion returns NotPrepared. Commands apply at frame zero of the next processed worker block. Their timing is not sample-accurate relative to the wall-clock UI click.

Call `Processor::process_automated(buffer, context, &[host::ParameterEvent { frame, id, value }])` from audio. Up to 64 sorted, in-range changes fit a block. Values are native plain values, not normalized 0–1 values unless the plug-in's range is 0–1. UI commands precede callback automation at the same frame. Invalid changes clear the output and return an error. Ordinary `process` supplies no automation. Preparation and owner destruction remain off callback.

The adapter attaches the original transport and changes to a fixed control packet tagged with a monotonically increasing render clock. The graph worker matches that clock, discards stale packets after an input drop and restores the actual musical transport for the native processor. A loop/seek therefore cannot attach an earlier block's automation to a later block with the same musical sample position. Reset advances the underlying graph generation while preserving the unique tag clock. Metadata overflow clears the affected input and is counted; unmatched packets produce silence and diagnostics.

## Returned events

Repeatedly call `BufferedOwner::pop_event()` on control to drain `TimedOutput`. Its `event` is the existing `host::OutputEvent`: MIDI, parameter value or begin/end gesture. `render_frame` equals the original monotonic render tag plus configured pipeline delay and event offset. `source_frame` is the original musical sample plus event offset and can repeat after loops/seeks. The event retains its native block offset. Delivery can occur after its render deadline under load; it is a timestamped observation, not an automatic external MIDI scheduler. The host must route/schedule output MIDI using its own transport and latency policy.

The output queue holds 4,096 events and helper replies accept at most 1,024 events per block. Overflow is counted and events are dropped rather than blocking audio. CLAP MIDI1 note/CC/pitch input is supported; unsupported core expressive/note-ID messages still reject at the worker protocol boundary. Other native note dialects, remote editor embedding and automatic MIDI output scheduling are not added here. Parameter/gesture wire roundtrip and invalid-bound tests complement the actual native MIDI/parameter exercise.

## Continuous services and diagnostics

`ClapInstance::with_bus_audio_serviced(rate, max_frames, requests, closure)` lends audio to a scoped audio thread while the owning main thread polls requested callbacks, registered timers and nonblocking FD readiness at roughly one-millisecond intervals. The ordinary bus scope delegates to this serviced path. The scope joins before deactivation or state access, and existing RAII stops processing on unwind. Shared host context and immutable port/parameter metadata remain valid throughout; no mutable instance reference is borrowed concurrently by the two threads.

`BufferedOwner::control` exposes atomics for metadata drops, event drops, packet mismatches, accumulated host request bits and timer/FD dispatch counts. Request bits retain the existing meaning: 1 restart/latency, 2 main callback, 4 process, 8 parameter rescan, 16 flush, 32 dirty state. Observing a restart is a reason to stop/checkpoint/rebuild explicitly; it does not mutate the running bus buffers. Native process failures still set the RPC client's fault state. Graph queue misses and process faults remain visible separately.

## Persistence, limits and recovery

No project schema changes are required. Checkpoints remain schema-1 bounded native state. Stop/drop BufferedOwner before checkpoint, restore or recover. A retained old Processor emits silence after owner destruction. Ordinary recover requires the same layout and latency; a requested topology change needs the separate reconfiguration workflow. UI parameter queues and output observations are transient and are not serialized automatically; save a stopped checkpoint when they must survive recovery.

The helper and parent protocol must come from the same release; new handshake/reply fields deliberately require matching binaries. The flattened bridge still permits eight channels per direction, 1–1,024 frames, 8–192 kHz, a 50–10,000 ms timeout, and an explicit recovery budget of 1–8. Control metadata uses 64 fixed packets. Native parameters are bounded to 4,096; wire messages remain capped at 32 MiB. JSON allocation, stdio, mutexes, timer service allocation and native loading happen on helper/main/worker threads. The audio adapter performs no pipe access, mutex acquisition, heap allocation or join.

[Executed checks](validation/v058/REPORT.md) distinguish the ABI fixture from untested third-party products and physical-device/native-GUI acceptance.
