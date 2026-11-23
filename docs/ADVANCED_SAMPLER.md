# Advanced sampler and SFZ subset — 0.11.0

## Features and scope

The memory sampler now supports velocity crossfades and release-triggered samples. Control-thread tools import a documented SFZ subset and detect energy onsets to build slice zones. Existing sample-map editing, round robin, expression, sample start/end, loop fades and background project loading remain available.

This is a bounded sampler extension, not complete SFZ/player compatibility. Unsupported syntax/opcodes fail explicitly so instruments are not silently imported with missing sound-shaping behavior. The paged DiskSampler currently rejects these new zone features instead of ignoring them.

## Workbench workflow

Open Sample map. Each selected zone exposes Release trigger, Velocity fade in/out endpoints and Equal-power velocity fades. A release zone must be one-shot without looping. Apply sample map validates and prepares a new memory bank before publication; invalid drafts do not replace the active project. Banks still have the existing PCM budget.

For two velocity layers, overlap their velocity ranges, assign **different RR groups**, set a fade-out interval on the soft zone and a matching fade-in interval on the hard zone. Same-group zones remain alternatives for round robin; they do not layer. For example, soft fade-out 32–96 and hard fade-in 32–96 gradually transition between the two samples. Linear gain is useful for aligned material; equal-power fades use square-root weights for different material. Separate zone gain and the existing note-velocity gain still apply.

Release trigger plays on note release using the remembered **note-on** velocity. Sustain postpones the release sample until pedal-up. A matching note-on is remembered even when an attack sample has already ended. A second note-off does not duplicate the release. Note choke and global panic clear held records without spawning release samples. Release samples play one-shot and use fresh per-voice expression plus channel controls; historical per-note expression is not copied from an ended attack voice.

To import an SFZ instrument, enter its path in Sample map and choose Import SFZ subset. Parsing, WAV validation and resource construction run on the loader worker. Playback stops; the document changes only after a successful load. Unsupported/malformed data leaves the old document intact and reports an error. The project root becomes the SFZ directory, preserving scoped relative asset references.

For automatic slices, select a sample and choose Detect transients. Full source PCM is decoded/analyzed on a worker; results do not immediately alter the draft. Replace selected asset zones with detected slices creates regions on keys 36 upward. It removes that asset's previous zones only, leaving other assets intact. Review the regions, then Apply sample map. Equal slicing remains available separately.

## Runtime API

`soniclaw::sampler::Zone` and `soniclaw::state::SampleZone` add:

| Field | Meaning/default |
|---|---|
| release_trigger | false: attack; true: one-shot release sample |
| velocity_fade_in | optional [low, high] velocity endpoints; default none |
| velocity_fade_out | optional [low, high] endpoints; default none |
| velocity_power | false for linear gain, true for square-root weights |

Fade endpoints are inclusive boundary definitions in 0–127, with low strictly below high. Fade-in gain is zero at/below low and one at/above high; fade-out reverses that behavior. Zero-weight zones are not started. Different groups layer. Attack and release groups have independent round-robin counters.

Prepared processing uses fixed voice, round-robin and held-note storage; no file access or heap allocation is added to callbacks. At most 128 held note records are remembered. MIDI-1 repeated same-channel/key note-ons replace that held record; identified note-ons retain separate note IDs. When full, the oldest held record is replaced. Voice polyphony/stealing still follows the sampler's fixed N voice slots. This is not unlimited per-key release history.

## Import API and supported SFZ syntax

```rust,ignore
let (project, asset_root) = soniclaw::sampling::import_sfz(
    std::path::Path::new("/instruments/piano/instrument.sfz"), "my.product")?;
let plugin = soniclaw::project::create_plugin(descriptor, project, asset_root)?;
```

`LoadRequest::Sfz { path, product }` offers the same operation through AsyncProjectLoader. Custom product factories must validate the expected product ID as usual.

Supported headers: global, group, region, with inherited defaults and region overrides. Supported opcodes:

- sample; key, lokey, hikey, pitch_keycenter; lovel, hivel; volume.
- offset, end; loop_mode, loop_start, loop_end.
- trigger; xfin_lovel, xfin_hivel, xfout_lovel, xfout_hivel, xf_velcurve.

