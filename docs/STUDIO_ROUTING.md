# Studio routing editor — 0.66.0

## Workflow

Open Routing matrix in Reference Studio and Load routing from stopped project. Each row is a track's permanent direct master feed plus its auxiliary sends. Add up to four auxiliaries, adjust send gains, choose each bus's pre/post-fader tap, set return gains and add/remove inserts. The reference UI includes a simple gain insert to demonstrate the actual signal path; arbitrary supported descriptors can be supplied by the API. Prepare and apply routing compiles the complete candidate before replacing the project. Playback remains stopped; Play publishes the prepared model on the device. Global undo/redo includes routing changes.

This is an editor for the existing stereo studio topology: tracks feed parallel auxiliary buses and master; buses return to master. It does not add arbitrary track-to-track routing, feedback, hardware outputs, subgroups with independent processors or native sidechain bus negotiation. The separate lower-level graph/bus APIs remain available for those application designs. No unsupported edge can be drawn in this matrix.

## API and example

`studio::routing::{Draft, Edit}`. `Draft::new(project)` records the project identity and base document. `edit(Edit)` applies to a cloned document, validates, then adopts the candidate; failure leaves the draft unchanged. Edits cover AddAux, RemoveAux, Tap, Send, Return, Insert and RemoveInsert. `commit(project, factory)` verifies the unchanged base, validates the candidate and compiles the real streamed session before replacing the project; it returns the PreparedSession for stopped publication. Failure leaves the project unchanged. The reference UI drops its validation session and prepares again on Play so the device's actual specification remains authoritative.

```sh
cargo run -p soniclaw-studio-routing -- NEW_DIRECTORY
```

The example renders actual PCM through baseline, post-fader and pre-fader routes: with a half-gain dry channel, a unity post send doubles its amplitude while a unity pre send yields three times baseline. It rejects a stale draft, removes an aux through history, undoes and reopens the saved routes. `report.json` records measured RMS ratios. Unit tests cover automation/control remapping and failed processor preparation rollback.

## Controls and automation

Send gain is 0..4 and is reflected in both mixer defaults and normalized `mix.TRACK.sendN` parameter values. Enabled automation still overrides defaults at its timeline positions. Pre-fader is pre-strip-gain but post-pan and mute/solo gate, matching the existing mixer contract. Return gain and insert chains use the existing auxiliary renderer and latency compensation; return controls are static document settings, not new automation parameters.

Removing an auxiliary removes its send lane/value and its insert parameter lanes/values, renumbers later send lane/value targets and shortens every strip's sends and tap list. Removing an insert also removes its parameter targets. Other document state is retained. Effect IDs must be unique; descriptor validity and processor preparation are checked by the normal compiler. Add/remove structural edits do not occur in an active callback.

## Ownership, persistence and limits

All Draft APIs allocate and may validate/hash/compile media. They belong on the stopped control owner. Factory/native plug-in creation and destruction must respect the existing host thread ownership; a compiled session's old owners cannot be destroyed on a callback. The GUI explicitly stops before taking a routing draft and before committing it. A project edited after the draft was loaded causes a stale error; reload the draft to incorporate those changes.

The existing schema-1 studio document already persists mixer taps/sends, auxiliaries, inserts, parameter values and automation; no schema migration. Uncommitted drafts are UI-local. Portable save/recovery/history carry committed routes through the normal project model. Undo stops playback before restoring.

At most four parallel stereo auxiliaries and sixteen inserts per chain, with existing track/region/parameter/PDC budgets. Nonfinite/out-of-range gains, duplicate IDs, unknown indices/tracks, stale documents and failed factories are explicit errors. An invalid draft does not partially replace current routing. Reference UI return sliders use nonnegative 0..4; the underlying existing document supports polarity inversion via signed auxiliary gains. This release adds no new graph latency limit. All-feature compile/docs and actual offline audio checks are recorded in the verification report; native window, hardware and external DAW acceptance remain pending.
