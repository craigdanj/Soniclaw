# Broader plug-in isolation (0.49)

The existing headless stereo VST3 isolation remains available. This release adds an independent **CLAP child-process host with multiple audio ports**, persistent audio leases, checkpoint recovery and a bounded callback bridge. Sidechain and auxiliary buses retain explicit identities/channel counts; no bus is silently discarded. This does not add VST3 multibus isolation, remote native editors or an operating-system security sandbox.

## Build and run

```sh
cargo build -p soniclaw-clap-isolation-helper -p soniclaw-isolated-clap-host \
  -p soniclaw-host-fixture
SONICLAW_FIXTURE_MULTIBUS=1 target/debug/soniclaw-isolated-clap-host \
  "$PWD/target/debug/soniclaw-clap-isolation-helper" \
  "$PWD/target/debug/libsoniclaw_host_fixture.so" \
  org.soniclaw.host-fixture new-isolation-report
```

The fixture exposes stereo main input plus a mono key input. The key ducks the main signal; the report verifies the resulting 0.25 peak with default gain 0.5, zero callback allocations and checkpoint reload in a different child PID. Set `SONICLAW_FIXTURE_PROCESS_FAULT=crash` or `hang` only for this deliberately fault-injectable fixture to test containment. Both real child failure paths were executed for this release. Keep these test variables out of normal applications.

## Owner workflow

`host::isolation::Config` requires explicit absolute executable/module paths, product ID, rate, maximum frames, timeout, recovery budget and optional CLAP audio configuration ID. `unsafe Isolated::load` loads native code only in the selected helper. The caller remains responsible for trusting both executable and plug-in. Configuration selection is stopped-time, before layout inspection and activation.

`layout()` returns the exact multi-port layout. `checkpoint()` saves schema-1 opaque state while stopped. `restore_checkpoint` validates the four-MiB state ceiling and reloads it while stopped. Checkpoints do not capture delay buffers, current note voices, native GUI state or host-side parameter changes not saved by the plug-in. Serialize with serde; supply only trusted state resources and impose a file-size bound before parsing in a product.

`start_buffered(pipeline_frames)` returns `BufferedOwner` and a core `AudioProcessor` bridge. Concatenate input buses by layout order within each interleaved frame. Channel width is `max(sum(input channels), sum(output channels))`; output buses use the same concatenation, with remaining channels zero. This makes sidechain routing explicit at the application boundary. It is not a dynamically negotiated stereo adapter.

Retain `BufferedOwner`, process the bridge on the device thread and read its atomic telemetry on control. Stop the device, drop `BufferedOwner` on control to join dispatch and end the child audio lease, then drop the bridge. State/recovery calls reject an active binding. A dropped owner's bridge stays permanently silent, including after a later recovery. Build a fresh bridge after recovery.

`faulted()` and `helper_pid()` are control-only, and can wait behind an in-flight RPC up to its timeout. `recover()` creates a fresh child, requires the original layout/latency, reloads the last checkpoint and increments a bounded attempt count. After the budget is exhausted, reconstruct explicitly; there is no hidden infinite restart. Recovery may also be requested while stopped without a fault. No in-flight DSP history is promised to survive.

## Callback and native-thread contracts

The callback uses the graph's timestamped SPSC bridge: bounded copies, no stdio, native calls, mutexes, waiting or allocation. A dispatcher/worker serializes bounded JSON to the helper. A separate reader enforces response timeout; EOF, malformed messages, protocol errors and timeouts fault and kill the child. Later worker calls are silent. A missed output deadline emits silence at that time, never delayed replay. `BufferedOwner::telemetry()` supplies pipeline misses/overflows; `faulted()` separately reports native/RPC failure. Silence can therefore be produced by a fault even when the packet itself arrived on time.

Inside the child, loading, instantiation, configuration, state and destruction occur on its main thread. `with_bus_audio` activates there and owns a persistent scoped audio thread until Stop. Audio processing remains on that scoped thread. Main-thread service requests are serviced when the lease ends; continuously serviced timers, platform UI pumping and plug-ins requiring those while playing need a richer helper event loop and are not supported here. Native stdout logging can corrupt the stdio protocol and is treated as failure. Stderr is inherited for diagnostics.

## Limits and compatibility

CLAP layout limits apply (up to eight ports each direction); the flattened bridge further caps total channels to eight each direction. Rate 8–192 kHz, maximum block 1–1024, pipeline at least two maximum blocks and at most 262144 frames. Timeout 50–10000 ms; recovery budget 1–8. Messages are bounded to 32 MiB, state to four MiB. Graph input/output queues each hold 32 fixed packets. JSON and background work allocate; only the device callback is allocation-free.

The first bridge forwards MIDI1 note-on/off, CC and pitch bend. Other core note-expression messages are rejected into silence by the background conversion; do not feed them to this bridge. Output native MIDI/parameter events and live parameter edits are not exposed by this public bridge yet. Use stopped opaque state for controls, or the in-process musical/bus APIs where their richer event contract is needed. Dynamic latency/port changes require stopped reconstruction and fresh acceptance testing.

This is an additive host API; existing CLAP/VST3 owners and project schemas are unchanged. Helper/host binaries must be built from the same release; the private JSON wire is not a stable external protocol. Checkpoint schema is separate from existing VST3 recovery checkpoints.

## Validation

Real native runs cover multi-port ducking, child crash, child hang/timeout, fresh-PID recovery and opaque checkpoint restore, each with zero callback allocations. Reports live next to [release checks](validation/v049/REPORT.md). Unit tests cover wire framing, unsupported MIDI rejection and checkpoint validation. Linux fixture success does not establish broad third-party compatibility, GUI support or macOS/Windows runtime acceptance.
