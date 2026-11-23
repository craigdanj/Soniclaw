# Live monophonic pitch correction

Use `pitch_correction::PitchCorrector` as a mono AudioProcessor.
Configure the YIN analysis window, hop and pitch range with `Settings`.
The existing phase-locked shifter supplies a separately configurable
synthesis window and optional formant treatment. Pass an absolute
note mask or create a scale with `Controls::scale(root, intervals)`.
Set strength, confidence threshold, retune time and note hysteresis.

The detector examines rolling source audio on a fixed sample clock.
Voiced estimates select the nearest permitted note. Hysteresis can hold a
previous note around a boundary. Correction moves toward the target at
the chosen rate. Unvoiced material and zero strength blend to delayed dry.
`estimate()` reads the latest pitch, selected note and applied semitones
from the audio owner. The shifter's declared latency is also the bypass
delay. Detection, retune and wet blend can take longer to settle.

```sh
cargo run -p soniclaw-dsp-suite -- pitch-correction corrected-notes
```

The example renders a detuned A against an aligned dry reference and
measures both after the detector has settled. Use a clean solo vocal or
pitched instrument source. Dense chords require a different approach.
Lower frequencies need a larger analysis window. Increase the hop or
narrow the range to reduce callback computation. This design schedules
analysis at hop boundaries; profile callback peaks for live products.

Prepare off the callback. Set controls on the same audio owner and use
reset at transport discontinuities. The engine accepts one mono channel,
8–192 kHz subject to the chosen detector range, and up to 8192 frames per
callback. Invalid updates keep their old values; invalid shape clears
output. Store settings and the scale/retune controls in presets.
Earlier pitch-shifter presets remain valid.

[Executed checks](validation/v0_132_0/REPORT.md).
