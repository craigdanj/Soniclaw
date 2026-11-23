# Studio native plug-ins — 0.83.0

Studio can instantiate explicitly trusted stereo CLAP and VST3 effects and instruments on tracks, auxiliary returns, buses and the master. This release connects the existing native hosts to the Studio project lifecycle; it does not establish compatibility with every third-party product.

## Workflow

Build `cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT_FOLDER [TRUSTED_NATIVE_PATH ...]`. Expand **Native plug-in manager**. Enter one module file or bundle path, choose its format, approve its code and dependencies, and select **Scan module**. Discovery runs in a child of this executable with a ten-second timeout. A crash, timeout, changed fingerprint, or malformed result prevents installation. Failed scans quarantine the module until a successful rescan. Supervision is process fault containment, not a security sandbox.

Select a discovered enabled product, destination track/bus/master and instrument/effect role, then install. Playback constructs the native instance. Instruments require an empty track instrument slot; remove a stock instrument first. Auxiliary effects remain supported through the public descriptor API. Product metadata does not prove instrument compatibility; select an appropriate role.

Cache save/load is explicit. Loading metadata grants no execution permission. Reapprove the selected module before installation or repair, and supply startup trust again after reopening a project. **Toggle bypass** works for missing modules without loading code: effects preserve the last saved latency and instruments produce silence. A never-run instance has no known latency, so its initial bypass latency is zero. **Repair path** requires a matching format and product ID, refreshes the module fingerprint, and rebinds a verified snapshot. Repair removes obsolete inline CLAP state; save before repair if it is your only legacy checkpoint.

While playing, the native inspector provides parameter values and editor open/close. CLAP supports the existing window preferences. VST3 uses its own native window; size/focus/scale operations may be unsupported and report an error. Stop before changing graph structure. Restart/rescan requests are surfaced; stop and rebuild playback rather than modifying an active native layout.

## Public API and thread ownership

Enable `soniclaw/studio-hosting`, or `soniclaw-host` features `studio,vst3`. `formats::TrustedLoader` is a non-Send capability constructed with an unsafe, explicit native-code trust contract. Construct it, `studio::NativeFactory`, and all `studio::Owner` instances on their controlling thread. `NativeFactory::create_detailed` returns a `Built` processor and exposes useful host errors; its `Factory` implementation preserves the existing generic Studio error contract and records `last_error`. The desktop also logs the native error with the processor ID.

The factory restores opaque state first, then writable parameter values. Products that do not implement opaque state require an explicit **Parameter-only checkpoint** selection (`state_mode:"parameters"`); samples, hidden settings and other non-parameter data then cannot be restored. A failed opaque capture is reported, never silently treated as a complete checkpoint. It accepts legacy single-slot CLAP `state.session` after identity checks. CLAP uses direct audio leases with no bridge latency. VST3 uses the musical worker bridge, carrying MIDI notes, supported expression, tempo, meter and loop context; it reports 1024 frames plus native latency for graph compensation. The bridge requires real scheduling time and can underrun under a fast offline loop. Offline export is integrated separately in 0.85.0.

Drop graph/audio leases before clearing owners. Stop the device and finish recordings before capturing each owner with `Owner::snapshot()`. A snapshot stops processing, closes the editor, reads state and parameters, and includes pending host parameter targets. Native editor changes that never reached the plug-in cannot be recovered by a host snapshot.

`registry::{scan,child,fingerprint,Registry,Entry,descriptor}` provides the reusable manager. The scanner helper accepts `--native-scan-child FORMAT MODULE OUTPUT`. The helper and selected module/dependencies must be trusted. `Registry::usable` rechecks enabled/quarantine status and fingerprint before installation. The live factory also checks a saved descriptor fingerprint before loading. Native module files must remain trusted and unchanged during use; fingerprints do not eliminate filesystem races or validate dependencies.

## Portable state, migration and history

`Project::native_states` maps stable processor IDs to immutable `native_state::Blob` resources. Each blob contains `Metadata` (schema 1, format, absolute module, product, version, parameter map, byte length, SHA-256) and shared raw bytes. JSON serializes metadata only. Saving writes `native-states/N.bin` before the project manifest. Reopening verifies bounded paths, size and digest without loading a module. Existing schema 1–3 projects default to an empty resource map; the outer project schema remains 3.

A descriptor uses kind `clap` or `vst3`, `state.path`, `plugin_id` or `class_id`, optional `module_sha256`, `role`, `bypass`, `latency_frames`, and `native_state:true` when a blob is required. A missing or mismatched required blob rejects the project. Native binaries are not copied into portable projects. Reinstall and explicitly approve the same products on the destination system.

History shares immutable payloads and counts unique retained bytes. Generation saves, pending recovery and consolidation use the normal Project serializer. Track duplication remaps native resource keys together with descriptor IDs. Frozen originals retain their snapshot bindings. Removing an instance leaves its resources retained for undo and frozen originals; there is no automatic native-resource garbage collection.

Bounds: 4 MiB per opaque blob, 256 resources, 128 MiB aggregate raw state, 65,536 aggregate stored parameter values; the existing project 2 GiB resource and 16 MiB manifest limits still apply. Descriptor JSON remains bounded independently, so large blobs must not be embedded inline. Registry: 4096 products, 256 quarantines, 4 MiB JSON; scan output 1 MiB; module fingerprint 1024 files / 512 MiB. Bundle symlinks are unsupported. On error keep the previous project/cache, inspect the message, bypass or repair deliberately, and rescan changed modules.

## Runnable example and evidence

`cargo run -p soniclaw-daw-plugins -- NEW_FOLDER` checks a 128 KiB blob round trip, metadata size, undo/redo, missing-module bypass latency and corrupted-resource rejection. Add `clap TRUSTED_MODULE` or `vst3 TRUSTED_MODULE` to scan and run an explicitly selected owned fixture, measure callback allocations, snapshot, save and reconstruct. Never pass an untrusted module.

Executed results are recorded in [the release verification](validation/v0_83_0/REPORT.md). Native fixture checks establish this integration on Linux only. Physical audio/MIDI, native editor interaction, broad third-party acceptance and Windows/macOS remain separate acceptance work. This is framework and reference-application infrastructure, not a claim of a complete commercial DAW.
