# Multicore graph processing (0.48)

`Graph::parallel(spec, workers, pipeline_frames)` prepares an acyclic graph, preserves its delay compensation and transfers processors to a fixed worker pool. Independent nodes in a dependency level execute concurrently; a dependent level begins only after all upstream results arrive. Fan-in sums in existing graph edge order, giving deterministic routing. A fixed processor-to-worker assignment keeps processing and reset on the same worker for that processor's lifetime.

## Use

```rust,ignore
let (owner, bridge) = graph.parallel(spec, 4, spec.max_block_frames * 4)?;
engine.push(bridge); // usual AudioProcessor, process on the device callback
// Read owner.telemetry atomics on UI/control.
// Stop the device, remove/drop the bridge, then drop owner on control.
```

Keep `Owner` alive while the bridge is active. Its destructor stops dispatch and joins workers; it must never run on the audio callback. The bridge itself performs bounded copies and SPSC queue operations. It never waits for a worker, joins, calls an arbitrary processor or uses a mutex. Preparation occurs on the caller; custom processors must support subsequent processing/reset/destruction on their assigned worker. Native plug-in main-thread ownership and existing audio leases require their own integration: do not assume moving a native lease into this executor satisfies its contract.

The serial `Graph` remains available. Select serial or parallel mode before playback based on graph size and measured load. This implementation does not transparently execute a late stateful node again on the callback. Such duplicate processing would advance its state twice.

## Timing and deadline behavior

The bridge timestamps every input block with an internal monotonically advancing clock. Outputs are scheduled at that clock plus the fixed pipeline delay. Graph intrinsic latency is added to the advertised `latency_frames()`. Initial pipeline silence is intentional and excluded from missing-frame counts. External transport and bounded MIDI events are passed unchanged to processors, so musical time remains the source time; a host compensates the advertised output latency.

Arbitrary block sizes through the prepared maximum, including zero frames, are accepted. Event validation follows the core contract. If a result is absent at its scheduled frame the bridge emits silence for that frame, increments `missing_frames`, and later discards stale output. It never shifts a late block into the future. Input overflow increments `dropped_inputs`; dispatcher discontinuities reset DSP and routing history before the next accepted input. Bridge `reset` increments a generation, so outputs from an earlier generation cannot sound after reset. The internal deadline clock continues across reset.

`Telemetry` exposes atomic `completed_blocks`, `dropped_inputs`, `missing_frames`, `late_packets` and `failures`. Output queue overflow and discarded packets share the late-packet counter; it is a packet count, not an exact count of lost samples. Worker channel disconnection increments failures. Stopping `Owner` makes subsequent bridge calls silent. Invalid callback blocks also produce silence; they do not advance the deadline clock.

## Bounds and ownership

Limits: 1–16 workers, at most 64 nodes, 1–8 channels, maximum block up to 1024 frames, up to 1024 core events per block. Pipeline delay must be at least two maximum blocks and at most 262144 frames. Input/output SPSC queues each hold 32 fixed packets; each packet reserves 8192 floats and 1024 events. A deep pipeline does not enlarge these queues: choose a delay and pacing compatible with their capacity.

The dispatcher waits off callback for a completed dependency wave. Workers own processor objects and reuse audio buffers. Internal task/message/event storage can allocate on the background threads; this is a callback isolation design, not a claim that every worker operation is allocation-free. An uncooperative or hung in-process processor can block dispatch and owner destruction. Use process isolation for untrusted native code. This executor does not force-kill threads, change thread priorities or promise universal performance gains.

There is no new graph persistence schema. Rebuild from the existing application graph definition when changing worker count, latency, routing or sample rate. All construction errors return before publishing the bridge. Graph edits use an explicitly prepared replacement; workers do not mutate routing during a block.

## Verification

`cargo test -p soniclaw-graph` includes a two-party barrier inside independent DSP branches: the test can finish only when both branches run concurrently. Their dependent summing node verifies the expected output after pipeline latency. Callback allocation checks, startup silence, stopped-owner silence and generation reset are exercised alongside existing serial fan-in/delay-compensation tests. [Release evidence](validation/v048/REPORT.md) records the executed commands. Benchmark actual products on target devices; the tests do not establish a speedup or physical-device deadline guarantee.
