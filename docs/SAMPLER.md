# Multisampling and streaming

`soniclaw-sampler` adds `SampleBank`, `Zone` and `MultiSampler<N>` on top of the existing validated `Sample` PCM container. A bank is immutable while playing; samples are owned for the bank's lifetime, so note-off and voice stealing never release the final PCM owner on the render thread.

```rust
use soniclaw::prelude::*;
use soniclaw::sampler::{MultiSampler, SampleBank, Zone};
use std::sync::Arc;

let file = soniclaw::io::read_wav("snare.wav")?; // control thread
let sample = Sample::new(file.samples, file.channels as usize, file.sample_rate as f64, 38)?;
let zone = Zone::single(0, 38);
let bank = Arc::new(SampleBank::new(vec![sample], vec![zone], 64 * 1024 * 1024)?);
let mut engine = Engine::default();
engine.push(MultiSampler::<64>::new(bank));
```

Zones specify inclusive key/velocity ranges, sample index, gain, one-shot behavior, a round-robin group and optional `[start,end)` loop frames. Matching zones in the same group are alternatives; different groups layer together. Assign a different group per drum to keep their round-robin sequences independent. At most 1024 samples, 1024 zones and 256 groups are accepted, with a caller-specified PCM memory budget.

Sustain CC64 holds released gated voices. CC123 releases notes according to sustain; CC120 and global `AllNotesOff` silence the appropriate voices. Pitch bend is per MIDI channel; set the pitch range while stopped (2 semitones by default). This is not full MPE support: per-note pressure, timbre and CLAP note-expression routing are not implemented.

Voice stealing retains one retiring voice per slot and crossfades old/new audio over 5 ms. A second steal in the same slot replaces its single retiring tail; unlimited overlapping steal tails are not promised. Gated voices have a 50 ms linear release. One-shots ignore note-off/sustain. Looping and linear interpolation are supported; pitch shifting is not band-limited.

`examples/reference` includes a deterministic, synthesized kick/snare/hat bank: two velocity layers and two round-robin variations per drum, with no downloaded samples. `soniclaw-reference-studio` uses the exact same instrument factory standalone. Its `--render output.wav` path runs without audio hardware.

## Long-file streaming

`soniclaw::stream::open_wav(path, capacity_frames)` returns a control-thread `StreamHandle` and an audio-thread `StreamingPlayer`. Call `handle.prebuffer(...)` before starting the device. Keep/drop the handle on the control thread; dropping it joins the file worker. Process receives only bounded queue data and never reads files or locks.

Streaming is forward-only mono/stereo WAV at the original sample rate. Underflow inserts silence and advances the timeline, with explicit counters; stale queued frames are discarded with bounded catch-up work. EOF produces silence. `reset()` clears no transport position; open a new player to rewind. Seek, resampling, looping streams and disk-backed multisampler voices remain future work. The multisampler itself still preloads PCM.
