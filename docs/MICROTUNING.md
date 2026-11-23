# Microtuning (0.39)

## Workflow and example

`cargo run -p soniclaw-microtuning -- NEW_DIRECTORY` parses a just-intonation scale, compiles its keyboard mapping, saves the resulting tuning and renders a real sine-synth triad at 256, 320 and 384 Hz. `soniclaw::tuning` reexports the standalone `soniclaw-tuning` crate.

Create a `Tuning` from an explicit `[Option<f64>;128]` absolute-frequency table, choose `equal_temperament(reference_key,hz)`, or use `Scale::from_scl`, `KeyboardMapping::from_kbm` and `Tuning::from_scala`. `None` denotes an unmapped MIDI key. All parsing, validation, table construction and Arc ownership changes happen on the control thread.

```rust
use soniclaw::tuning::{Scale, KeyboardMapping, Tuning};
let scale = Scale::from_scl("Just triad\n3\n5/4\n3/2\n2/1").unwrap();
let tuning = Tuning::from_scala(&scale, &KeyboardMapping::linear(60, 256.).unwrap()).unwrap();
assert!((tuning.frequency(61).unwrap() - 320.).abs() < 1e-9);
```

## Instruments and projects

`ExpressiveSynth::set_tuning(Some(Arc<Tuning>))` and `MultiSampler::set_tuning` clear their voices and install the immutable table. Call while stopped, never from a callback. `None` restores the original equal-tempered behavior. `PolyModSynth` uses its public `synth` tuning API. Ordinary MIDI and note-expression pitch bend remain relative semitone offsets around the table's frequency. Unmapped note-ons do not allocate/steal voices; note-offs and panic still release normally.

Sampler playback uses absolute target Hz divided by the recorded sample root's standard A440 equal-tempered frequency, so A432 correctly retunes a root-A440 sample instead of cancelling the change. Source root metadata is still an equal-tempered note label; samples recorded at other calibrations need corrected root metadata/PCM or a product-specific source-frequency policy.

Project schema 2 gains optional `tuning`. Absent fields deserialize as None and are omitted on serialization, preserving old behavior. Reconstruction installs it on built-in PolyModSynth and preloaded MultiSampler nodes. Other oscillator models, disk-streamed samplers, custom registry processors and external native plug-ins must opt in explicitly; there is no automatic universal retuning layer. Old framework versions reject this new field due to strict schema validation. Remove it only when intentionally returning to standard tuning. Tuning changes require prepared project reconstruction, not live host parameter automation.

## Scala import subset and persistence

The parser follows the [Scala scale rules](https://www.huygens-fokker.org/scala/scl_format.html) and [keyboard mapping description](https://www.huygens-fokker.org/scala/help.htm#mappings) within documented bounds. SCL supports comments, empty descriptions, ratios (including integer ratios), decimal cents and trailing pitch labels. Degree zero is implicit; the last interval is the repeating period. Non-monotonic/negative intermediate degrees are retained. KBM supports gaps (`x`), negative degrees, mapping repetition, reference key/Hz, key-range limits and octave degree zero meaning the scale size. A size-zero map is linear. Negative-key extension uses Euclidean division so notes below the middle key repeat correctly.

This is a bounded UTF-8 subset: 1–128 scale entries (a zero-entry scale is rejected), period > 0, every interval magnitude <= 9600 cents; KBM 0–128 entries with degrees/octave magnitude <= 4096. Files are <= 64 KiB, names <= 256 bytes. Imported frequencies outside 0.01–1,000,000 Hz become unmapped, allowing short/non-octave scales to cover a useful keyboard range without infinite/extreme frequencies. The reference frequency must be in that range and mapped. There is no Scala command-language, MTS/MTS-ESP, MIDI tuning SysEx, channel-specific mapping or SCL/KBM exporter.

`Tuning::to_json/from_json` uses validated schema-1 tables, <= 32 KiB, preserving f64 roundtrips. Serde deserialization also validates, including when embedded in project state. The snapshot stores compiled absolute frequencies, not external scale paths; unavailable files cannot change a restored tuning. Keep the previous tuning on parse/validation errors. Extreme explicit table frequencies and entirely unmapped tables are rejected.

## Validation and limits

Tests cover A440/octaves, exact JSON roundtrips, malformed tables/ratios, just thirds/fifths, negative degrees, empty descriptions, KBM gaps/repetition and invalid reference mappings. Integration tests measure actual synth and sampler output at 432 Hz, verify zero callback allocations and unmapped-note silence, and check optional project persistence. The example exports audible PCM. This release adds deterministic tuning support; it makes no hardware tuning/DAW interoperability or universal instrument support claim.
