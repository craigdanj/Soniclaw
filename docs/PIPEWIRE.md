# Native PipeWire integration (0.51)

`soniclaw-pipewire` connects a core `AudioProcessor` directly to a Linux PipeWire graph through native DSP filter ports. It provides separate named input/output ports, prepared interleaved processing, explicit link ownership, registry snapshots and atomic timing telemetry. It uses PipeWire's C ABI through a small compiled shim, independently of the existing CPAL device backend.

**Validation status:** native headers/library compilation and callback tests passed. The executable private-server probe was attempted but the execution runtime rejected native protocol socket setup with `Operation not permitted`. End-to-end server registration/routing, real device audio and disconnect/reconnect acceptance remain pending. The shipped probe is executable acceptance work, not evidence that those checks passed.

## Build

On Debian/Ubuntu install `libpipewire-0.3-dev`, `libspa-0.2-dev`, `pkg-config` and a C compiler. The build requires PipeWire development API 0.3.65 or newer. This release compiled against 1.4.2. SPA headers use GNU C extensions; the shim enables GNU11 and warnings as errors. Non-Linux builds expose a stopped unsupported-backend error without linking PipeWire.

```sh
cargo check -p soniclaw --features pipewire
cargo test -p soniclaw-pipewire
cargo build -p soniclaw-pipewire-probe
```

Use `soniclaw::pipewire` with the optional facade feature or depend on `soniclaw-pipewire` directly. Other native/device features do not implicitly select it.

## Start and connect

```rust,ignore
let mut host = pipewire::Host::start(pipewire::Config {
    name: "my-audio-app".into(), remote: "pipewire-0".into(),
    sample_rate: 48_000, channels: 2, max_block_frames: 256,
    requested_quantum: 128,
}, Box::new(processor))?;
host.wait_ready(std::time::Duration::from_secs(2))?;
let graph = host.snapshot();
// Select output/input global port IDs from the current graph, then:
let handle = host.link(output_port_id, input_port_id)?;
// Observe subsequent snapshots and error state for asynchronous server acceptance.
host.unlink(handle)?;
```

Each channel creates `input_N` and `output_N`, numbered from one. Ports are planar float32 DSP ports at the native boundary; Rust copies to its prepared interleaved buffer, calls the processor and writes planar output. Missing input buffers are zero. Nonfinite input/output samples are sanitized to zero. The processor receives a monotonically increasing local sample clock, 120 BPM, playing true and no MIDI events. Application transport/MIDI injection is not exposed by this first backend.

`requested_quantum` is a request, not a guarantee or a measured latency. An actual quantum larger than the prepared block is split into sub-blocks without allocation. The backend accepts actual process cycles up to 65536 frames; configure the server within that bound. It does not resample: a driver rate different from the prepared rate produces silence and increments `rate_mismatches`. Stop/rebuild at the negotiated rate rather than altering DSP specifications from a callback. DSP latency is advertised as a native ProcessLatency parameter.

## Routing and live graph observations

`snapshot()` returns this node's global ID, native filter state, error text, current nodes/ports/links and registry-overflow count. States follow PipeWire: -1 error, 0 unconnected, 1 connecting, 2 paused, 3 streaming. The graph is asynchronous: a returned ID or successful create request is not confirmation of final server acceptance. `wait_ready` polls off callback for paused/streaming registration, with a caller deadline capped at ten seconds.

`link(output, input)` validates port presence and direction against the current registry snapshot, then asks `link-factory` to connect global port IDs. The returned handle is local to this Host, not the eventual global link ID. The server can still reject the request or remove a port afterward; inspect snapshots/error text. Links do not linger after their owning Host is dropped. `unlink` releases the selected owned link. Up to 64 owned links and 512 registry objects are retained; overflow is explicit. Registry global removal events remove corresponding entries, allowing application device/port change handling. Names are bounded to 255 bytes in snapshots; global numeric IDs are session-scoped and must be resolved again after restart.

No physical ports are auto-connected. Select hardware/capture/playback ports using snapshots or a separate graph manager. Policy/session managers remain responsible for physical device discovery and general graph policy; this backend is an application node and explicit link client. It does not promise driver ownership or transparently recreate missing hardware links.

## Telemetry and thread ownership

`telemetry()` clones an Arc on control; `snapshot()` on that telemetry reads atomics without calling PipeWire. Timing includes callbacks/frames, observed quantum/rate, driver clock nanoseconds, signed driver delay in clock frames, cycles with XRUN_RECOVER flag, rate mismatches, maximum Rust callback duration, callback durations exceeding the quantum period and lifetime input/output peaks. These are independent atomic reads, not a transactional snapshot. Driver delay is not measured converter round-trip latency. Xrun recovery cycles are not necessarily unique xrun events. Scheduling overruns are local measured processing duration, not a complete server deadline metric.

Host construction/preparation, registry reads, link creation/removal, waits and destruction run on the owning control thread. `Host` is intentionally not Send/Sync. The PipeWire data loop exclusively owns mutable processor state during playback. Callback working storage is allocated before connecting; it performs bounded planar/interleaved copies, core processing and atomic telemetry. It does not allocate, lock the control loop, join threads or access files. User processors must meet the usual callback contract and must not panic across the native callback boundary.

Dropping Host stops its main loop, destroys owned links/registry/filter/core/context and joins native data-loop ownership before dropping callback DSP storage. Keep external telemetry Arcs freely; they retain counters rather than native resources. On error or server loss, stop/drop the owner on control and explicitly construct a fresh Host/processor. Opaque project/native state belongs to the application's existing persistence layer. There is no automatic reconnect loop or hot transfer of DSP histories.

## Limits, migration and acceptance probe

Configuration: rate 8–192 kHz; 1–8 input/output channels; prepared block 1–1024; requested quantum 16–8192; node/remote names 1–128 UTF-8 bytes without NUL. Native process cycles above 65536 are outside the backend contract. This is an additive optional API and changes no existing project or device schema. Consumers enabling the feature now need PipeWire development dependencies on Linux. Do not serialize global object IDs as durable routing identities.

On a normal Linux machine with PipeWire runtime modules installed:

```sh
python3 tools/check-pipewire.py --report new-pipewire-report.json
```

This starts a temporary software-only daemon from `examples/pipewire-probe/headless.conf`, without changing the desktop server. The probe creates source/sink nodes, explicitly links both channels, checks input 1/output 0.5, removes a link and observes node removal, then stops its private server. A new JSON report and daemon log are written. It requires permission to create the private Unix socket. Alternatively run the probe against an explicitly selected test remote. Using the private unrestricted-access config outside its temporary private runtime directory is not intended.

The unit test directly invokes the same Rust ABI callback with a 191-frame planar cycle and 64-frame prepared storage, verifies stereo gain, sample-clock advancement, zero allocations, mismatched-rate silence and xrun-flag counting. See [executed validation and blocked native attempt](validation/v051/REPORT.md). Target-machine hardware, a working private-server run, sample-rate changes and disconnect/reconnect remain acceptance requirements.

API references: [PipeWire DSP filter example](https://docs.pipewire.org/audio-dsp-filter_8c-example.html), [Filter API](https://docs.pipewire.org/group__pw__filter.html), [Link factory properties](https://docs.pipewire.org/page_module_link_factory.html). The local headers used for compilation are the compatibility authority for this release.
