# Studio recovery, migration and media repair (0.61)

The unified studio can now save verified portable recovery snapshots on a background worker, restore an older valid snapshot when the newest is corrupt, upgrade its earlier project manifests and repair a missing audio resource using its original checksum. Recovery owns a dedicated store and never loads native plug-in code during inspection or restoration.

## Run and use

```sh
cargo run -p soniclaw-studio-recovery -- NEW_DIRECTORY
cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT
```

The software example removes/relinks a known audio resource, migrates a schema-1 project, saves schema 2, runs two background checkpoints, corrupts the newest snapshot, leaves an incomplete snapshot directory and restores the earlier valid one. It then rotates recovery storage and verifies the restored working project still opens. `report.json` records these outcomes. It opens no device or native window.

In the studio open **Recovery and missing media**, choose a dedicated recovery folder and interval, then explicitly enable periodic autosave. The default interval is thirty seconds; nothing is saved periodically until enabled. **Checkpoint now** captures a stopped project through the same worker without enabling periodic saves. Only one job is retained; busy controls reject duplicate submissions. Disable stops future scheduling, while an already accepted snapshot finishes and reports its result. Changing store/interval settings is disabled while periodic saving or an active job would conflict.

**Restore latest verified snapshot** validates recovery content and copies the selected snapshot into a new working folder before replacing the active project. If the newest is corrupt it tries older complete snapshots, reporting warnings. It never overwrites a working project. Opening another project disables the prior autosave association and clears project/editor history. Restoring also clears histories and discards unapplied comp drafts.

To repair missing media, enter its project folder in the main Project folder field, click **Inspect project media**, select a reported resource and enter the original replacement file path. **Repair selected missing media** accepts byte-identical content only. Open the folder after repairing it. Existing files, including damaged files, are never overwritten; move a damaged file aside deliberately before restoring its exact original. Take/freeze directory repair is not provided by this asset-file helper; use a complete recovery snapshot or restore those original folders.

## Project identity and schema migration

`Project` gains a stable public `project_id`. New projects generate an opaque identifier from time, process and sequence data hashed with SHA-256. This is an identity, not an authentication secret or globally certified UUID. Save As preserves it; an explicit fork can assign a new valid identity before creating a separate recovery store. Rust struct literals must supply this field; using `Project::new` remains the normal constructor.

Portable project manifests now save **schema 2** with `project_id`. `Project::open` accepts schemas 1 and 2; `open_with_report` additionally returns `OpenReport { project, source_schema, migrations }`. Older schema-1 documents without an identity receive a deterministic identity derived from their canonical manifest. Reloading that same legacy manifest, including after folder relocation, produces the same identity. Saving it writes schema 2; unsupported future schemas and malformed/missing schema-2 identities reject. Earlier defaults for absent comp/warp metadata remain supported. Older strict readers cannot read schema 2; keep an original copy when sharing with older framework versions. Native product/parameter IDs, take and freeze schemas do not change.

`resource_issues` reads bounded manifest metadata and checks typed memory/disk resource identities, paths and SHA-256. `repair_missing(folder,id,kind,replacement)` validates the original checksum, performs a bounded copy to its own temporary file, syncs/rehashes and publishes with a no-replace hard link. Memory and disk IDs may coincide, so callers must provide ResourceKind. The target filesystem must support hard links. A changed source, wrong file, escaping manifest/resource parent or existing target rejects without replacing that target. Native binaries are not project audio resources and are never located or loaded automatically.

## Recovery store and ownership

`recovery::Store::open(folder, project_id, Limits)` exclusively creates a new dedicated folder or recognizes a matching store marker. It rejects unrelated folders and another project's identity. A standard OS file lock serializes writers; process termination releases that lock. Do not modify this managed tree concurrently with the API.

`checkpoint(project)` writes a complete portable project into an exclusively created generation folder. A bounded inventory of relative files, sizes and hashes is written as `recovery.json` last. Incomplete generations are ignored and their numbers are not reused. Ordinary write failures remove only the newly created snapshot. Process death may leave an incomplete folder, which is retained for inspection rather than mistaken for a valid backup. Commit/resource verification detects corruption; file and directory durability after sudden power loss is filesystem/platform dependent.

`inventory()` lists complete commit metadata and warnings without loading processors. `restore_latest(NEW_WORKING_FOLDER)` checks the complete inventory and project, tries older candidates after corruption, copies the chosen data outside the managed store and reopens that copy. Restored audio must not depend on a directory that retention can delete. Checkpoint therefore rejects projects whose disk/take/freeze paths still point inside its recovery store; use the restore API rather than opening a managed snapshot as a live working project. A failed restoration does not mutate an application project; adopt the returned candidate only after success.

Retention removes oldest committed snapshots only after verifying their exact owned-file inventory. Unexpected or modified files block deletion and produce an explicit warning on the successfully saved result. Another save is refused if prior blocked retention already exceeded its count/byte policy. Incomplete crash folders are not automatically removed or included in committed-snapshot byte accounting; inspect them explicitly. Defaults retain at most five committed snapshots within four GiB. Peak free-space needs include retained snapshots plus one incoming portable copy; full snapshots are not incremental backups or a realtime edit journal.

## Background scheduling and limits

`Autosave::new(store, interval)` accepts one to 3,600 seconds. `tick(elapsed, applied_project)` uses caller-provided monotonic elapsed time, clones applied metadata/shared PCM and starts at most one worker when due. There is no queued backlog. `checkpoint_now` submits immediately if idle. `poll` joins only a finished worker; Drop joins on the caller and can wait for disk I/O. Schedule, poll, restore, repair and destroy off the audio callback. Current restore/repair/open UI actions are control-thread operations and can pause the reference interface; snapshot copying itself runs on its worker.

The reference app syncs applied core parameter values before scheduling. It postpones periodic snapshots while native owners are active because their fresh opaque checkpoint requires the stopped main-owner lifecycle. Stop or Checkpoint now captures that state first. Voices, transport/DSP history, unapplied editor drafts and unfinished recording data are not restored as project state. No DAW/hardware crash guarantee is inferred from a portable snapshot.

Store limits are 1–32 snapshots and one MiB–eight GiB of committed retained data. Metadata reads/writes are bounded, recovery inventory is capped at eight MiB/20,000 files, root enumeration at 4,096 entries and tree depth at 32. Symlinks in snapshot trees reject. Existing portable project bounds still apply, including its resource-copy and decoded-memory budgets. Hashing uses fixed 64 KiB buffers. Readiness and retention errors are returned explicitly; callers must show them and retain the current project.

Tests exercise migration, exact-content repair/no overwrite, writer exclusion, one-job autosave scheduling, interrupted-generation exclusion, corrupt-newest fallback and working-copy survival during retention. The actual headless reference studio additionally checkpoints and restores its applied frozen project. See [executed release checks](validation/v0_61_0/REPORT.md). Physical-device, native-window, third-party host and abrupt-power-loss acceptance remain separate target-machine work.
