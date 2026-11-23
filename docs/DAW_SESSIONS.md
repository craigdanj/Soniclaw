# Durable in-place sessions and portable performances — 0.73.0

## Workflow

The reference Studio now offers **Save**, **Save as**, and Command/Ctrl+S. Set Project folder, Save once, then keep saving to the same folder. Save as requires a fresh destination. Open folder resolves the latest committed generation. Save runs in the shared worker scheduler after playback/recording stops and the live model is synchronized. Further edits can proceed while resources are copied; the saved baseline is the captured snapshot, so later edits still show unsaved. Wait for the completion message before quitting. New/Open/restore are blocked during an active save.

Completed MIDI takes are now included as `midi-takes/N/midi-take.json`, plus valid optional `performance.mid`, in project saves, recovery snapshots and consolidated copies. This preserves raw sustain, bend, pressure and control messages independently of editable note regions. Failed note application still retains the raw performance. The original external performance folder is not needed to reopen the saved copy.

While MIDI or automation recording is active the GUI checkpoints pending work every two seconds to a sibling `PROJECT-pending` journal. Eight committed prefixes are retained; incomplete final writes fall back with warnings. In Recovery choose **Accept and recover pending MIDI prefix** or **Recover pending automation**. Open the saved original project first. MIDI recovery closes held notes at the checkpoint boundary and marks provenance; lost/unsupported/discontinuous input is refused. Automation recovery requires the same project and unchanged armed lanes/structure; it restores stopped gestures and commits through project history. The journal is not automatically applied, and recovery omits events after the last successful checkpoint. Save the armed session before a critical performance. Raw audio-file salvage follows in the recording workflow; this journal covers MIDI and automation.

```sh
cargo run -p soniclaw-daw-sessions -- NEW_DIRECTORY
```

The runnable example creates two save generations, captures a synthetic MIDI prefix with bend, journals/recovers it, deletes the external raw take and reopens the saved copy. No device is opened.

## API and ownership

`studio::sessions::save(&Project, root)` returns `Saved { directory, generation }`. `current_directory(root)` resolves the checked head or a standalone legacy project. `Project::open`, `open_with_report`, resource inspection and repair accept either. `Project::save_new` still exclusively creates a standalone portable folder. Use save for repeated updates. `Project::midi_takes` stores durable raw performance directories; all metadata reads are bounded and validated.

`midi_recording::Recorder::checkpoint(input_dropped)` drains on the control side while the processor continues. `Take::accept_checkpoint` explicitly accepts a lossless interrupted prefix. `automation_write::Writer::{checkpoint,recover}` preserves pending lanes, project identity and base structure. `pending::Checkpoint::{save,load}` stores these payloads. All cloning, validation, hashing, locking and filesystem operations run outside the audio callback. The GUI submits saves/journal writes to bounded jobs; one project save at a time, no growing checkpoint queue. A 256 MiB save reservation covers the maximum shared memory asset capture plus metadata; reservations may temporarily reject other work.

## Persistence and migration

Each save exclusively creates `session-store/generation-N`, writes a self-contained schema-3 project, syncs its files, then atomically replaces `soniclaw-session.json` with generation and project-manifest SHA256. A writer lock rejects simultaneous writers. An existing legacy folder is adopted only after verified identity matching; its original files remain. Arbitrary nonempty folders and different project identities are refused. Generations are immutable by contract; opened projects and undo history may refer to older generation media. No automatic deletion can invalidate those references. Consolidate to a new root when retention fills. An interrupted head replacement leaves the previous committed project available; partial generations are retained and skipped by future generation allocation. Unrecognized marker creation failures require inspection or a new folder.

Project schema 3 adds an optional raw-MIDI count; absent counts mean zero, preserving legacy schema-1 identity derivation. MIDI take schema 2 adds interrupted/recovered provenance and accepts schema 1. Previous readers with strict fields may reject these new projects/takes; retain old source copies. Visual plug-in-window/editor sidecars remain separate UI state and are written after the model commit. Sidecar failure can report an error after the model has already saved; Open still resolves the committed model.

## Bounds, errors, validation and limitations

There are at most 128 retained save generations; each is an independent copy subject to the existing two-GiB portable resource bound, 16 MiB manifest and 64 Mi decoded-sample budget. Saving large projects requires free space for a complete generation and copies/hashes media; this release does not deduplicate resources. Failed resource copy leaves the prior head unchanged. The root/marker/head and generation paths reject symlink redirection for managed metadata. Owned generation contents remain subject to Project checksum/path checks. Local filesystem atomic rename and file synchronization are used; network filesystem behavior and sudden-power-loss durability require target validation. Replacement failure on a target OS reports an error and keeps the previous head.

Pending payloads are capped at 32 MiB, 64 directory entries and eight retained committed files; raw MIDI remains bounded by its existing event/duration limits. A process crash can lose the last two seconds plus an unfinished background write. Journal recovery is a prefix, not proof of a complete performance. Existing physical MIDI timestamps are still callback-block aligned until the timing release. Tests cover repeated save, wrong-project refusal, failed resource copy, changed-manifest rejection, journal retention/torn-write fallback; desktop tests exercise asynchronous save/reopen and existing undo/freeze/recovery. See adjacent release evidence. Physical hardware and Windows/macOS runtime were not inferred from these checks.
