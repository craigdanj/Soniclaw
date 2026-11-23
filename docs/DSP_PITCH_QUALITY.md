# Pitch envelope controls and quality presets — 0.94.0

## Workflow and API

`soniclaw_dsp::pitch_quality::Quality::{LowLatency, Music, Voice}` constructs the existing prepared phase-vocoder `PitchShifter` or push/pull WSOLA `TimeStretcher` with explicit tradeoffs. Pitch windows are 512/2048/4096 frames; stretch windows 512/1024/2048 with search radius one eighth of the window. Voice enables optional envelope preservation; Music and LowLatency retain the old pitch path. These are algorithm settings, not a guarantee of better sound for every source.

```rust
use soniclaw_dsp::{pitch_quality::Quality, streaming::FormantSettings};
let mut pitch = Quality::Voice.pitch(5.0)?;
pitch.set_formants(Some(FormantSettings {
    shift_semitones: -2.0, ..Default::default()
}))?;
# Ok::<(), soniclaw_core::Error>(())
```

Use `prepare(spec)` then the normal AudioProcessor callback. `set_formants(None)` disables correction; old PitchSettings constructors continue to produce the original behavior. The setting is held for a block and must be changed by the audio owner or its command queue. Pitch remains within ±12 semitones; pitch duration stays fixed. Formant shift moves the estimated spectral envelope independently of pitch. This log-magnitude moving-average estimate is an approximation: it does not identify vocal-tract resonances, phonemes, preserve every transient, or provide commercial vocal editing.

## Algorithm, timing and quality bounds

For each analysis frame, a prefix sum averages log magnitudes over ±`smoothing_hz` (clamped to 1–256 bins). A temporal one pole follows that envelope. Output magnitudes receive the ratio of the desired shifted envelope to the pitch-shifted envelope, blended by amount and limited by maximum gain. Stereo channels estimate separately. Disabled and unity-preserving paths retain their previous output. Enabling after disabling reinitializes the envelope; changing controls is not a click-free automation promise.

FormantSettings schema 1 rejects unknown fields, nonfinite values and out-of-range controls: amount 0–1, shift ±12 semitones, smoothing 50–2000 Hz, time 0–200 ms, gain limit 0–24 dB. Invalid updates preserve the previous settings. Pitch supports 8–192 kHz mono/stereo and reports its existing STFT latency (`window_frames`); allocate latency compensation accordingly. Larger windows improve frequency resolution at the cost of latency and transient spread. The extra envelope operation adds no separate frame delay.

`Quality::stretch(ratio, channels, max_block_frames)` uses shared WSOLA offsets across channels; rate 0.5–2 and channels 1–64. `priming_frames` is a streaming requirement, not the pitch latency. Push at most max_block_frames, resubmit unaccepted frames, pull with a bounded grain budget (0–64); after input call finish and pull until is_drained. Do not busy-wait on an audio callback when neither input nor output is available. Ratio is fixed for that stream; use an off-thread replacement for another ratio. See PITCH_TIME.md for detailed backpressure semantics.

## Lifecycle, persistence and recovery

All envelope and prefix buffers allocate in prepare; processing, setters, reset and prepared push/pull allocate nothing. One owner mutates state. Reset clears spectral and envelope history. Persist the quality choice, old PitchSettings/StretchSettings and optional FormantSettings separately. Existing schema-1 pitch presets require no migration and default to no correction; runtime analysis history is not serialized. A host must silence/stop on prepare failure and retain the prior valid processor on configuration rejection. No Studio or plugin host is required.

## Example and executed evidence

`cargo run -p soniclaw-dsp-suite --offline -- pitch NEW_DIRECTORY` writes a synthetic vowel-like harmonic fixture pitched up five semitones with correction off/on and an independent envelope shift, plus a 1.5× duration stretch. JSON reports latency, finite samples, zero measured processing allocations and the exact 48000→72000 stretch length. Unit fixtures check unchanged unity output, rejection without mutation and callback partition invariance. Release checks: docs/validation/v0_94_0/. These are deterministic synthetic checks, not listening approval, human voice fidelity or a universal quality score.
