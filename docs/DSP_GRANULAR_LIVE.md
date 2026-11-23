# Tempo-synced live granular effects

Construct `granular_fx::LiveGranular` with history in milliseconds, grain
pool size, and Controls. Prepare it as a mono or stereo AudioProcessor.
Feed live audio with a transport sample position, tempo and play flag.
`interval_beats` is the grain trigger interval. The internal beat clock
starts when playback starts and follows tempo per block. A transport
seek during playback clears history and restarts that clock.
Keep positions contiguous when streaming.

Freeze suspends writes and keeps triggering from captured history.
Use delay, size and pitch to change which section is read.
The causal read requires enough history to fit a whole grain at its
playback rate. Until enough history exists, the engine counts a missed
launch. It also counts misses if the prepared grain pool is full.
Captured stereo channels share every start and read position.
The wet control blends with direct input.

```sh
cargo run -p soniclaw-dsp-suite -- granular-fx grain-demo
```

The demo stops the source while wet playback continues from a frozen
buffer. Use an interval of 0.25 beats for sixteenth-note triggers.
Update controls on the audio owner. Trigger intervals are relative to
the instance start or seek; provide a musical transport when using it
inside a host. Freeze captures audio temporarily. Persist the controls,
then refill history when loading a preset.

Prepare off-thread for 8–192 kHz, 1–2 channels, up to 8192 frames per
block, 100–4000 ms of history and a pool of 1–64 grains.
The host tempo range is 20–400 BPM. Invalid controls preserve existing
state, invalid buffer shapes are silenced, and reset invalidates
history without clearing allocated memory. Musical history is part of
the effect, with no fixed extra delay on the dry path.
Existing MIDI-triggered granular synthesis remains available.

[Executed checks](validation/v0_135_0/REPORT.md).
