# Tempo-aware streamed audio (0.56)

`studio::tempo_cache` turns original mono/stereo WAVs, including mixed sample rates, into verified project-rate playback caches. Source-frame/beat warp markers place audio on the musical timeline. PreservePitch uses the existing stereo-linked WSOLA stretcher; Repitch changes duration and pitch together. The result plays through ordinary disk-backed regions, retaining the stream engine's bounded page cache and underrun telemetry.

## Run and use

```
cargo run -p soniclaw-tempo-stream -- NEW_DIRECTORY
```

The example generates a stereo 44.1 kHz, 440 Hz source, fits two beats to 90 BPM in a 48 kHz project, checks exactly 64,000 frames and preserved tone frequency/stereo proportion, then saves/reopens a portable project. `report.json` records measurements. Unit coverage additionally exercises a tempo ramp, exact marker endpoints, source immutability, stale detection and cancellation cleanup.

Create a `Recipe { schema: 1, markers, mode }`. Each `Marker` maps an absolute original `source_frame` to an absolute project `beat`. Markers strictly increase on both axes. Call `prepare(source_path, source_asset_id, recipe, timeline, project_rate, NEW_DIRECTORY, cancel)` on a control or worker thread. It creates `audio.wav` and `warp.json`, returning a `Binding` with source/timeline/output hashes and exact project range. Call `Project::apply_warp(track_id, region_id, cache_asset_id, binding, audio_path)` to verify and commit a streamed region plus its portable metadata. Original disk assets must already be in `Project::disk`.

A cache ID can be rebuilt into a new directory and replaced through the same method. Replacement requires the existing cache to be owned by the named region; shared dependents must be rebuilt explicitly. New unrelated asset/region collisions reject without changing the project. Keep the old rendered session running while preparation happens, then use the existing session replacement transaction. Direct edits to the public project data still require validation and a rebuild before publication.

## Musical timing and sound

Every marker and tempo knot uses `TempoMap::sample_at`, so independently rounded segment lengths telescope to the exact final endpoint. Step changes split sections at their beat. Linear ramps are approximated by constant average stretch ratios in sections no longer than half a beat. They have exact section endpoints; this is not continuously varying intra-grain realtime warping. Add denser explicit markers where transients need alignment. This implementation uses fixed 512-frame grains and a 64-frame search, with existing transient protection and stereo-linked search. It is not a transient detector or automatic beat analyzer.

Windowed-sinc conversion first maps each source section into the project rate; PreservePitch then stretches that converted stream between 0.5 and 2 times its duration. Repitch performs direct resampling to the target length. At most one rounding frame is held/trimmed to hit an exact endpoint. Each section has a 16-frame edge ramp to reduce discontinuities. Marker joins may still produce audible level dips or grain artifacts; audition complex/polyphonic material and use offline listening tests. No claim of commercial time-stretch quality is implied.

## Thread ownership, resources and failure

Preparation performs disk I/O, allocation, hashing and WAV writing. It must not run in an audio callback. A host may spawn it on its own worker; the cancellation atomic is checked between 1,024-frame chunks and sections. There is no implicit worker or unbounded job queue. The resampling read window is fixed at 33,796 frames, at most two channels; input/output blocks are 1,024 frames and WSOLA uses its fixed bounded buffers. PCM memory use is independent of source duration. Only marker/tempo metadata grows, within the limits below. Existing playback makes no callback filesystem calls; ordinary disk misses remain visible silence.

Sources must be mono/stereo WAV at 8–192 kHz. Integer PCM and float32 decoding are supported. Samples must be finite and at most 1e6 in magnitude. The sinc conversion ratio is limited to 1/32–32; pitch-preserving time ratios are 0.5–2. Recipes allow 2–256 markers; preparation allows at most 8,192 sections, four hours of output and the float-WAV RIFF size bound. Very dense/short mappings with fewer than two frames in a section reject. Cache preparation refuses an existing destination. Errors, cancellation and detected concurrent source modification remove only the newly created directory. It never deletes or rewrites the source WAV.

## Persistence, invalidation and migration

`Binding::verify` checks recipe/rate/range, WAV headers, original/cache SHA-256 and the complete timeline fingerprint. A changed tempo, marker binding, source file or cache requires a rebuild. Changing unrelated timeline metadata conservatively invalidates too. Verifying hashes occurs off callback. The portable project stores optional schema-1 `warps` metadata and copies both originals and rendered audio into its resource catalog. Resource identities survive relocation. Existing projects read with an empty map; older strict project readers may reject the new field. At most 32 cache bindings fit a project, alongside the existing total of sixteen live streamed regions. Repeated rebuilds should retire old cache folders only after their prepared players have been retired.

[Release evidence](validation/v056/REPORT.md) records executed software checks. Hardware playback, long-file throughput and listening acceptance on target systems remain separate work.
