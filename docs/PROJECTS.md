# Project state, reconstruction and undo/redo

`soniclaw::state::ProjectState` schema 2 stores product/name, scalar parameters,
relative WAV asset references and root notes, sample-map key/velocity zones,
round-robin groups, one-shot/loop settings, rack nodes and settings, graph
connections and output gains. It describes a project; delay buffers, playing
voices and other transient DSP history are deliberately not saved. Audio files
remain external: ship them beside the document, keeping relative paths intact.

`ProjectState::from_json` accepts schema 2 and migrates schema-1 parameter
presets to a document with empty structural sections. Unknown schema versions,
unknown fields, duplicate IDs, invalid zones, cyclic graphs and unsafe paths
are rejected. Migrating a parameter preset does not invent its original graph
or sample bank. JSON input is capped at 4 MiB; the model additionally bounds
nodes, assets, zones, connections and parameters.

v0.6 adds optional global modulation configuration and `RackNode.asset` for
granular/convolution WAV nodes. See [migration](MIGRATION_060.md),
[module bounds](DSP_MODULATION.md) and the [native workbench](WORKBENCH.md).

## Build a runnable rack

```rust
use soniclaw::{project, plugin::PluginDescriptor, state::{ProjectState, RackNode, ProcessorKind}};
let mut state = ProjectState::empty("com.example.effect", "My effect");
state.nodes.push(RackNode {
    id: "amp".into(), kind: ProcessorKind::Gain,
    settings: [("gain".into(), 0.5)].into(), input_gain: 1.0,
});
state.outputs.insert("amp".into(), 1.0);
let plugin = project::create_plugin(PluginDescriptor {
    id: "com.example.effect", name: "My effect", vendor: "Example",
    version: "0.4.0", is_instrument: false, accepts_midi: false,
}, state, "/path/to/project")?;
let json = plugin.save_state()?;
```

The factory builds the v0.5 advanced DSP nodes described in [DSP expansion](DSP_ADVANCED.md), a basic-bank Wavetable source, and the earlier gain, low-pass, peaking EQ, compressor, gate, chorus, reverb,
limiter, drive, delay, preloaded sampler and disk sampler nodes. Global control
IDs are `node_id.control_key`; node IDs cannot contain dots. See
`EffectKind`/`AutomatedEffect::parameters()` for effect controls. `settings`
provides initial values and `parameters` overrides current values when loading.
Unknown or out-of-range settings/parameters fail reconstruction.

Sampler node settings support `attack_ms`, `release_ms`, `quality` (0 linear,
1 sinc-fast, 2 sinc-best) and optional `mpe_master`, `mpe_first`, `mpe_last`,
`mpe_member_bend`, `mpe_master_bend`. They are structural configuration in this
release. Preloaded sampler nodes share one immutable project bank, capped at
128 MiB of decoded PCM. Disk sampler nodes use 64 voices and a 16 MiB PCM-cache
budget per node, open at most 128 WAV assets, and retain their worker handle
inside the processor for control-thread teardown. See [streaming](STREAMING.md).

Asset resolution canonicalizes both root and file and rejects paths/symlinks
outside the root. Missing, malformed or oversized assets fail loading before
replacing the existing engine. Assets must remain unchanged during playback.
This is a framework document format rather than a graphical DAW arrangement
editor or a universal serializer for third-party processor types.

## Host state and compatibility

Ordinary `PluginInstance` factories retain schema-1 parameter state. Instances
created with `project::create_plugin` save schema 2 and attach a reconstruction
factory. `PluginInstance::load_state` requires deactivation. It validates and
constructs the replacement before publishing it. Rebuilds preserve the original
parameter Arcs/IDs; adding/removing host-visible parameters requires creating a
new instance. Standalone projects can rebuild an entirely new instance.

The CLAP adapter stages complete scalar snapshots at block boundaries, including
legacy schema-1 snapshots. Project scalar snapshots can load during processing
when their structure is unchanged. Structural changes require a deactivated
plug-in and the same parameter identities; the adapter returns false while
active rather than doing file I/O on the callback. Unsupported products do not
silently discard project structure. The supplied Project Rack demonstrates this
path; the older reference products retain their fixed factories/schema 1.

## Undo/redo

`ProjectHistory` keeps a caller-bounded number of labeled document snapshots.
Use `edit`, `undo`, `redo`, `undo_label` and `redo_label` on a control/UI thread.
Validation failure preserves both document and history. Undo capacity is a
snapshot count, not a byte-memory limit.

`project::ProjectSession` combines history with asset resolution/reconstruction.
Its `edit`, `undo` and `redo` build a new `PluginInstance` before committing
history, so missing files and invalid processor settings do not partially apply
an edit. Stop the device, take exclusive ownership, replace the processor, then
prepare/restart it. These APIs do not hot-swap active graphs. Capture current
host state with `save_state()` before starting a new document-editing session.
Runtime preparation and device negotiation can still fail and must be handled
by the application.

Run `cargo run -p soniclaw-ecosystem -- /new/output/directory` for a hardware-free
walkthrough, or build `soniclaw-project-plugin` for a fully automatable CLAP rack.
