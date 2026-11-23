# Studio MIDI performance recording — 0.71.0

## Workflow

Open Reference Studio with `cargo run -p soniclaw-desktop-studio --features gui -- --gui PROJECT`. Expand **MIDI input and performance recording**. Refresh input ports and select one, or choose On-screen keyboard only. Ports are opened only when Play/Record starts and remain fixed until the next restart. Normal Play and audio recording now also accept these bounded MIDI sources; live monitoring follows the document's `midi_input` flags. The small keyboard sends note-on while a button is held and matching note-off on release, preserving the original channel even if settings change.

For a MIDI take select an instrument track, disable arrangement looping, enter the starting beat, optional onset grid (quarter-note beats), optional end quantization, capture channel and duration limit. Enter a new take folder or leave it empty for a generated one. Start MIDI take creates a fresh engine and routes live MIDI to the selected track; existing arrangement clips still play and are not recaptured. The capture channel filter limits recorded messages, while monitoring still receives the original input. Finish MIDI take/Stop ends capture; the duration limit also finalizes through the control thread.

Stopping first closes input, stops/drops the audio processor and checkpoints native owners. It saves `midi-take.json`, exports `performance.mid` when supported, then compiles the complete project candidate before appending a new note clip. Existing regions are retained: this is overdub insertion, not destructive replacement. The normal piano roll edits the new notes; global project undo/redo includes the insertion. Incomplete, empty-note, duplicate-ID, stale-timeline or unpreparable results retain raw evidence and do not publish a clip. Save last raw take to new folder can recover a retained take after a filesystem error. Use a different new folder for subsequent takes.

## Public API and runnable example

`studio::midi_recording::{Settings, prepare, Armed, Processor, Recorder, Status, Take, Captured, Message}`. Validate/round-trip Settings with `to_json/from_json`; input is bounded to 16 KiB. Call `prepare(project, factory, settings, latency_budget)` while stopped. It primes disk sources at the chosen start, enables callback clock staging and returns Controller, Processor and control-thread Recorder. Publish Processor through Engine and supply input Events. Do not reprepare, seek, reset, replace graphs or attach an audio-recording tap during a MIDI take. Graph replacement/audio-tap attachment is rejected while the MIDI owner is alive. The reference studio starts separate audio and MIDI takes.

Poll `Recorder::drain` and `status` on the control thread. `limit_reached` signals exact duration completion. Stop/drop the device Processor before `Recorder::finish(host_input_dropped_count)`: finalization while running is rejected. The host must include losses from its upstream input bridge; Reference Studio uses the fresh device telemetry counter. Internal capture loss, unsupported messages, reset/seek discontinuities and external loss are separately reported. A take is complete only when all loss/discontinuity counters are zero.

`Take::note_clip(track_index)` creates a note-only `MidiClip`. `apply(project)` validates and appends an isolated candidate; `apply_prepared(project, factory)` additionally compiles before publication and returns the PreparedSession. Use `History::edit` to make this one undoable operation. Project identity, sample rate and exact timeline must match; IDs cannot collide with an existing audio, streamed or MIDI region. Unrelated committed project edits are retained rather than overwritten. The reference studio discards its validation session and prepares again on Play for actual device configuration.

```sh
cargo run -p soniclaw-studio-midi-recording -- NEW_DIRECTORY
```

The hardware-independent example captures a supplied performance through actual prepared session blocks, compares 64/127-frame partitions, measures zero callback allocations, round-trips raw JSON, exports/reads controller and bend data in SMF, quantizes/appends/prepares a clip, undoes/redoes and reopens a portable project. `report.json` records assertions. It does not simulate physical-port timestamp accuracy.

## Timing, notes and controllers

Events supplied with an intra-block frame offset are stamped using the real session clock at that offset, including tempo changes within a block. Starting at a nonzero beat retains absolute project sample positions in the raw take. Conversion to notes uses the same TempoMap. This is allocation-free callback capture, not a new operating-system MIDI timestamp service. The existing physical-input bridge emits queued messages at the next audio callback boundary; physical timing therefore has block/queue granularity. GUI key timing also includes UI scheduling. No physical input latency calibration is claimed.

