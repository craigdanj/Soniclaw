# Audio and MIDI clips (0.35)

## Workflow and runnable example

`cargo run -p soniclaw-clip-studio -- NEW_DIRECTORY` creates a five-second stereo preview WAV, arrangement JSON and a block-partition comparison report. Add `--gui` for the native arrangement/piano-roll editor, or `--gui --snapshot NEW.png` for a reproducible native screenshot. Drag regions/notes, double click to insert notes, adjust lengths/velocities/fades and render the edited draft. Preview rendering and JSON writing run on the control thread; a production application should move those jobs to its worker. This demo does not open an audio device.

Use `soniclaw::clips::{Document,AudioClip,MidiClip,Note,Assets}`. Load/resample decoded PCM off the callback into immutable `AudioAsset` arrays. Audio positions, trim offsets, fades and lengths use project frames; MIDI uses quarter-note beats and the document tempo map, including tempo changes. `Document::apply(Edit)` validates an entire draft before replacing it and returns the previous document for undo. Move, trim, resize, duplicate, fade, note upsert/removal and clip removal are supported. Duplicate IDs fail rather than replacing another region.

```rust,ignore
let previous = document.apply(Edit::MoveAudio {
    id: "Bass-loop".into(), frame: 24000, snap_frames: 6000,
})?;
let mut player = document.compile(&assets, 256, Some(Box::new(instrument)))?;
player.render(&mut output, transport)?;
```

## Playback and lifecycle

Compile/prepare on a control thread, publish a newly prepared player at a safe engine boundary, and retire the old player off the callback. Compilation expands looped MIDI through `TempoMap`, sorts note-offs before note-ons, validates density, builds an interval tree for seek recovery and allocates all callback scratch. The instrument is prepared once. Do not mutate the draft or assets from the audio callback. `Player` implements `AudioProcessor`, or use fallible `render` and inspect `last_events` for routing to another prepared MIDI destination.

Audio and the optional instrument sum into the project channels; mono assets map to every output channel. Linear fades apply at region boundaries; source offsets and loop periods select PCM without copying it. Audio is delayed by the instrument's fixed reported latency. Track numbers are edit metadata: this player mixes its selected document into one bus/instrument. Build separate players from filtered documents for independent mixer tracks. Cross-clip audio summing can exceed unity; provide output gain/limiting as appropriate.

Playing a discontinuous sample position sends panic and reconstructs held notes at frame zero; stopping sends panic once and permits instrument tails. This restores note ownership, not historical oscillator/envelope phase. Delay state is invalidated without clearing a large buffer on the callback. `prepare` allocates; explicit `reset` forwards reset to the instrument (some native bridge processors revoke their run on reset). Do not call reset to seek a native bridge. A failed `process` clears output, increments `failures` and invalidates source continuity. Direct `render` callers handle the returned error. Incoming context events must be empty; live MIDI merging is deliberately explicit in the application.

## Persistence, limits and recovery

Schema 1 JSON stores edits/timeline and asset IDs, not decoded PCM or arbitrary plug-in code. `to_json`/`from_json` validate and limit JSON to 16 MiB. Reconnect assets by stable ID before compilation. Keep the previous validated JSON/player on errors; no automatic migration from unrelated project/session formats occurs. Existing projects remain unchanged.

Bounds: 256 regions, 64 track tags, 1–64 channels, sample rates 8–384 kHz, blocks up to 8192, 64 Mi decoded samples across distinct used assets, 65,536 source notes, one million expansion candidates/events, 500,000 compiled note spans, 128 simultaneous held notes and 262,144 latency frames. The callback reserves 1024 MIDI events, including seek recovery. A chosen native plug-in bridge can impose a smaller packet/event capacity; validate against that processor's documented contract. The default UI shows six arrangement lanes and MIDI keys 48–71; the data model supports all MIDI keys. It does not provide an infinite scrolling DAW arrangement, audio warping, sample-rate conversion, waveform decoding, live graph swapping or automatic native event-capacity negotiation.

## Validation

Unit tests cover transactional rollback/JSON, fades/loops, mono mapping, uneven block equivalence, zero callback allocations, exact MIDI offsets and loop boundaries, held-note recovery, stop panic, instrument latency alignment and rejection of excessive polyphony/expansion. The runnable example compares real synth output at 64/256 frame blocks and exports a WAV. See the release verification report for executed commands and native editor evidence.
