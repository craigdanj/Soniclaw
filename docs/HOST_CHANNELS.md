# Multichannel hosting and delay compensation

`bus::multigraph` adds a prepared graph whose edges identify individual channels
of named buses. `vst3::buses::BusInstance` exposes every advertised native audio
bus through the same callback contract. Sidechain key signals and auxiliary
outputs retain their own buffers; only explicit routes are mixed.

## User workflow

1. Inspect/select a trusted VST3 class on the main thread. Load `BusInstance` and
   inspect `layout()`. Loading enables all advertised audio buses while stopped.
2. Assign application role metadata with `set_input_role(index, Role::Sidechain)`
   where appropriate. Bus IDs are stable index-derived names (`input-0`,
   `output-0`); these are not claims to reproduce vendor bus names.
3. Save an explicit `multigraph::Blueprint`: schema1, external `Layout`, node
   IDs/layouts and `Connection`s. `Endpoint { node:None, bus, channel }` means an
   external input when used as a source, or an external output as a destination.
4. Start the native bridge, then `Blueprint::build(rate,max_frames,factory)` while
   stopped. The factory returns a prepared `Box<dyn MultiProcessor>` per node.
   Move the graph to the device thread; retain/poll native owners on main.
5. Feed one **interleaved slice per bus**, with exact advertised channel counts.
   Poll diagnostics, stop native owners and rebuild for structural changes.

Run the working native sidechain example:

```sh
python3 tools/build-formats.py --fetch --no-editor
cargo run -p soniclaw-multibus-host -- build/formats/Release/SoniclawReference.vst3 new-multibus-report
```

It writes the complete JSON graph plus a measured report. The graph feeds main
and key buses separately, mixes a compensated dry path with the hosted ducker,
and checks baseline, key-on ducking and key-off recovery. The destination must
be a new directory. No sound device is required.

## Public API and compensation

`MultiSpec {sample_rate,max_block_frames,layout}` and `MultiProcessor` define
prepare/process/reset/latency. `MultiGraph` implements that trait, so graphs can
be nested. Custom nodes prepare all buffers before returning. Their
`latency_frames()` must stay fixed until a stopped-time rebuild. `process`
returns a typed core `Error` and clears external output on invalid shape or a
node error. Earlier nodes may have advanced when a later node fails; reset or
reconstruct before retrying if consistent history is required.

The builder topologically sorts nodes, calculates the longest input arrival at
each join, and delays shorter **channel routes**, including sidechain routes.
It also aligns all graph outputs to the longest connected output path. A dry
external-input/output route therefore gets the same delay as a parallel native
plug-in path. `latency_frames()` includes the native latency plus the fixed1024
VST3 bridge. Cycles are rejected; feedback needs a separately designed processor
rather than an implicit graph cycle. No arbitrary speaker-layout downmixing,
surround panning law or silent channel truncation is performed.

```rust
use soniclaw_bus::multigraph::{Blueprint, Connection, Endpoint};
// Serialize Blueprint using serde; resolve processors by your stable application IDs.
// A route may connect any validated source channel to any destination channel.
let route = Connection {
    from: Endpoint {node: None,bus:1,channel:0},
    to: Endpoint {node: Some("ducker".into()),bus:1,channel:0},
    gain: 1.0,
};
# let _ = route;
```

## Lifecycle, persistence and migration

Blueprint schema1 stores routing, IDs, gains and exact layouts. Processor/native
state is application-owned; use the existing VST3 state API while stopped.
Store the plug-in class/path alongside the graph in your application document;
blueprint loading alone never executes a saved native path. Revalidate before
resolving factories. Save/write JSON off the callback with your existing
transactional project persistence API. Previous stereo `hosted::Session` files
keep their existing schema and behavior; the multibus blueprint is a separate
API and is not automatically converted to workbench's stereo session panels.

The native owner keeps main-thread lifecycle/editor services. The VST3 worker
performs locks/native calls and buffer conversion. Callback packets and both
SPSC queues are fixed-size. Owner stop/drop joins the worker before unloading;
a retained callback clears output and returns `NotPrepared`. Reset revokes a
native bridge; create a new run to continue. Polling structural native restart
requests revokes the old run. Changing role metadata requires a stopped owner;
changing native channel arrangements requires reconstructing the owner.

Native multibus events currently cover MIDI1 core messages. Use v0.27's stereo
musical bridge for its richer clock/note-expression/SysEx contract. Multiple-bus
CLAP loading is not added by this release. Applications connect the bus callback
to their chosen device mapping explicitly; automatic workbench surround-device
configuration is not provided.

## Bounds and failure recovery

At most32 nodes,512 channel routes,8 buses and64 aggregate channels in each
direction; block maximum8192 frames. Graph latency and individual processor
latency are limited to262144 frames. Node buffers are capped at64Mi samples
(256MiB); compensation lines at32Mi samples (128MiB). All resource checks run
before buffer allocation/factory work where possible. Gains must be finite and
within ±16; duplicate route endpoints, unknown IDs/channels and cycles fail.
Event lists are capped at1024/callback. Native packets accept128 MIDI events,
queues8 packets each. Queue saturation revokes the run; late output is silent
and counted. Clear errors on the control side by stopping and reconstructing.
These limits bound work and memory; they do not guarantee that arbitrary native
code finishes before a device deadline.

## Validation

`cargo test -p soniclaw-bus -p soniclaw-vst3` checks dry/processed impulse
alignment across uneven callback sizes, zero graph callback allocations,
independent auxiliary/key routes, malformed buffer silence and cycle/channel
rejection. The native example adds real VST3 sidechain activation and measured
compensated dry/wet routing with callback allocation counts. Exact run results
are in the release validation report. Physical interfaces, external DAWs and
Windows/macOS runtime acceptance remain separate pending checks.
