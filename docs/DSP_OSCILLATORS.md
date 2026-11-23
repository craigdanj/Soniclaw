# Oversampled modulated oscillators — 0.93.0

## Workflow and API

Use `soniclaw_dsp::oscillators::{Config, Controls, Shape, Oscillator}` in a voice or instrument without Studio. It is an audio source: input PCM is replaced. Existing basic/wavetable synth APIs keep their behavior. The new engine supplies pulse-width, sinusoidal frequency/phase modulation, fractional hard-sync reset, continuous phase warp and band-selected wavetable morphing, processed at an optional 2×/4×/8× rate.

Construct with schema-1 `Config` and an optional `Arc<WavetableBank>`, then prepare. Default is 4× High-quality conversion, a 220 Hz saw and gain 0.2. Wavetable shape requires a bank. `set_controls` validates all controls before updating and preserves phases. Calls belong to the audio owner or an application command queue; parameters are held for the processed block. Abrupt shape/frequency changes can still click/alias; this is not a polyphonic MIDI synthesizer or a replacement for voice management.

Run `cargo run -p soniclaw-dsp-suite --offline -- oscillators NEW_DIRECTORY`. The example writes PWM, FM, phase-warp, sync and wavetable WAVs, measures callback allocations and compares an owned high-frequency pulse fixture with/without oversampling. Its FFT report excludes the fundamental; for that 50%-pulse fixture higher odd harmonics exceed Nyquist. The measured reduction is signal-specific.

## Signal path and timing

`Controls` exposes frequency, pulse width, FM frequency/depth, PM frequency/depth in cycles, hard-sync ratio 1–8, warp −0.95–0.95, morph and gain. The warp is a monotonic periodic sine displacement of phase. A fractional reset advances the slave through the remainder of a master-crossing sample. Wavetable rendering reuses existing harmonic mipmaps and selects conservatively for the modulation bandwidth estimate. Oversampling attenuates aliasing; it does not make hard sync, rapid modulation or phase warping mathematically alias-free.

This engine addresses source latency explicitly: because its input is replaced, it adds a delay equal to the unused interpolation half of the converter. Its reported full conversion latency therefore matches the nominal generated-source path. At 4× High this is 96 base-rate frames (48 decimation plus 48 explicit padding). FIR pre-ringing can precede the group-delay peak; a latency value does not mean that every earlier sample must be zero. The generic 0.89 effect wrapper still has its separately documented source/MIDI caveat.

## Lifecycle, persistence and bounds

Construction/bank preparation and `prepare` may allocate. Rendering and reset use prepared buffers and state. One processor owns its phases; channels are dual-mono copies, with mono/stereo preparation supported at 8–192 kHz. Core oversampling buffer/rate limits apply. Frequency is 1–20000 Hz and must be below 0.45 of base rate; FM/PM frequencies and FM depth are 0–20000, PM depth 0–4 cycles, pulse width 0.02–0.98, morph/gain 0–1. Instantaneous slave rate is limited to 0.45 of the high rate. Invalid controls preserve the old controls; invalid/unprepared buffer shapes are silenced. Oversampling changes require an off-thread replacement. Reset clears phases, converter history and source padding.

Persist Config (schema 1, unknown fields rejected) and an application-owned bank reference; bank PCM/phase history is not serialized in Config. Use immutable shared banks, and do not release their last owner from a callback. Product IDs and old presets are unchanged.

## Validation

Executed checks are in `docs/validation/v0_93_0/`. Fixtures cover callback partition equality with combined modulation, stereo coherence, reset, source padding/reporting, invalid controls, zero allocations and an owned pulse alias comparison. Generated examples invite listening review; no human approval, universal spectral limit, hardware/DAW acceptance or unrun platform claim is implied.
