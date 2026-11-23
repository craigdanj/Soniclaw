# Track freeze and bounce-in-place (0.47)

`soniclaw::studio::freeze` renders a track after its instrument/inserts and track delay alignment, before mixer gain, pan, mute, solo and sends. A frozen playback document references a float32 WAV through the disk-streamed clip engine and omits the original instrument/inserts. Those processors are not instantiated during frozen playback. The manifest preserves the original track for restoration.

## Example and API

```sh
cargo run -p soniclaw-freeze-session -- new-freeze
```

The example renders the synth track for two seconds plus half a second of tail, reloads and verifies the cache, compiles its disk-backed playback document and writes frozen/restored project JSON and a report.

```rust,ignore
let cache = freeze::render(&document, "lead", &assets, &disk_assets,
    &mut factory, new_directory, 0, 96_000, 24_000)?;
let (playback, catalog) = cache.apply(&document, &assets, &disk_assets)?;
let prepared = playback.compile_streamed(&assets, &catalog, &mut factory)?;
// Later, on the control thread:
let restored = cache.unfreeze(&playback)?;
```

`start`, selection length and tail use source frames. Rendering starts at frame zero to warm voices, automation and effects, discards common track latency, then writes the requested half-open selection plus tail. Mixer settings and aux returns are not baked into the stem. Existing timeline loops are honored; disable the loop before rendering a one-shot selection or a decay beyond its end. Tail is a caller-selected duration, not an automatic decay estimate. Dry input must be zero and live MIDI input disabled for the target track; there is no offline source for live performances.

`apply` returns a new document and catalog, leaving its arguments unchanged. Only the chosen track's processors/clips are replaced. Factory-reported parameters belonging to those processors are removed from the playback parameter snapshot and automation lanes. Mixer automation remains live. `unfreeze` restores the original track and removed parameter state/lanes while retaining current mixer edits. Stop/publish the new prepared document using the live controller's normal retirement rules.

`bounce` returns the same baked playback document/catalog as `apply`, allowing the application to commit the rendered audio as its new track content. Retaining `Frozen` permits undo; discarding that manifest is the application's explicit destructive commit. This is a whole-track processor replacement for a selected range, not region comping or a new DAW edit command.

## Integrity and invalidation

Schema-1 `freeze.json` contains the original document, removed parameter IDs, selection, source fingerprint and WAV SHA-256. `Frozen::read(directory)` relocates the manifest to the supplied directory and validates `stem.wav`. `apply` hashes the supplied original source configuration and assets again and rejects stale caches. Audio memory samples and all caller-supplied disk catalog files are hashed. Target track, timeline, rate, block size and non-mixer automation/parameter state participate. Unrelated non-mixer edits conservatively invalidate the cache; mixer gain/pan/send edits do not. Factory identity/version is not automatically discoverable: change the descriptor state/kind when the DSP implementation changes, or render a fresh cache.

The destination directory must be new. Render failure removes this newly created directory; it never overwrites an existing cache. WAV tampering or deletion fails validation. Cache identity `freeze.TRACK_ID` must fit existing 64-character IDs and must not collide with caller catalog entries. Very long track IDs can therefore fail `apply` without changing the project. Manifest validation is not a security sandbox for user code or native plug-ins.

## Ownership, limits and compatibility

Preparation, hashing, rendering, joins, JSON and disk operations run on the calling offline/control thread. No freeze work belongs in an audio callback. A fresh factory creates fresh processor state; ongoing opaque native state must first be captured into the caller's descriptor snapshot. Render state is not captured from an actively running instance.

This first implementation accepts memory-backed source sessions. It refuses sessions containing streamed sources, because an unpaced offline renderer cannot assume disk worker pages are ready. Materialize those sources into `Assets` before rendering. Live playback of the resulting frozen stem is streamed and retains the streaming engine's underrun telemetry. Bound: end time at most four hours and stereo float RIFF frame count below its four-GiB ceiling. Rendering is bounded in working PCM memory but can take selection-plus-warmup time and substantial disk space. No cancellation or automatic cache eviction is provided.

Existing project documents need no new fields; frozen playback is ordinary streamed-clip schema. Preserve `freeze.json` separately for restoration, and store catalog paths with the product's project resources. The original document inside the manifest can be large; reads limit it to 16 MiB. Older releases with streamed clip support can play the baked document but do not understand the restore manifest.

Tests verify actual stem samples, bypassed factory construction, preserved mixer edits on restore, source invalidation and rejected tampered files. The executable creates and reloads a real synth cache. See [validation](validation/v047/REPORT.md). Native opaque-state restoration and physical device playback are product acceptance work.
