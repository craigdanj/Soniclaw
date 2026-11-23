# Media audit and consolidated copies — 0.70.0

## Workflow

Expand Media audit and consolidation in Reference Studio and Audit registered media. The shared worker inspects dependencies and displays each memory/disk resource as retained or unused in a new copy. Enter a **new** destination directory and Write consolidated copy. The source project and original files remain unchanged. After success, Open that new folder to work from the consolidated copy; the studio does not silently replace the current project/history.

This feature omits unused registered assets while writing a new portable project. It never deletes the originals, searches arbitrary user directories or prunes unknown filesystem files. It conservatively preserves disabled regions (which may be enabled later), every saved take, freeze histories and original assets required to unfreeze, and source/cache chains for retained tempo warps. Custom/native processor descriptors have opaque dependencies, so **all registered resources are retained** when any are present. External plug-in sample files remain outside the core project's dependency knowledge; the report warns that these are not discovered/copied automatically.

## API and runnable example

`studio::media::{audit, Plan, Report, Entry, Storage, consolidate, Consolidated}`. `audit(project)` returns a Plan with a read-only `report()` and an internal fingerprint. Report lists typed Memory/Disk IDs, byte counts and retained flags plus warnings. `consolidate(project, &plan, new_directory)` re-audits, rejects a stale fingerprint, prunes an isolated project clone, writes through the existing portable saver, reopens/validates the result and writes `media-audit.json`. It returns the Consolidated report. No native library is instantiated by either operation.

```sh
cargo run -p soniclaw-studio-media-audit -- NEW_DIRECTORY
```

The example creates used and unused resources plus a warp and a frozen track. It rejects a stale project plan, omits exactly two unused entries, reopens the copy, verifies warp/original-source retention, successfully unfreezes the track and compares original file bytes unchanged. Unit tests additionally retain disabled clips and keep all assets for an opaque custom processor.

## Reachability and validation

Roots include all active document audio/streamed regions, including disabled ones, and every frozen original document. Retained warp-cache assets recursively retain their original source. All take/freeze folders are copied by the core portable saver; their history is preserved even if no current comp uses them. Orphaned warp metadata is omitted only if its generated asset has no retained reference. Missing referenced core assets fail the audit. Arbitrary strings in custom plug-in state are not guessed to be reliable asset references.

The plan fingerprint covers project identity/name/document, freeze/take/comp/warp metadata, registered disk paths and disk file bytes, and memory asset format/sample bits. Consolidation verifies it again before copy and checks registered source content after copy. The portable saver/opener separately validates their take/freeze/resource manifests and checksums. This is not a hostile concurrent filesystem transaction or a snapshot of external native plug-in files. No directory fsync or universal power-loss guarantee is added.

Byte totals concern direct registered entries only. Memory bytes mean logical decoded f32 storage (four bytes per sample), not unique Arc allocation/RSS or exact encoded WAV size. Disk bytes mean registered file sizes; aliases can count the same physical data more than once. Take/freeze directory storage and filesystem overhead are excluded. The totals describe candidates omitted from a copy, not source files deleted or guaranteed physical space reclaimed.

## Ownership, persistence and recovery

Audit/hash/copy/reopen run on a stopped control worker, never an audio callback. The studio uses its shared scheduler with a 64 MiB working reservation; project PCM snapshots are shared separately and estimates are not allocator-enforced limits. Core hashing/copying are synchronous within the worker: scheduler cancellation can suppress a result but may wait for current filesystem work and does not interrupt every copy. A successfully committed output can remain even if result cancellation or a project-epoch change wins afterwards. The UI states this behavior and never deletes an unrelated destination.

Plan/queue/UI selection are transient. Report schema is 1; the project schema remains unchanged. A consolidated copy retains the source project identity and committed core data; editor-window/view sidecars and in-memory undo stacks are not carried by the core saver. Original project history remains available in the original running session; opening the new copy resets its transient history. Re-run an audit after edits or media changes. Stale plan, missing resource, copy/hash error and invalid dependency are explicit failures, leaving the source untouched. Failed output validation removes only the newly created job-owned destination. The destination must stay exclusively owned by the job until completion.

Limits inherit the project's 1,024 registered resources, 64 Mi decoded samples, 32 freezes/warps and 256 takes plus existing portable copy/metadata bounds. This is conservative consolidation rather than deduplication, orphan-folder deletion, take trimming, destructive cleanup or external sample-library packaging. See the release verification for actual executed checks and limits.
