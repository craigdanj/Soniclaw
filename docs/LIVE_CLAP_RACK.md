# Live CLAP rack and hosted floating editors (0.14)

`soniclaw::host::live` adds an actual continuously running serial plug-in rack. `LiveRack` owns instances on their creation/main thread; `RackProcessor` is the Send callback half. A dedicated audio worker processes foreign DSP. The device callback exchanges bounded packets and never waits for that worker. This introduces a fixed 1,024-frame bridge delay plus plug-in latency; at 48 kHz the bridge alone is approximately 21.33 ms. It is intended for an initial resilient hosting path, not low-latency guitar monitoring.

## Native workbench workflow

1. Build/run `cargo run -p soniclaw-workbench`. Open **Host**.
2. Enter a trusted CLAP file/directory and **Discover / scan**. Native code executes inside this process. Discovery and inspection stop live audio before initializing another library.
3. Select a product, **Add selected to rack**, and repeat up to eight nodes. Nodes process audio in insertion order. Select a rack row for its generic parameter controls. Clear the rack to change ordering; drag/reorder editing is not provided yet.
4. **Start live rack** negotiates the existing audio-device settings and creates instances on the GUI thread. Output-only mode supplies the workbench's quiet test sine to effects; the unified Audio mode supplies the configured input. Enable Monitor in Audio when using unified mode. Input MIDI is broadcast to nodes with note input ports. The keyboard can send notes to hosted instruments.
5. Move the generic sliders to queue validated updates for the next worker packet. The metadata list is captured on activation; restart/rescan requests stop the rack rather than changing metadata while audio is using it.
6. **Show floating editor** creates/shows the selected product's native floating editor when it advertises support for the platform window API. **Close editor** hides/destroys it. Embedded-only plug-ins return a clear unsupported error. Floating plug-ins may need platform/timer/fd services that this host does not provide; acceptance is per product.
7. Watch bridge counters. Any underrun represents 256 frames of scheduled silence; input/output/event drops and process errors are separately counted. **Stop rack** closes the device stream, joins DSP and destroys editors/instances on the GUI thread. A blocking foreign process call can delay this stop.
8. **Save rack list** writes `host-rack.json` in the workbench project root; **Load rack list** restores topology while stopped. This list is independent of the internal Soniclaw project and its preset catalog. Starting a loaded list is an explicit native-code load action. Inspect the paths before starting an imported list.

The regular Play control can restart the selected live rack after it has been started. Stop exits hosted mode. Applying an internal project edit while hosted replaces the bridge; the resulting stopped rack is detected on the next GUI poll. Start the hosted rack again explicitly. Live rack topology is limited to the main stereo path; sidechains and parallel routing are not implemented.

## Runnable checks and example

```sh
cargo build -p soniclaw-host-fixture -p soniclaw-sdk-tools
cargo run -p soniclaw-sdk-tools -- --live-rack-demo \
  target/debug/libsoniclaw_host_fixture.so /tmp/new-live-rack-demo
cargo run -p soniclaw-sdk-tools -- --host-contract \
  target/debug/libsoniclaw_host_fixture.so /tmp/new-host-contract.json
cargo test -p soniclaw-host
```

On macOS/Windows substitute the platform library filename. These are build/run instructions, not claims that those runtimes were tested. The demo intentionally selects `org.soniclaw.host-fixture`, instantiates it twice, verifies gain 0.5 × 0.5 = 0.25 after exactly 1,024 silent frames, observes plug-in output events and main callbacks, exercises repeated floating-editor creation/show/close and rejects invalid parameter IDs/values. Its clock is synthetic and waits **between** callback calls for repeatable ABI checks. The fixture GUI is an ABI mock with no actual OS window. The demo is not a deadline benchmark or a third-party GUI acceptance test.

```rust,ignore
use soniclaw::host::live::{instantiate, LiveRack, RackEntry};
let entries = vec![RackEntry {
    path: "/trusted/MyEffect.clap".into(),
    plugin_id: "vendor.product".into(),
}];
// SAFETY: caller has chosen trusted ABI-conforming native code.
let instances = unsafe { instantiate(&entries)? };
let (mut rack, processor) = LiveRack::start(instances, 48_000.0)?;
// Move processor into your stereo device engine; keep rack on this GUI thread.
// In your event loop:
rack.poll();
for event in rack.drain_events() { println!("{event:?}"); }
// Parameter identifiers and physical ranges are obtained from rack.parameters(node).
rack.set_parameter(0, /* known ID */ 1, /* in range */ 0.5)?;
rack.show_editor(0)?;
// Before dropping the native libraries:
rack.stop();
```

`instantiate` canonicalizes paths and shares one `ClapLibrary` initialization for repeated paths within a chain. `LiveRack::start` accepts 1–8 stopped instances, activates stereo f32 processing at a fixed 256-frame worker maximum, and returns the main owner plus callback processor. Already-active, wrong-layout or activation failures are errors. The existing host supports a single mono/stereo main input and a stereo output for this rack, supported note dialects, and its bounded CLAP event subset. General multi-port/audio64 hosting is outside this release.

