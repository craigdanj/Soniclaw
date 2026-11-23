# Multi-operator FM and phase modulation

Use `soniclaw::dsp::fm_operators::{Patch, FmSynth}`. A patch describes four sine
operators in a fixed execution order. Each operator has a frequency ratio,
output weight, ADSR envelope and one-sample self phase feedback. In
`routes[from][to]`, an earlier operator can modulate a later operator. Set an
operator's output to zero to use it as a modulator only. Nonzero diagonals and
backward routes are rejected. Several parallel carriers or a serial operator
stack use the same representation.

Choose `Mode::Phase` for modulation depth in cycles or `Mode::Frequency` for
deviation relative to each carrier's frequency. Frequency mode permits a
negative instantaneous frequency. Self feedback uses phase in both modes.
Configure a patch on the control thread, allocate 1–64 voices, prepare the
synth, and pass ordinary sample-offset MIDI note events to AudioProcessor.
Note releases match one held voice by channel and key. When the pool is full,
the oldest releasing voice is reused first; a 32-frame fade joins the previous
sample to the new voice. Stereo repeats the mono mix. MIDI 1 basic notes and
all-notes-off are the supported messages in this engine.

```sh
cargo run -p soniclaw-dsp-suite -- fm-operators output/fm
```

The demo renders a short polyphonic phrase as FM bass, PM electric piano,
and PM bell WAVs and exports each patch as JSON. The audio callback uses
prepared memory. Compile new routing or prepare a replacement engine off the
audio thread; swap it at an application-defined boundary. Reset clears
notes and phases. The patch validates finite ratios of 0.125–16, route depths
within ±4, feedback within ±0.45 cycles and 0–10 second envelope times.
Invalid block dimensions or event offsets produce silence and leave voices
unchanged. The processor accepts 8–192 kHz and one or two channels.
Strong modulation and feedback can produce aliases. Applying external
oversampling requires retiming MIDI and checking source latency.

Persist Patch JSON with the app's preset version identifier. Voice history
is transient. Existing FM controls in the single-oscillator engine remain
compatible. [Executed checks](validation/v0_123_0/REPORT.md).
