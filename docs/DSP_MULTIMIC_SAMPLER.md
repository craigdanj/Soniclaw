# Phase-locked multi-mic sample voices

A `multimic_sampler::Take` holds one recorded hit across any fixed set of
mic positions. Use one mono stem per mic, at the same sample rate and
length, with silence padding where necessary. Keep all stems aligned to
capture origin. Actual acoustic arrival differences stay in the audio.

Group takes into note/velocity `Zone`s. Construct `MultiMicSampler` with the
prepared sample rate, mic count, maximum callback size, and voice limit.
Use its `process` method with independent equal-length mono destinations
and block-relative MIDI. One hit selects one take and advances a single
cursor across every mic. Velocity controls the zone and a shared linear
gain. Mix the outputs in the application to change the mic balance.
Overlapping velocity zones for a note fail construction.

```sh
cargo run -p soniclaw-dsp-suite -- multimic-sampler drum-mics
```

The demo writes close, overhead, and room WAVs from the same generated
hits. Arrival gaps are encoded in the individual stems. Optional choke
groups fade earlier hits on the same MIDI channel over 32 samples.
One-shots play through ordinary note-off. All-notes-off applies the
same short fade; reset immediately clears voices and round-robin state.
When the pool fills, the oldest voice is stolen as a group.

Prepare on the control thread and use one mutable audio owner.
The resident backend supports 1–16 mics, up to 64 voices and a bounded
32-million-sample budget. Rate covers 8–192 kHz. Callback shape or invalid
events return an error before advancing playback. Save zone definitions,
ordered mic names and asset references in the application. Existing
single-zone and streaming sampler APIs keep their own mapping behavior.

[Executed checks](validation/v0_131_0/REPORT.md).
