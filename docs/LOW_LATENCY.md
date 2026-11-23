# Low-latency hosting and diagnostics (0.16)

Soniclaw now offers direct trusted CLAP processing alongside configurable asynchronous buffering. The direct path removes the framework's audio bridge delay; device/driver buffering, plug-in latency and duplex input prefill still contribute to physical latency. No hardware round-trip measurement is claimed by this release.

## Workbench workflow

1. Open Host, scan an explicitly trusted library and add products to the rack.
2. Select **Direct low-latency mode** before Start. This is the default for newly opened workbench sessions. Direct mode processes native code on the device audio callback; a slow plug-in can cause a dropout and a hung plug-in can block the callback.
3. Disable Direct to use buffered mode. Choose 1–7 lookahead packets, each 256 frames: 256–1,792 software frames. Four packets preserve the old 1,024-frame behavior. Restart to apply mode or delay changes.
4. Start the rack, use generic parameter controls and inspect the diagnostics. `bridge_latency_frames` is zero in direct mode. `max_callback_ns` and `deadline_misses` describe direct plug-in callback work, not the entire device pipeline. Buffered input/output/event loss counters retain their previous meaning.
5. Stop closes the audio stream, revokes processing access and destroys plug-ins on the owning GUI thread. Stop can wait for an in-flight native call; native crashes/hangs are not sandboxed.

A lower buffered delay reduces scheduling slack. It is not a promise of glitch-free operation. The mode/buffer selection is currently a workbench control and is not yet a persisted device preference. The rack-session release adds its own versioned host state separately.

## Runnable example and evidence

```sh
cargo build -p soniclaw-sdk-tools -p soniclaw-host-fixture
cargo run -p soniclaw-sdk-tools -- --direct-demo \
  target/debug/libsoniclaw_host_fixture.so /tmp/new-direct-demo
cargo test -p soniclaw-host
```

Use `.dll` / `.dylib` filenames on Windows/macOS. Those commands require a native toolchain and are not evidence of execution on those targets. The demo instantiates two independent fixture effects, sends gain 0.8 to one and retains gain 0.5 on the other, then checks output 0.4 starting at sample zero. Callback sizes are 1, 7, 64, 255, 257 and 512. It checks callback allocations, native thread/lifecycle rules and silence after owner-first destruction while the callback half still exists. Synthetic impulse alignment is also exercised. WAV/report output is created in a new directory. Tests and current runtime results are in [verification](validation/v016/REPORT.md).

## API

```rust,ignore
let spec = soniclaw::core::ProcessSpec {
    sample_rate: 48_000.0, channels: 2, max_block_frames: 512,
};
// instances were created on this thread from explicitly trusted CLAP libraries.
let (mut owner, callback) = soniclaw::host::live::LiveRack::start_direct(instances, spec)?;
// Move callback into a prepared device Engine. Keep owner on its creation GUI thread.
owner.poll();
let diagnostic = owner.stats().snapshot();
// Stream drop/stop normally happens first; owner revocation also protects the inverse order.
owner.stop();
```

`LiveRack::start_direct` accepts 1–8 deactivated instances, stereo f32, a valid sample rate and a maximum block of at most 8,192 frames. Every instance must accept the layout. Preparation of the callback must match rate/channels and not exceed the activation maximum. The callback retains existing validation, bounded parameter queues and output-event observation. Native latency is summed; direct bridge latency is zero. `start(instances, rate)` still means buffered four-packet behavior. `start_buffered(instances, rate, delay_blocks)` explicitly chooses 1–7 packets and rejects unsupported delays before activation.

Direct controls are delivered at the next callback's frame zero. Existing sample-offset MIDI is preserved. MIDI panic replaces pending note events on the next compatible process call. GUI-requested parameter flush executes on that callback before processing. This mode does not add a sample-offset host parameter automation API; use the lower-level scoped API when needed. Read-only/out-of-range parameters and full queues return errors through the owner as before.

`direct::impulse_latency(reference, captured, max_lag)` is an offline peak-alignment helper for **isolated impulse** captures. It finds the strongest absolute peak in each mono array and checks a nonnegative lag within the supplied bound. It is not general music cross-correlation, does not open hardware, and includes any device/plug-in delay present in the supplied capture. Feed a known impulse and an actual loopback recording to use it as part of a measurement. Arrays must be nonempty, finite and at most 1,048,576 samples; silence, negative lag or excess lag is an error.

## Lifecycle and real-time ownership

`LiveRack` remains !Send. Libraries, instances, metadata inspection, GUI calls and destruction stay on their creation thread. Direct callback resources live in a shared cell protected by a single atomic **try**-acquisition: the callback never waits for the owner. Concurrent/reentrant acquisition failure silences that call. The owner sets the stop flag before waiting off the audio thread for an in-flight call to finish, then takes and drops resources before native deactivation/unload. Subsequent calls from a retained callback half see stopped state and produce silence without dereferencing native pointers.

Each direct callback brackets every node with CLAP `start_processing` and `stop_processing` on the audio thread. This permits legal owner revocation between calls without asking a destroyed device callback to run a final stop. DSP history is not reset, but plug-ins with expensive start/stop behavior may perform poorly. Measure those plug-ins; buffered mode keeps a continuous processing interval and remains available. Hosts whose device callback migrates between OS threads should evaluate plug-in-specific expectations. This is a first direct lifecycle contract, not universal commercial plug-in acceptance.

Prepared buffers, metadata, bounded command vectors and output queues are allocated off-thread. Soniclaw's successful direct fixture path allocates no callback heap and performs no locks/files. Foreign plug-in code may allocate, block or crash; choosing trusted code does not establish its real-time quality. The owner must not be dropped from inside its own callback. A hung callback also prevents safe unload; there is deliberately no unsafe force-unload API.

## Errors, persistence and migration

Invalid layout/rate/block/preparation returns an error. Invalid MIDI/buffer input is silent. Process errors or nonfinite native output silence the node, increment process_errors and request note panic; later serial effects may produce tails. Failure to start processing is counted and silenced. Stop is idempotent. Deadline counters count calls exceeding frames/sample-rate for the plug-in portion; shared-VM/debug-build timings are descriptive, not acceptance thresholds. A zero bridge count is not a zero physical latency claim.

No ProjectState or device-preference schema changes are introduced. Existing buffered API calls keep their behavior. New diagnostics fields are additive JSON fields; consumers using a rigid external JSON schema must accept them. Runtime Arcs, resources, active lifecycle and callback timing are never persisted. See [release notes](releases/v0.16.0.md) and [desktop roadmap](DESKTOP_ROADMAP.md). Physical audio, real DAWs and macOS/Windows runtime remain pending unless explicitly recorded.
