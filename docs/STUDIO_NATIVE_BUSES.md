# Native sidechains and output returns — 0.87.0

Studio now connects explicitly negotiated VST3 input/output buses to the track engine. A pre-fader, post-insert source can drive an extra native input, and native output channels can feed separate mono/stereo return tracks with their own mixer controls, inserts, routing, cues and routed export. The typed routing/exchange API is reusable independently of the desktop.

## Studio workflow

Select a trusted scanned VST3 product and an existing native track instance in the plug-in manager. Under Native sidechains and output returns, inspect its ports, then enable that layout on the matching instance. Choose an input bus other than Main, choose a source track, and connect. Muting that source's mixer strip does not disable its pre-fader detector feed.

Choose an output bus and add an output-return track. A mono return duplicates its selected channel into the stereo track; a stereo return maps channels 0/1. The public API can select any one or two valid channels. Returns have independent faders, mute/solo, inserts and normal bus destinations. If the native source's main output isn't wanted, mute that source strip. Selecting its main bus as a return deliberately duplicates that signal; manage mixer routes accordingly.

Edits are stopped-time and undoable. Native layout changes require inspection and reconstruction. Disconnect linked inputs/returns before deleting or freezing their source. Return tracks must remain empty of clips, direct inputs and MIDI; they receive the named native output through their instrument slot. Track order is independent of processing order: native and stock sidechain/return dependencies are combined into a validated DAG.

## Framework API

`studio::native_buses::{Request,Return,Registry,Frames}` implements saved requests and prepared exchanges. `Request { layout, sidechains }` validates a `soniclaw_bus::flex::Layout`, maps extra input bus indexes to source track IDs and stores it in descriptor `state.buses`. `Return::descriptor` creates a stable `native.return` instrument with native processor ID, output bus index and one/two channel indexes.

`Factory::prepare_document` and `track_tap` are additive default hooks. NativeFactory uses them to prepare a Registry for the complete compile. It accepts `bus_graph: &mut Registry` in its struct literal. Reuse that registry through all descriptor builds in one preparation; do not reconstruct a fresh registry per descriptor. Other factories retain default behavior. Source tracks publish into prepared sample-clock exchanges before common track alignment; a topological preparation pass ensures native sources exist before their return processors. The Renderer retains ordinary document order for mixer/clip identities.

`soniclaw_host::studio::bus_adapter::Adapter` maps a negotiated VST3 multibus driver to stereo main processing, selected detector feeds and all output publication banks. `BusInstance::into_owner` retains the original control owner in Studio. Live processing uses the existing bounded native multibus worker, extended with up to 1024 normalized parameter points per 256-frame packet. Selected controls use the same stable native automation IDs as stereo hosting. The adapter's declared latency includes 1024 bridge frames plus native latency.

Offline mode uses `soniclaw_vst3::bus_offline::Processor`, starts native processing once and uses musical timeline spans, explicit note identities and supported expression on the original owner thread. It has only native latency and a weak owner reference/revocation flag. Shared stereo/native state restoration and zero-frame flush select multi-bus buffers when needed. No native object is moved to the audio callback for control operations.

## Bounds and behavior

Up to eight ports each direction and 64 aggregate channels each direction, following the framework Layout contract. Main input/output must be mono or stereo; connected detector buses are mono/stereo. Return tracks select one or two valid channels. Registry publication storage is bounded at 16 Mi floats per preparation; block size 1..8192 is inherited, while live VST3 worker preparation reserves at least 256 frames. Frames uses atomic sample-clock tags and sequential write/read ownership; stale tags read as silence. It is not a coherent concurrent snapshot interface.

Native sidechain sources and the target's insert prefix currently require zero processing latency, avoiding an unaligned detector. Native processor latency itself is compensated normally. Studio's stock sidechain latency expansion is a later release. Source/return graph cycles, unknown tracks/outputs, malformed layouts, mismatched native layouts and incompatible clip/input data reject preparation without adopting a candidate.

Live native bus processing retains the existing multibus bridge's packet-start transport and MIDI1 contract (128 MIDI events per 256-frame packet). Unsupported expression is surfaced as native processing failure; offline mode supports the musical identity/expression path. Dense parameter changes can exceed the 1024-point packet bound and report an error. Live resets request native MIDI panic; they do not promise to clear arbitrary plug-in effect history. Missing/bypassed native sources produce silent extra outputs while retaining cached latency. Broad native bus reconfiguration needs stopped reconstruction.

This Studio adapter currently targets VST3. CLAP multi-port processing/isolation already exists in lower-level framework APIs and can be adapted through the same generic routing/exchange model; this release does not claim a CLAP Studio multibus owner. Native GUI and state capabilities still depend on the product.

## Persistence, migration and validation

No outer project/document schema change: the typed request and return live in existing descriptor state. Native blobs retain format/product identity. Bypass retains known output latency; native return metadata never loads a module by itself. Old strict applications that cannot build `native.return` report an unavailable processor. Audit retains opaque/native dependencies conservatively. Changing an instance's port layout can invalidate existing return selections and must be validated as a project edit.

The `soniclaw-studio-native-buses` example enables an owned-fixture-only port layout, connects a muted key source, checks keyed main peak 0.0625 and independent output 0.25, verifies return mixing, zero callback allocations, 1024 live/zero offline bridge latency, state/project reopen and cyclic routing rejection. Run with a new folder and the rebuilt owned protocol fixture. It uses virtual audio and does not establish physical-device, arbitrary-product or Windows/macOS acceptance. [Executed checks](validation/v0_87_0/REPORT.md).