Supported trigger values: attack and release. Supported loop modes: no_loop, one_shot, loop_sustain. Release forces one-shot/no-loop. Other modes/triggers are errors. Velocity curves accept gain and power; imported defaults use power. Envelope/filter/controller/keyswitch/sequence/randomization opcodes, control/master headers, generators, embedded samples and preprocessor/includes are unsupported.

Numeric MIDI notes and note names such as C4 (60), F#3 and Bb2 are accepted. Sample paths may be quoted or contain spaces; backslashes are normalized to forward slashes. Relative paths must stay within the SFZ directory, including symlink resolution. Absolute paths, traversal and URLs are rejected by project asset validation. Line comments beginning // are recognized outside quotes. Quoted values do not support escaped quote characters.

SFZ end and loop_end are inclusive and become Soniclaw's exclusive frame endpoints. offset becomes start_frame. Region and loop ranges must fit the decoded WAV. volume is interpreted as decibels and converted to the existing bounded zone gain. Every region receives its own layer/RR group; SFZ sequence/randomization/choke-group semantics are not imported. Per-region references may duplicate decoded PCM; the total memory budget still applies.

## Transient API

`detect_transients(&AudioFile, TransientOptions)` returns region starts including zero. It compares one-millisecond mean-absolute energy windows with a decaying baseline, using an absolute threshold, rise ratio, minimum gap and pre-roll. This is a basic onset heuristic, not beat tracking or a machine-learning detector. Quiet, sustained or dense material can need different thresholds or manual edits.

Defaults: threshold 0.02, ratio 3, minimum gap 40 ms, pre-roll 1 ms, at most 64 slices. Valid ratios: 1.1–100; gaps: 5–5,000 ms; pre-roll: 0–100 ms; at most 128 markers. The workbench caps detected slices to 92 so keys 36–127 fit. `sliced_zones(asset_id, markers, frames, first_key)` validates ordered unique markers and MIDI key limits, returning one-shot zones with exclusive ends. No file I/O occurs inside these pure analysis/zone-building functions; they allocate and must run off the callback.

## Bounds, errors and recovery

SFZ source: at most 1 MiB, 16,384 tokens, 255 regions and 4,096 bytes per value. The project retains its 1,024-asset/zone and 4 MiB JSON limits, while imports enforce the existing 128 MiB decoded PCM budget. Waves with malformed/non-finite audio or out-of-bounds regions fail before a project is published. Imported release zones cannot loop. Unsupported opcodes identify their names in errors.

The importer reads files without modifying the source SFZ or samples. A failed import requires correcting the instrument or selecting a supported file, then retrying. Transient detection is single-flight in the workbench; applying results requires the selected asset identity to match. Avoid changing external sample files during detection. Source-file edits are outside project undo; zone edits become undoable when applied.

## Persistence and migration

Project schema remains 2. New fields default to false/none and are omitted when unused, preserving old built-in-only project serialization. Existing projects and product/parameter IDs remain valid. Older readers reject saved projects that contain the new fields. Rust Zone/SampleZone struct literals must supply the four new fields; prefer Zone::single and SampleZone::new for defaults.

New zone features currently require the memory sampler. DiskSampler returns a clear reconstruction error for release/fade zones. Portable bundles, catalog entries and recovery checkpoints retain the new serialized fields; they still store external assets only through the established project asset list.

## Runnable evidence

`cargo test -p soniclaw --test v011` covers endpoints, sustain/release, note IDs/choke, import inheritance/ranges, explicit unsupported/unsafe rejection and generated onset positions. `cargo run -p soniclaw-sdk-tools -- --sampler-demo /new/demo` writes generated soft/hard/release WAVs, a supported instrument.sfz, rendered instrument.wav, a project and onset report. No external sample license/download is required.

See the [fresh validation report](validation/v011/REPORT.md). These tests establish the implemented subset; they are not full SFZ conformance or a commercial sample-library acceptance claim.

Primary format references: [headers](https://sfzformat.com/headers/), [release triggers](https://sfzformat.com/opcodes/trigger/), [velocity fades](https://sfzformat.com/opcodes/xfin_lovel/). The support list and differences above define Soniclaw's actual contract.
