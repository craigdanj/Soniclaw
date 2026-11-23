# Typed parameter SDK

`soniclaw::params` / `soniclaw-params` defines float, integer, boolean, indexed
choice and real Rust enum controls. One immutable declaration supplies physical
DSP conversion, stable identity, host metadata/text, generic editor controls and
versioned normalized snapshots. It is opt-in; existing scalar parameters retain
their existing storage and mapping.

## Declare once, use everywhere

```rust
use soniclaw_params::*;
enum_choices! { pub enum Mode { LowPass=>"Low pass", HighPass=>"High pass" } }
parameter_set! { pub struct Controls {
    cutoff: FloatParam = FloatParam::new(Definition::new("filter.cutoff","Cutoff",
        Kind::Float {min:20.,max:20000.,default:1000.,curve:Curve::Logarithmic})
        .unit("Hz").group("Filter")),
    voices: IntParam = IntParam::new(Definition::new("synth.voices","Voices",
        Kind::Integer {min:1,max:64,default:8})),
    bypass: BoolParam = BoolParam::new(Definition::new("effect.bypass","Bypass",
        Kind::Boolean {default:false})),
    mode: EnumParam<Mode> = EnumParam::new("filter.mode","Mode",Mode::LowPass),
} }
let controls=Controls::new()?;
controls.cutoff.set(1200.)?;       // physical Hz
controls.voices.set(12)?;         // i32
controls.bypass.set(true)?;       // bool
controls.mode.set(Mode::HighPass)?;// actual Rust enum, no index at this call site
let cutoff_hz: f32=controls.cutoff.get();
let raw=controls.raw_parameters(); // pass these exact shared handles to PluginInstance
let declarations=controls.declarations(); // serde-compatible metadata export
# let _=(cutoff_hz,raw,declarations);
# Ok::<(),soniclaw_params::__core::Error>(())
```

`ChoiceParam` exposes a checked `usize` index for dynamic lists.
`EnumParam<T>` exposes a fieldless Rust enum implementing `EnumChoice`;
`enum_choices!` generates its ordered variants, labels and index conversion.
`parameter_set!` generates typed public fields, fallible construction,
`raw_parameters()` and declaration export, checking duplicate/colliding host IDs.
Macros resolve their core types through the SDK and do not require an additional
direct `soniclaw-core` dependency in the consumer.

Keep clones of typed handles in processors. Cloning shares the same atomic raw
value. `get/set` use the typed physical value; `normalized/set_normalized` use
finite0...1. `modulated` applies the raw normalized modulation offset then
converts/quantizes to a physical typed result. Smooth normalized or physical
values explicitly in DSP as appropriate; this SDK does not add implicit ramps.

## Storage, host ABI and UI contract

Float storage is normalized0...1. Linear/logarithmic declarations convert that
to physical DSP values and format/parse physical text. For example the geometric
midpoint of20–20000Hz is about632.456Hz at normalized.5, not10010Hz. Integers,
bools and choices use exact stepped raw values in their declared native range.
The host's normalized space maps that range to0...1. This keeps CLAP-wrapper
VST3/AUv2 normalization consistent with the declared float mapping.

Core `ParameterCodec` is an immutable presentation/quantization hook. Standard
SDK definitions implement it; `Parameter::with_codec` attaches it. Core numeric
setters validate the canonical result. `Parameter::detached()` retains metadata
in a separate control-thread edit copy, preserving the editor's existing queued
edit/acknowledgment behavior. Existing `Parameter::new` has no codec and retains
its scalar behavior.

CLAP exports automation/stepped/enum flags, group/module name, raw ranges,
defaults and shared value/text conversion. `host::ClapInstance::parameter_text`
and `parameter_from_text` expose bounded native conversion on the control side.
The generic UI slider uses shared formatting/parsing. `ui::parameter_control`
selects a bool checkbox, enum dropdown or numeric slider; integer sliders step
by1. The exported native editor retains declaration metadata in its detached
edit copies and uses discrete controls for stepped values. Custom apps may use
the existing parameter knob for continuous controls.

