# Polyphonic pitch and onset analysis

Call `soniclaw::dsp::polyphonic_analysis::transcribe` with mono PCM and
`Settings`. The analyzer scores overlapping, centered Hann windows,
interpolates local spectral peaks, and greedily selects note candidates
whose fundamentals and harmonics explain those peaks. It tracks candidates
across hops and returns half-open frame ranges, MIDI notes and relative
spectral confidence. Convert frame positions to seconds using the input rate.
The tuning reference is A4 = 440 Hz.

```sh
cargo run -p soniclaw-dsp-suite -- polyphonic-analysis output/synthetic-chords
cargo run -p soniclaw-dsp-suite -- polyphonic-analysis output/note-events recording.wav
```

The fixture uses two chords with six known pitches in total. The second command
accepts a mono WAV. It writes a CSV with note start and end times.
For real recordings, narrow the MIDI search range to the instrument, limit
polyphony to the expected chord size, and adjust the relative threshold.
Use a larger FFT to distinguish nearby low pitches or a smaller window for
better onset resolution. Confidence describes spectral support relative to
the frame, so it is useful for filtering or manual review.

This is an offline control/worker-thread analyzer. Each call owns its FFT,
scratch and note events. It accepts finite mono audio through 8,388,608 frames,
8–192 kHz, a power-of-two window of 1024–32,768, 1–12 simultaneous candidates,
and MIDI bounds 24–112 subject to FFT resolution and Nyquist. Missing
fundamentals, notes with shared harmonics, mistuning, vibrato, fast attacks and
dense mixes can cause missed or extra notes. Onsets are bounded by STFT window
and hop resolution. The tracker bridges a single missing analysis hop.

Keep the recording rate with saved note events. This API does not alter
source recordings, MIDI schemas, or existing real-time mono pitch detection.
Invalid bounds or nonfinite data return an error. Retry with valid settings
or downmix/select the relevant channel before analysis.

[Executed checks](validation/v0_126_0/REPORT.md).