MIDI 1 note-on/off, CC, pitch bend, channel/poly pressure and global panic are retained as typed messages. Velocity-zero note-on releases a note. Retriggering the same channel/key closes its previous note; overlapping independent voices on one MIDI 1 channel/key are not distinguished. Orphan offs are retained in raw data and ignored by note conversion. CC120/123 and panic close held notes; otherwise held notes close at the actual take end. Onsets can snap to a beat grid; ends snap only when requested. Rounding to the same point creates a bounded minimum one-sample span rather than an invalid zero-length note. Quantization affects the editable note clip, not raw data or raw SMF export.

The current studio MidiClip stores notes, so **CC/pedal/bend/pressure playback is not inserted into that clip**. Pedal does not extend editable key-held durations; those messages remain in the raw take and SMF. Existing native input Parser consumes program changes, SysEx and system messages without emitting them, so this is not a lossless wire-byte recorder. Note-ID/note-expression messages are unsupported by this MIDI 1 capture module; they count as unsupported and prevent publication as a complete take. Existing lower-level expressive MIDI APIs remain available.

## MIDI file export

`Take::smf(ppqn)` uses the existing type-1 SMF codec. It retains supported recorded controllers and notes at unquantized timing, shifted to take start and rounded to PPQN ticks. It exports stepped tempo changes, beginning with the tempo at take start, and appends pedal release/all-notes-off for every channel to avoid hanging output notes. Global panic expands across all channels. Event expansion is bounded before allocation grows past the codec's limit. Supported PPQN is 1..32767; Reference Studio uses 960. Time signatures are not emitted by the existing codec.

Continuous tempo ramps are explicitly rejected for SMF rather than silently approximated. Capture, JSON persistence and beat-note conversion still support the prepared ramp map. A ramp take can therefore yield a valid editable clip and raw JSON while displaying an SMF-unavailable warning. Incomplete takes cannot be exported as complete SMF; keep their JSON for inspection.

## Ownership, storage and migration

The callback owns Processor and one producer; Recorder owns the consumer and bounded control-thread event vector. A separate clock-only capture flag publishes timestamps without copying track PCM. Queue admission, atomic counters and event stamping do not allocate, lock or write files in the callback. Factory/native owners remain on their permitted thread; UI connection creation/destruction, hashing, JSON/SMF work, quantization and candidate compilation are control-side operations. Drop/stop ordering must follow the existing device/native-host ownership contract. Disconnect MIDI before dropping audio; only then finalize.

Raw Take/settings schemas are 1 with strict fields, validated bounds and 16 MiB/16 KiB input limits respectively. `save_new` exclusively creates a raw-take directory and refuses overwrite. The reference app writes SMF only inside that newly owned folder. The latest finished take also remains in memory for explicit saving. Pending capture is transient; a crash before finish can lose it. There is no pending-MIDI autosave journal.

The existing portable project schema is unchanged: committed note clips participate in save/recovery/history. **Raw JSON/SMF take folders are separate artifacts and are not copied by core Project::save_new/recovery.** Keep/export those folders separately when moving controller-rich performances. Port choice, take settings, keyboard state and pending/last-take UI state are app-session preferences, not portable project data. GUI builds now enable the already-existing `soniclaw/midi-input` feature; core/offline builds still do not require a native MIDI backend.

## Bounds, failures and evidence

One instrument-track target per take; linear arrangement only; start beats within 0..1,000,000; capture channel None or zero-based 0..15; grid 0..16 beats; duration 1..14,400 seconds; 128..65,536 queue entries and total accepted events; default 4,096 queue/65,536 event slots and 600 seconds. Conversion bounds note count to 32,768 plus normal session event-density/polyphony constraints, which `apply_prepared` verifies. Duration end is exact even when the last callback crosses it. A full queue/event budget reports loss and refuses complete publication. Upstream bridge overflow also triggers its existing panic mechanism and is counted at finish.

Settings/schema errors fail before device publication. Invalid/unsupported events and discontinuities mark the take incomplete; raw JSON can preserve discontinuous sample order for inspection while note/SMF publication remains blocked. Save failures retain the finished take in memory. A source-timeline change or failed factory leaves the current project unchanged. No retry overwrites an existing directory.

Tests cover partition invariance, callback allocations, nonzero starts and tempo boundaries, quantization, channel filtering, retrigger/panic/held notes, capacity/unsupported/upstream loss, duration cap, stale timeline, raw/SMF round-trip and undoable prepared insertion. The headless reference-studio test exercises finish/save/append/undo/redo without opening hardware. Physical MIDI/audio, native window interaction, Windows/macOS runtime and external DAW acceptance remain pending. Exact executed commands and evidence are in this release's verification report.