Stable numeric IDs remain the existing FNV-1a hash of the persistent string ID;
`core::stable_parameter_id` and `plugin::parameter_id` produce the same values.
Set construction rejects duplicate/numeric collisions and the reserved ID.
Display names/units/groups can change without changing identity; keep the ID
stable for a compatible control.

## Runnable processor and exported plug-in

```sh
cargo test -p soniclaw-params -p soniclaw-core -p soniclaw-clap --locked
cargo build -p soniclaw-typed-effect --features editor --locked
# Linux example; use the actual .dll/.dylib on Windows/macOS:
target/debug/soniclaw-typed-effect new-typed-report target/debug/libsoniclaw_typed_effect.so
# In a Linux X11 session, with SDK tools built:
target/debug/soniclaw-sdk-tools --editor-demo target/debug/libsoniclaw_typed_effect.so new-typed-editor-report
```

`examples/typed-effect` is an actual exported gain/filter CLAP effect with linear
gain, logarithmic cutoff, integer filter stages, boolean bypass and enum mode.
Its executable writes declarations/snapshot/report JSON, runs the same DSP
factory directly, and optionally loads the built native export to verify flags,
physical text parsing and sample-offset host automation. The example uses
immediate gain changes to make offsets testable; production processors should
apply their chosen click-free ramp policy. Report paths must be new directories.

## Presets and migration

`Snapshot::capture(product, declaration_revision, raw_handles)` stores schema1,
product ID, application-managed nonzero declaration revision and normalized
values keyed by persistent IDs. `from_json` limits input to1MiB and validates
shape/numeric bounds. `restore` checks the whole document before mutating any
parameter, rejecting wrong product/revision or unknown IDs. Missing keys keep
current values, allowing partial presets. Serialize with serde_json and use
existing transactional project storage off the callback. Restore while stopped
if the application needs a coherent multi-parameter transition; individual
atomic controls are not a transactional simultaneous snapshot.

Changing a range, curve or choice ordering/count requires a new declaration
revision and an explicit application migration. Normalized old values cannot
be blindly reused after such changes. Appending enum variants preserves raw
indices for existing choices, but changes normalized spacing; migrate SDK
snapshots/host automation accordingly. No schema guessing is performed.

Legacy scalar project/preset values remain physical raw scalars. Opting an
existing ID into normalized float storage is a representation change: an old
cutoff value1000 must be converted using `Definition::raw_from_physical`, not
loaded as normalized1000. Keep legacy storage for compatibility or provide a
versioned state/host-automation migration before adopting the new declaration.
The CLAP state envelope and project schemas do not automatically convert legacy
IDs. The new example uses its own product identity to avoid that ambiguity.

## Bounds, threads and errors

Definitions validate finite ranges/defaults; logarithmic floats require a
positive minimum. IDs are ASCII alphanumeric/`._-`,1–128bytes; names1–128,
units≤24, groups≤128. Choice lists contain2–256 unique, nonempty labels≤128bytes.
Integers stay within ±16777216 with span≤65535 to preserve exact stepped float
storage and normalized snapshot roundtrips. Parameter sets cap at4096 handles;
snapshots cap4096 entries and1MiB JSON. Invalid definitions/types/indices/physical
values and ID collisions return core `Error::InvalidParameter`.

Construct, clone lists, format/parse text, export declarations and serialize
snapshots on the control side. Prepared typed numeric reads/writes and standard
SDK quantization allocate no memory and take no locks. Custom `ParameterCodec`
implementations must honor that numeric contract; formatting may allocate.
Group/unit text is presentation, not a DSP unit-conversion parser: e.g. `1000 Hz`
works for a Hz declaration, but arbitrary SI suffix expressions are not inferred.

## Validation and limits

Tests cover geometric/log mapping, integer quantization and extreme bounded
integer roundtrips, bool/choice/real-enum typing, host labels, invalid ranges and
ID collisions, snapshot revision rejection before mutation, and4096 rounds of
allocation-free numeric control operations. Native exported-effect validation
checks real CLAP metadata/text plus gain at frame128 and bypass at192. Optional
native editor evidence is recorded separately. Broad DAW and Windows/macOS
runtime acceptance remains pending; no new JUCE-parity percentage is inferred.
