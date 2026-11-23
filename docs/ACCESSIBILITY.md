# Accessible audio controls (0.50)

Custom knobs, faders and filmstrips now expose AccessKit slider semantics and assistive actions. Arrangement clips and piano-roll notes have semantic selection and keyboard editing; accessible lists expose offscreen objects. Native eframe products enable the platform AccessKit adapter. This is an accessibility foundation with executable semantic tests, not a claim of complete screen-reader certification.

## Audio controls

`audio_knob`, `audio_fader`, `Filmstrip::show` and `parameter_knob` announce parameter name, stored numeric value/range, step and codec-formatted value text. `accessibility::parameter_semantics` is public for additional painted parameter controls. It handles Increment, Decrement, numeric SetValue and text SetValue (via the parameter codec/parser). Changes set `Response::changed`, so existing gesture/undo integration can observe them.

Continuous increments use one percent of normalized range; stepped parameters use one discrete step. SetValue uses stored parameter units and clamps to metadata bounds. For typed normalized parameters the numeric range remains 0–1 while formatted text supplies physical units; no false Hz numeric range is invented. Nonfinite numeric requests and invalid text are ignored. Disabled controls ignore assistive modifications and omit modification actions. Egui handles focus/activation through its standard response plumbing.

Arrow keys adjust focused audio controls; the main toolkit also supports Home/End, double-click reset and Shift fine pointer drags. Toolkit knobs draw a focus ring. Boolean toggles, enum controls and standard sliders use egui's native semantics. The XY pad now has two ordinary parameter sliders below it, giving each axis independent keyboard/screen-reader access. Mixer labels include strip names so several Gain or Mute controls can be distinguished. Meters announce displayed numeric levels through existing progress/text widgets; they do not continuously shout live announcements.

## Arrangement and notes

Tab/Shift-Tab can focus interactive clip/note regions. Enter/Space selects. On a focused clip, Left/Right moves by configured snap (audio: at least one frame; MIDI: fallback quarter beat), Delete removes it. On a focused note, Left/Right moves in beat snap, Up/Down transposes a semitone and Delete removes. All changes use validated document edits.

“Accessible clip list” and “Accessible note list” provide native selectable controls, including objects outside the painted viewport. Select an item there to expose start position or note beat/pitch/length/velocity fields. “Add note at start” is a keyboard-accessible alternative to double-click creation. Existing Remove selected/Delete note buttons remain available. The painted view retains six arrangement lanes and MIDI keys 48–71; the note list and numeric pitch editor can access notes outside that painted pitch range. This release does not add an automatic viewport-follow system or a full keyboard command map for every editor operation.

## Integration and ownership

All semantics, actions and editing execute on the UI thread; the audio callback still reads shared parameters. No screen-reader or layout work belongs in the DSP callback. Native eframe applications in this workspace enable its `accesskit` feature, including editor products using that dependency. Other consumers must enable the adapter in their chosen egui integration. A custom plug-in embedding backend must route the accessibility tree/actions to the platform; merely rendering a texture does not export accessibility. This release does not add that native bridge to every custom embedded editor host.

For deterministic headless audits call `Context::enable_accesskit()` before building frames:

```sh
cargo run -p soniclaw-accessibility-audit -- new-accessibility-report.json
cargo test -p soniclaw-ui
```

The audit writes actual tree roles, labels, values, ranges and supported actions for toolkit knob/fader controls. Tests inject real AccessKit actions, verify parameter changes and disabled behavior, and send focused note keyboard events through the real editor. A native screen reader may activate tree production automatically through the platform adapter.

No project schema or parameter identity changes are needed. Extra XY controls affect layout height. New native adapter dependencies are recorded in Cargo.lock; Linux uses the platform accessibility bus. See egui's [AccessKit context API](https://docs.rs/egui/0.36.1/egui/struct.Context.html) and the pinned source for integration details.

## Validation limits

[Release checks](validation/v050/REPORT.md) include workspace all-feature lint/docs, eight UI tests and an actual semantic-tree JSON audit. Orca, VoiceOver and NVDA end-to-end use, platform focus navigation, native editor embedding and user testing remain pending. Do not infer full application accessibility from the audited controls alone.
