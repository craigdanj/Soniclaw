# Expressive instruments and note routing

`ExpressiveSynth<N>` is a new fixed-voice instrument. It adds MIDI pitch bend,
channel/poly pressure, CC74 timbre, sustain, explicit MIDI-1 MPE-zone mapping,
CLAP note identity, per-note tuning/pressure/timbre/pan/volume, configurable ADSR,
pressure-to-gain modulation and a timbre-controlled per-voice low-pass.
`MultiSampler` and `DiskSampler` support the same note-address/expression path,
with their own attack/release and sample playback behavior.

```rust
use soniclaw::prelude::*;
let mut synth = ExpressiveSynth::<32>::new(Waveform::Saw, AdsrSettings::default())?;
synth.set_mpe_zone(Some(MpeZone {
    master: 0, first_member: 1, last_member: 15,
    member_bend: 48.0, master_bend: 2.0,
}))?;
let expression = Event { frame: 64, message: MidiMessage::NoteExpression {
    target: NoteTarget { note_id: 42, channel: -1, key: -1 },
    expression: NoteExpression::Tuning, value: 7.0,
}};
```

Channels are zero based. Master and member bends add for a member channel; member
pressure/CC74 remain independently addressable. MPE zones are explicitly configured
while stopped. Automatic RPN zone negotiation, MIDI-2 UMP and MIDI-CI are not
implemented. The MIDI byte parser now emits channel and polyphonic aftertouch.

Native note-ons use `NoteOnId`; offs and chokes use `NoteTarget` selectors. `-1`
means wildcard. Matching applies all specified selectors, so two overlapping
same-key notes can remain independently addressable. Events retain sample offsets
through engine automation segmentation and CLAP translation. Pressure/timbre can
also arrive as MIDI channel/poly-pressure or CC74 messages. Expression values are
validated before instrument mutation.

| Expression | Value |
|---|---|
| Tuning | Semitone offset, -120 to 120 |
| Pressure | 0 to 1 |
| Timbre / CLAP brightness | 0 to 1, neutral 0.5 |
| Pan | 0 left, 0.5 center, 1 right |
| Volume | Linear multiplier, 0 to 4 |

The CLAP adapter routes these core events with note IDs and wildcard targets.
Vibrato, CLAP expression-controller modulation, per-note parameter-modulation
matrices and emitted NOTE_END notifications remain unsupported. MPE and CLAP note
expression are separate input paths with shared instrument controls.

The legacy `PolySynth`/`Sampler` APIs remain available and accept basic native
channel/key notes, but they do not provide note-ID isolation or the richer MPE
implementation. Use `ExpressiveSynth`, `MultiSampler` or `DiskSampler` for expressive
products. The exported demo synth now uses `ExpressiveSynth`; its stable product
and gain-parameter IDs remain unchanged, but its default timbre/filter behavior
can sound different from the legacy synth.

Expression updates apply at their event frame; this release does not smooth all
expressive controls or promise click-free hard chokes. `ExpressiveSynth` steals
oldest voices without a retiring crossfade. Polyphony, effect headroom and extreme
pitch behavior should be chosen/tested for the actual product.
