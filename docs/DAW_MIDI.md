# Expressive MIDI clips and controller editing — 0.77.0

## Workflow and examples

Recorded MIDI now projects CC, 14-bit bend, channel pressure and polyphonic key pressure into clips alongside notes. Choose a region in the piano-roll selector, open **MIDI controller points**, select a point or New point ID, edit source beat/channel/type/value, and Apply. Remove point ID deletes an event. Project undo/redo covers both. Play publishes the edited model; points survive duplication, move, looping, trimming and portable Save/Open. These are stepped MIDI events; smooth interpolation is not invented.

Starting/seeking inside a clip restores controllers and sustained notes. Seeking after a key release while its pedal is still down reconstructs the voice and logical key release in order, then the recorded pedal-up releases it. Bend/channel pressure precede chased notes; poly pressure follows them. Loop pass projection also retains controller state and multiple pedal-held voices of the same key across the boundary. Raw performances remain separate from editable projections.

```sh
cargo run -p soniclaw-daw-midi -- NEW_DIRECTORY
cargo test -p soniclaw-clips control --all-features
cargo test -p soniclaw-studio midi_pass --all-features
```

The example records sustain/bend/pressure, edits a bend through history, reopens a save and seeks into a sustained key after release. Tests compare 64/127-frame callback partitions, chase/reset order, zero callback allocation, loop pedal-held voices, failed edits, legacy clips and desktop edit/undo actions.

## API and ownership

`clips::{Control,ControlEvent}` stores ID, source beat, zero-based channel and a typed MIDI value. `MidiClip::controls` defaults empty. `clips::Edit::{UpsertControl,RemoveControl}` validates a cloned candidate; failure leaves the prior model unchanged. `studio::midi_edit::{upsert,remove}` uses the unified memory/disk arrangement catalog. Wrap in project History::edit for undo. Raw take `note_clip` now includes controller points; its existing name remains for compatibility. `Player::last_events` exposes the prepared stream to adapters.

Preparation allocates, expands source repeats, orders events and builds controller timelines/seek interval trees off callback. Playback uses binary searches, precomputed release indices, fixed event capacity and stable bounded merging. It reserves chase capacity plus the reference device's 128 incoming events. Native owners/editors/trust remain unchanged; products interpret MIDI only to the extent their adapters implement it. No path in this feature loads native code merely by reading a project.

## Musical semantics

Old note-offs precede controller reset/value events and new notes at boundaries; poly pressure follows new notes. Point IDs order equal-beat controls deterministically. Callback merging preserves established equal-frame order, including the on/off pair needed to chase a pedal-held voice. Touched controls initialize at source phase and reset at the clip/end-repeat boundary, preventing stuck pedal/bend state. Untouched controls remain untouched. Defaults are CC7=100, CC10=64, CC11=127, CC70–79=64, other CC=0, bend=8192, pressure=0. Put an explicit point at source beat zero for a different initial value. The instrument's bend range is its own configuration.

Channel-mode CC120–127 are commands delivered only at their points; they are not chased as sticky state. CC121 resets tracked controller state. CC120 ends sounding-note chase; CC123 updates logical key release. Overlapping regions on the same channel share the resulting stream and deterministic document ordering. Use distinct channels/tracks for independent expression. Program changes, MIDI 2/MPE note editing and smooth controller curves are outside this clip format.

## Persistence and migration

Clip schema remains 1 with optional `controls`; old clips read empty and omit empty controls when serialized, preserving legacy portable-project identity derivation. Project schema remains 3. MIDI take schema 3 gains optional `Captured::chased` provenance, omitted when false. Initial chase notes can have a minimum one-sample logical span while sustaining instead of disappearing as zero-duration notes. Raw capture messages/timestamps are unchanged. Old strict readers may reject the new populated fields. Keep earlier copies when upgrading. Inspector fields/selection are UI state; actual points and take selections are portable.

## Bounds, failures and evidence

There are at most 65536 combined notes/control points per clip document, unique point IDs per region, channels 0–15, CC/key/value 0–127, bend 0–16383 and beats strictly before source_length. Repeat expansion and combined schedules are bounded at one million events. At most 128 sticky controller type/channel keys are chased per prepared track; channel-mode commands do not consume sticky state. Sustained voice overlap is capped at 128. Dense windows reject during preparation; playback never grows the event queue. The session's combined 1024-event contract still applies, including external messages.

Invalid values/IDs/ranges, over-dense schedules, excessive sustain overlap and missing regions reject explicitly. Raw performances whose panic/chase expansion exceeds the editable limit remain saved for splitting/filtering. Starting mid-note reconstructs MIDI state, not previous oscillator/sample/DSP history; exact waveform continuity across arbitrary seeks is not claimed. One-sample chase spans and integer MIDI quantization are explicit limits. Executed evidence is in this release's validation folder. Physical MIDI jitter, product-specific native interpretation and Windows/macOS runtime remain pending.
