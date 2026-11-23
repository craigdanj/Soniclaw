# Sample-instrument authoring (0.38)

## Workflow and runnable example

Run `cargo run -p soniclaw-sample-author -- NEW_DIRECTORY`. It generates three original test samples, maps a two-alternative round robin and a second root, normalizes copies, exports a native project/SFZ, imports that SFZ, creates a real sample-instrument plug-in and renders a preview WAV. It writes a JSON receipt with detected pitch and preview energy. No hardware or third-party samples are needed.

Use `soniclaw::authoring::Source::new(path)` for each WAV. Optional root, velocity, round-robin ordinal, trim `[start,end)` and sustain-loop `[start,end)` overrides are explicit. `export(&sources,new_directory,product_id,target_peak)` validates/decodes everything before opening output. Sources are read-only; the destination must not exist. On a later write failure it attempts to remove only the newly created export directory. Do not let other processes write into that directory during export.

## Mapping and audio editing API

Filename tokens use underscore separators: `Piano_n60_v1-80_rr2.wav`. Root and velocity/RR overrides take precedence over inferred values. Roots must be 0–127, velocities 1–127, alternatives contiguous/unique from rr1. Roots spread across nearest-key midpoint zones, covering keys 0–127; disjoint velocity layers remain separate. Overlapping velocity layers at one root fail. Native zones sharing a root/layer receive one round-robin group; different root/layer groups remain independent.

When a root is absent, `estimate_root(AudioFile)` uses mono-average autocorrelation over at most 250 ms, decimated to approximately 16 kHz, and returns Hz, nearest equal-tempered MIDI note and normalized correlation confidence. It rejects silence and confidence below 0.85. Its search is 30–2000 Hz and needs at least two 30 Hz periods. This is a monophonic helper: periodic harmonics, stereo cancellation, noise and attacks can yield an octave error or rejection. Review estimates and assign an explicit root for drums or complex sounds. It does not tune/correct PCM.

`nearest_zero_crossing(audio,frame,radius)` searches the quietest mono-average sign crossing within a bounded radius; pass the result as a manual trim/loop boundary. Stereo channel crossings can differ. This is a boundary helper, not a loop similarity/phase-matching guarantee. Existing `sampling::detect_transients` and `sliced_zones` remain available for slicing.

Normalization measures the selected trim region peak and scales a copied entire source; selected-region timing remains unchanged. Silence is copied unchanged. Samples outside the selected region may saturate during normalization. Output is PCM16 WAV with the existing writer's rounding/saturation, without dither. Exported zones retain source frame offsets; no destructive trim or sample-rate conversion happens.

## Formats, persistence and compatibility

The output contains `instrument.json` (existing project schema 2), relative sample paths, `instrument.sfz`, and schema-1 `manifest.json` with output SHA-256 hashes, decoded-byte count and estimates. Root/product/zone state uses the existing project loader and sampler. Normalized PCM is copied alongside both formats, so sources need not remain available. No absolute source path is embedded in the exported project.

SFZ subset import now accepts `seq_length`/`seq_position`: each group/key/velocity/root sequence must contain every ordinal once. Imported alternatives are ordered by position and share a sampler group. Inherited `<group>` sequences are independent. Unsupported SFZ opcodes still fail explicitly; this is not the entire SFZ specification. Existing files without sequence opcodes retain their original grouping. Exported SFZ uses sustain/no-loop modes, velocity ranges and frame bounds supported by Soniclaw; native project is authoritative for future Soniclaw-only fields.

## Lifecycle, limits, errors and validation

Every operation here allocates and performs blocking file/CPU work. Run it on a control/authoring worker, then prepare/publish the resulting sampler at a stopped or safe engine boundary. No authoring call belongs on the audio callback. Bounds: 1–255 sources/regions, mono/stereo WAV, 128 MiB aggregate decoded PCM, pitch analysis input up to 32 Mi samples, zero-crossing radius up to 65,536 frames. Existing project and SFZ parser limits also apply. Invalid bounds, missing files, duplicate RR positions or unreliable pitch return an error; keep the previous instrument/export. An existing output is never overwritten.

Tests cover A440 detection, silence rejection, crossing selection, token parsing, normalized peak, native/SFZ mapping roundtrip, round-robin ownership, invalid-map rejection and refusal to overwrite. The runnable example additionally restores and plays the exported SFZ through the framework's actual project sampler. Source exports are not sample copyright/licensing clearance, a graphical sample editor or a commercial auto-loop/pitch-detection system.