`parameters(node)` exposes cached read-only metadata. `set_parameter` rejects unknown/read-only IDs, nonfinite/out-of-range values and a full 64-command queue; it schedules values at offset zero of the next processed packet. Values are physical units, not normalized. No sample-specific parameter scheduler is added to this convenience queue; the lower-level `HostedAudio::process_block` retains its existing timed-event API.

`poll` services `on_main_thread`, GUI show/hide/closed notifications and requested parameter flushes. Flush executes on the audio worker before the next process packet. Restart, latency-change and parameter-rescan requests stop the worker and require reconstruction. Dirty/process requests remain observable in the returned per-node flag bits. `drain_events` removes at most 128 observed MIDI/parameter/gesture events on the control thread; these events include node index and packet-local frame. They are observations, not MIDI routing into the next node. UI parameter echoes update the selected generic controls. Processed input MIDI is broadcast to compatible nodes independently.

## Callback protocol, latency and failures

The bridge assembles 256-frame interleaved stereo packets across arbitrary device callback sizes. Eight packets fit in each input/output queue. Every packet carries its sequence and transport at its first frame. Output is scheduled exactly four packets later. Early/future packets are retained for their deadline, stale ones discarded, and an absent packet yields silence for that whole deadline block. The initial four blocks are deliberately silent and do not count as underruns. An input queue overflow drops the packet; an output queue overflow drops the result. MIDI is limited to 128 events per packet; overflow/input loss requests an AllNotesOff recovery event before the next processed packet. That panic replaces the packet's MIDI rather than replaying potentially stale note-ons. The existing host expands panic into per-channel CC123 for MIDI1-only products.

A process failure/nonfinite output silences that node's result, increments process_errors and schedules panic. Subsequent serial nodes may still generate a tail from silence. This does not unload or recover a crashed/hung native library. Unsupported MIDI forms can cause process errors through the lower-level dialect checks. The worker sleeps briefly when its input queue is empty; its scheduling and plug-in cost must fit the bridge's lookahead on a target machine. Counters are the evidence; no zero-underrun guarantee is implied by queue bounds.

`RackProcessor::prepare` requires the configured sample rate and stereo channels before processing starts. Re-preparing after processing starts is rejected; reconstruct for a new device clock. `reset` requests panic and leaves the monotonic bridge sequence intact. Dropping either half requests stop; stopped callback output is silent. `latency_frames` reports 1,024 plus the sum of activation-time plug-in latencies. There is no parallel latency compensation, asynchronous sample-rate conversion or hot topology edit.

## Ownership and editor services

The owner remains !Send because its CLAP instances/library Rcs stay on the creation thread. The worker receives exclusive resources and raw leases to stable instance/context allocations. It borrows immutable cached metadata and uses atomic host flags. The owner joins the worker **before** any deactivation, native destruction or library unload. It exposes only legal main-thread services during processing; state load/save and activation are unavailable through a running LiveRack. An audio-worker panic is caught by join and counted. User code must keep polling on the same GUI thread and run that platform's normal GUI event loop. Dropping LiveRack must also occur there.

Floating GUI support selects X11, Win32 or Cocoa for the platform. The host advertises `clap.gui`, queues request_show/hide/closed flags and refuses embedded resize requests. A was_destroyed closure notification relinquishes the editor handle without double destruction. Other closures are destroyed by the owner. Create/show failures destroy any created GUI and return an error; missing extension/callbacks/support also fail explicitly. No embedded parent handles, host timers, POSIX fd callbacks, plug-in discovery sandbox or additional platform message pump are supplied. Some plug-ins therefore cannot run their editor here. macOS callers must use the actual OS main GUI thread, not an arbitrary worker. Workbench does so; it has not been runtime validated on macOS/Windows.

## Persistence and migration

`RackEntry` is strict serde JSON with `path` and `plugin_id`. The workbench reads at most 64 KiB and at most eight entries, then loads code only when Start is pressed. Relative paths resolve against the application's working directory; Save uses the discovered absolute paths. This is a local machine rack list, not a portable dependency bundle. Parameter values, opaque plug-in states, GUI geometry, queues and DSP history are not saved in that list. Existing stopped `ClapInstance::save_state/load_state` remain the API for applications that implement their own session format; do not call them concurrently with a live lease. No internal ProjectState schema change is needed for 0.14. Older Soniclaw projects continue to load normally.

## Validation and next extensions

See [verification](validation/v014/REPORT.md) and [release notes](releases/v0.14.0.md). Tests cover variable callback sizes, exact delay, late/missing packet handling, bounded overflow, callback allocations, stereo/rate validation and stopped output. The independent fixture validates native ABI thread/lifecycle, flush and callback delivery. Hardware listening, physical latency, real DAW sessions, third-party graphical editors, Linux real-time scheduling and Windows/macOS acceptance remain target-machine work. Source support for a platform is not a runtime pass. Lower-latency direct scoped integration, opaque rack-session state, embedded editors, timer/fd services and multi-bus routing are future work.
