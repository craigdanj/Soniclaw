# CLAP adapter (v0.3 update)

The adapter now supports a real optional stereo sidechain input and a feature-gated embedded editor. See [FORMATS.md](FORMATS.md) for the current editor/port/build contracts. Existing gain/synth examples remain headless by default; the four-product reference enables the editor. Unsafe FFI is confined to the CLAP adapter and the native-handle boundary in soniclaw-editor; portable DSP/core crates keep the safe-code policy.

The following v0.2 design notes describe the original lifecycle/state/event boundary; statements about missing editors or sidechain ports are superseded by the update above.

# CLAP export adapter

`soniclaw-plugin` owns the format-independent processing model. `soniclaw-clap` translates the CLAP ABI and exports a factory through `export_clap!`. `examples/clap-demo` provides two working definitions and `tools/cargo-soniclaw` can generate a one-effect project.

## Supported contract

- A static definition table and descriptors, one boxed instance per factory create.
- Stereo f32 main output; an effect has stereo main input and an instrument has MIDI 1 input.
- Activate/deactivate, processing start/stop, reset and variable blocks up to activation maximum (65,536 frames maximum accepted).
- Host parameter metadata/value/text/flush callbacks; stable hashed parameter IDs; sample-timed parameter events during render.
- Parameter-state JSON through CLAP streams, including partial reads/writes, invalid-state rejection and a 1 MiB load limit.
- Process-start tempo/position/playing context, latency, tail and real-time/offline render extensions.
- A fixed 1024 incoming events per process block limit. Unknown event spaces and unsupported events are ignored. Oversized or invalid event blocks fail explicitly.

The adapter accepts MIDI 1 and native CLAP notes. v0.4 preserves note IDs and routes tuning, pressure, brightness, pan and volume expressions with sample offsets. Expressive instruments support explicit MIDI-1 MPE zones. Per-note parameter modulation and emitted note-end tracking remain unsupported. Mid-block transport/tempo changes are not scheduled. Main ports remain fixed stereo f32; the reference ducker exposes a separate stereo key and reference/project products can enable embedded editors. Dynamic layouts/float64 remain unsupported. See [expression](EXPRESSION.md).

## State and thread ownership

Descriptors and definitions are immutable for the life of the library. Each instance owns a boxed runtime behind `UnsafeCell`. The host must serialize the CLAP lifecycle and audio callbacks as specified by CLAP. Parameter callbacks read separate shared atomics and never borrow the runtime. Activation allocates buffers and event vectors; render reuses them.

State load validates a complete snapshot, writes a sequence-stamped atomic staging bank and publishes an epoch. Render accepts a stable bank at the next block boundary and applies the complete group before processing; it never spins waiting for the control thread. When processing is stopped the group applies immediately. Parameter queries can report staged values before the next audio block. Scalar capture remains a snapshot of independent parameters. Opt-in schema-2 project state also records assets, sample maps, settings and routing. Structural restore requires deactivation and unchanged parameter identities; see [projects](PROJECTS.md).

The FFI adapter contains necessary unsafe pointer conversion and one immutable static-feature-list `Sync` assertion. Unsafe operations document the CLAP pointer/lifetime/thread invariants. Processor construction, activation and render catch Rust panics at the ABI boundary; production processors still must not panic or allocate in their callbacks. The safe DSP/core workspace retains its `unsafe_code = forbid` policy. Export examples and the independent host are narrow ABI-boundary exceptions.

## Validation and packaging

Build the demo, run `cargo run -p soniclaw-clap-smoke`, and use `tools/bundle-clap.py` to make a `.clap` file/bundle. The smoke host loads the actual shared library through `libloading`, constructs both instances and checks audio, event timing, automation, partial-stream state restore, lifecycle, text conversion and extension behavior. It is not a real DAW compatibility test.

The upstream [CLAP validator](https://github.com/free-audio/clap-validator) source snapshot reporting 0.4.1 was built and run on Linux for this release. The complete report is included at `docs/validation/clap-validator-linux.json`: 81 cases, 63 passed, zero failed/warnings, 18 skipped for unsupported capabilities. Those results do not certify another platform or a particular DAW. Native interactive host testing remains required.

The bundle helper copies an already compiled library. macOS gets `Contents/MacOS` and `Info.plist`; Linux/Windows get a `.clap` file. It refuses existing destinations. `--platform` changes package layout only; it does not change the library's architecture or operating system. Signing and notarization are outside this helper.

Primary API reference: [CLAP headers](https://github.com/free-audio/clap/tree/main/include/clap) and [clap-sys 0.5.0](https://docs.rs/clap-sys/0.5.0/clap_sys/).
