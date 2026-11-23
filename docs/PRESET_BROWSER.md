# Preset and asset browser — 0.10.0

## What this adds

`soniclaw::catalog` stores complete project presets with searchable metadata. `PresetCatalog` is independent of the native window/device stack. With facade `ui`, `catalog::browser::PresetBrowser` supplies a reusable egui list and action interface. The workbench's eleventh tab, Presets, integrates storing/loading, worker-rendered previews, optional audible audition, metadata editing and missing-asset repair.

## Workbench walkthrough

1. Run `cargo run -p soniclaw-workbench -- --tab 10`.
2. Choose a dedicated writable catalog directory and click Open catalog. Nothing is stored until requested.
3. Select an instrument/effect project, adjust its controls, return to Presets and click Store current project. The complete applied project is captured; un-applied graph/sample editor drafts are excluded.
4. Select the entry. Enter a display name, comma-separated tags and a bank name, then Save metadata. Favourite toggles its favourite flag.
5. Search uses case-insensitive AND words across name, bank and tags. The bank dropdown and favourites filter further restrict results. Entries sort by display name and stable ID.
6. Load reconstructs the project with the existing background resource loader. Playback stops before loading; a failure preserves the previous document.
7. Render preview renders a two-second preview on a worker without changing the loaded document. Wait until it finishes, then Audition preview opens the selected output device to play that rendered clip. Stop ends playback; normal Play switches back to the loaded project. A reconnect during audition restarts the clip. Hardware playback requires a functioning configured device.
8. Check asset references lists missing, inaccessible or escaping asset references. It runs on request, not every repaint.
9. To repair moved files, set the new asset root. Existing relative paths are preserved; optionally enter one asset ID and replacement relative WAV path. Validate and relocate checks every project asset before committing. If multiple paths changed, use the API's mapping argument.
10. Export selected writes a new portable `.soniclaw` bundle including its WAV assets. To import a bundle, use the existing project Import control, then Store current project. Banks are searchable grouping labels; this release does not implement a monolithic bank-file exchange format.

Remove from index removes a catalog listing, retaining immutable document files for recovery. The workbench has no automatic deletion of sample files.

## Library API

```rust,ignore
use soniclaw::catalog::PresetCatalog;
let mut catalog = PresetCatalog::open("/writable/presets")?;
let id = catalog.insert(&project, &asset_root,
    vec!["Ambient".into(), "Pad".into()], "Factory".into())?;
catalog.edit_metadata(&id, "Warm Pad".into(),
    vec!["Ambient".into()], "Factory".into(), true)?;
let hits = catalog.search("warm ambient", Some("Factory"), true);
let (project, asset_root) = catalog.project(&id)?;
```

`entries` and `entry` expose metadata; `project` verifies the immutable document checksum before parsing. `missing_assets` returns asset IDs/expected paths. `relocate(id, new_root, mappings)` rebinds the root and optional asset-ID-to-relative-path overrides transactionally. It requires all assets to resolve within the new root, including symlink checks through ProjectState. `export_preset` uses the existing bounded portable bundle writer and refuses an existing destination.

`PresetBrowser::show(ui, &catalog)` performs no file/audio actions itself. It returns `BrowserAction` for the application to handle. Its public query, bank, favourites_only and selected fields permit application-specific state persistence. The workbench handles those actions on the control thread and uses its resource/render workers for expensive construction/rendering. Catalog metadata and relocation I/O are blocking bounded control-thread operations; applications with slow/network storage should dispatch them to their own resource worker.

## Audible previews

`PreviewPlayer::new(Arc<AudioFile>)` validates a finite, nonempty decoded clip. `prepare` accepts an output ProcessSpec and establishes linear sample-rate conversion; `process` interpolates from immutable PCM with no file I/O or heap work. Source channels are mapped by index, with extra output channels repeating the last source channel. A finished clip produces silence. Reset restarts its position. The player owns no device; use an Engine/DeviceManager to route it. Linear preview conversion is an audition convenience, not a high-quality export resampler.

## Persistence and recovery

The catalog uses schema 1 metadata envelopes in `catalog-NNN.json` files and SHA-256-addressed immutable `presets/<digest>.json` documents. A separate stable entry ID survives metadata edits and asset relocation. Entry asset roots are absolute external paths; presets remain references until explicitly exported as a portable bundle.

A successful metadata commit writes and syncs a new generation with create-new semantics before updating memory. The newest valid index is selected on reopen; a corrupt newest generation can fall back to an earlier valid generation. Three index generations are retained. If all indexes are corrupt, open returns an error instead of silently presenting an empty catalog. Preserve the folder and restore a known good generation manually. A crash can leave an incomplete newer generation or orphan immutable document. Directory-entry durability after power loss depends on the filesystem; no universal transaction guarantee is claimed.

Use one writer per catalog. Competing processes are not coordinated; create-new generation conflicts fail. Immutable documents remain after logical deletion/relocation for recovery; no garbage collector is provided. Size storage accordingly.

## Bounds and errors

- At most 4,096 entries and 4,096 index generations considered during open.
- Index envelopes at most 16 MiB; each project uses the existing 4 MiB bound.
- Names: 1–256 bytes; bank names up to 128 bytes; at most 16 nonempty tags of up to 64 bytes.
- Duplicate identical project insertion reports already present. Unknown IDs, invalid metadata, missing files, corrupt checksums and invalid relocation produce errors.
- Only generated immutable project paths are accepted. Asset resolution retains existing project root/symlink restrictions.
- Callbacks must never open/search/edit catalogs, decode files or render projects. Only the already prepared PreviewPlayer belongs in a callback.

## Compatibility and verification

Existing project schema 2 and exported product/parameter IDs are unchanged. The new catalog format is separate from project state. Existing preset JSON and portable bundles still load through the original workflows. Applications need the same custom processor registrations when loading catalog entries that use them.

Run `cargo test -p soniclaw --test v010` for persistence/search/recovery, relocation/export, invalid data and allocation-free preview tests. `cargo run -p soniclaw-sdk-tools -- --catalog-demo /new/demo` generates a reusable two-entry catalog, preview WAV, selected portable bundle and report. Open it with `cargo run -p soniclaw-workbench -- --catalog /new/demo/catalog --tab 10`.

See the [release validation report](validation/v010/REPORT.md). Physical audible preview, device-specific latency and real DAW acceptance require target-machine tests and are not inferred from offline rendering.
