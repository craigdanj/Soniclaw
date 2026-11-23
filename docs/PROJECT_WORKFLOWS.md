# Portable projects, background loading and recovery (v0.8.0)

Use `soniclaw::project_storage` and `soniclaw::project_loading` from the facade.
These APIs operate on control/worker threads. Nothing here belongs in an audio callback.

## Workbench

Run `cargo run -p soniclaw-workbench`. The scrollable left panel provides:

- **Load**: parse JSON, validate assets and build resources on a worker. A failed load leaves the document intact; playback stops when loading begins.
- **Pack project + assets**: export the current base values, routing, editors and WAV assets into a new `.soniclaw` file on a worker.
- **Import .soniclaw**: verify and extract into the specified **new** folder, then load it on a worker. Existing destinations are rejected.
- **Checkpoint**: enqueue one recovery snapshot. **Autosave every 30 seconds** is opt-in and saves five snapshots in the displayed recovery folder.
- **Recover latest**: find the latest valid snapshot for this product, skip corrupt/incompatible files, and construct it on the loader before committing it.

Recovery never rewrites a user's JSON or audio assets. Snapshots keep the original absolute asset root; moving/deleting external assets can prevent recovery. Use a bundle when archiving or transferring assets. Autosave captures current applied controls/edits; un-applied editor drafts, voices, transport and DSP histories are not captured. The interval runs while the app is open and responsive; this is not an operating-system background service.

## CLI walkthrough

```sh
cargo run -p soniclaw-sdk-tools -- --workflow-demo /new/workflow-folder
cargo run -p soniclaw-sdk-tools -- --pack /project/project.json /new/song.soniclaw
cargo run -p soniclaw-sdk-tools -- --unpack /song.soniclaw /new/project-folder
```

The walkthrough packages Sampler Studio, imports its WAV assets, writes a checkpoint on an autosave worker, loads recovery on the resource worker, and renders a one-second WAV from the prepared result.

## Bundle contract

Format 1 is `SLPK`, little-endian u32 version, manifest length and asset count, SHA-256 of the UTF-8 project JSON, the JSON, then one record per asset: little-endian u64 byte length, SHA-256 and WAV bytes. Asset ordering matches the manifest; paths must be exactly `assets/{index}.wav`. Limits: 4 MiB manifest, 1,024 assets, 128 MiB per encoded asset, 256 MiB total encoded assets. The resource factory retains its separate 128 MiB decoded project PCM budget. Bundles with trailing bytes, unsupported format, checksum mismatch, unsafe paths, bad WAV or excessive lengths are rejected.

Packing writes a synced temporary file and publishes with a same-filesystem hard link that refuses replacement. Recovery uses the same no-replace file publication. Filesystems must support hard links. Import exclusively creates a new directory; assets are verified before writing, and project.json is written last as the commit marker. Failed imports remove only that new directory. A crash can leave an incomplete import directory or hidden temporary file; choose a new destination or inspect/remove the incomplete directory before retrying. Extraction is not an atomic directory rename. File sync does not guarantee directory-entry durability after sudden power loss on every filesystem. Digests detect corruption, not trusted authorship; this is not an encrypted or authenticated archive.

## Resource loader

`AsyncProjectLoader::new(descriptor)` uses the built-in factory. `with_factory` supports an application-specific `Fn(&ProjectState, &Path)` factory that is `Send` and runs on a single worker. Requests support JSON files, boxed snapshots with an asset root, bundles with a new extraction directory and recovery stores with an expected product ID. JSON is bounded at 4 MiB. Pass an optional `ProcessSpec` to prepare DSP on the worker.

`submit` is nonblocking with two queued jobs; a full queue returns an error and retains the previous request. Each accepted request invalidates older results. `try_poll` exposes at most one latest result. `cancel` invalidates queued/in-progress results but does not interrupt I/O or undo an already extracted bundle. Drop joins the worker and can wait for disk operations. Do not drop a loader on an audio callback. Factories must validate their application product ID; the workbench does so. Native CLAP owners and window objects require their own main-thread lifecycle and are not supported by this Send-only loader.

`LoadedProject::into_processor` retains a worker-prepared spec; an identical `prepare` reuses resources without allocation. A different device spec triggers normal control-thread preparation. The workbench loads an unprepared plugin and retains it for the first Play, negotiating the real device spec at that point. Live playback, graph construction on subsequent edits and waveform preparation still have their existing control-thread work; this does not promise that all editor actions are asynchronous.

## Recovery store

`RecoveryStore::new(directory, retain)` accepts 1–32 immutable checkpoints. Give a store exclusively to one session/product. Snapshots use schema 1 envelopes with a checksummed JSON payload; `latest(expected_product)` skips malformed, corrupt and incompatible entries. Normal pruning happens only after a new synced checkpoint has been published. Keep adequate disk space; pruning errors report failure even if the new snapshot was committed. Snapshot ordering follows timestamp-based filenames, so large clock changes or competing processes are outside the ordering contract.

`AutosaveWorker` accepts one queued snapshot and returns completion/error through `try_poll`; backpressure is explicit. Submission clones state on the editor thread; filesystem and serialization work run on the worker. Shutdown waits for queued saves. Use manual checkpoints or a different interval in your application as needed.
