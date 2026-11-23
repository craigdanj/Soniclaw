# Waveguide strings and modal resonators — 0.95.0

## Workflow and API

`soniclaw_dsp::physical::WaveguideString` is a single fractional-delay string voice. Construct from schema-1 StringConfig, prepare, call `pluck(amplitude, pick_position, seed)` on the audio owner, then render with AudioProcessor or sample-by-sample `tick`. Input is replaced; mono/stereo outputs are copies of the same voice. A deterministic noise difference shapes pick position. Feedback supplies frequency-dependent loss and an allpass dispersive section. Low-frequency filter delays are subtracted from the requested period; fractional interpolation and dispersion still introduce tuning/decay error, especially at high notes.

`set_config` changes tuning, brightness, nominal decay, dispersion and a bounded friction exciter without clearing the sounding string. A nonzero bow_pressure continuously injects `tanh` friction driven by bow_velocity. This is an exploratory excitation model, not a calibrated violin, guitar body or physical bow/string contact solver. Tune by ear and render comparisons before using it in an instrument. Parameter jumps and new plucks can click; a voice/envelope manager belongs to the application.

`ModalBank::new(ModalConfig { schema: 1, modes: Vec<Mode> })` prepares up to 32 independently damped sinusoidal resonators. Each Mode specifies frequency, exponential amplitude decay to −60 dB, and signed gain. The AudioProcessor input's first channel excites every mode; outputs copy their sum to each channel with square-root mode-count normalization. Call tick(excitation) for a voice graph or drive it with impulses/noise/other audio. It is not a stereo room simulator. Decay is exact for an isolated recurrence up to floating-point error; string decay includes additional brightness/interpolation losses.

## Bounds, timing and recovery

Both prepare at 8–192 kHz, mono/stereo, max block 8192; modes/string frequencies must be below 0.4× sample rate. String frequency is 20–2000 Hz, nominal decay 0.1–30 s, brightness 0–1, dispersion 0–0.95, bow pressure 0–1, velocity ±1. Pluck amplitude 0–1, pick position 0.02–0.98; seed zero maps to one. Modal frequency is 20–20000 Hz, decay 0.01–30 s, gain ±1. Nonfinite/out-of-range configs are rejected; a failed string update retains the previous controls. Calling pluck before prepare returns NotPrepared. Invalid/unprepared callback shapes are silenced without advancing state; invalid modal excitation returns silence for that sample. Input excitation is limited to ±1 and string feedback to ±2. These guards are not an output loudness limiter.

Both report zero algorithmic buffering latency. The physical loop and resonance buildup are sound behaviour, not host compensation delays. No automatic oversampling, polyphony, MIDI, acoustic-body calibration or bidirectional string coupling is included.

## Ownership, persistence and migration

Construct/prepare off the callback; mode coefficients and delay storage allocate there. Process/tick/pluck/set_config/reset are allocation-free under the measured fixtures. Pluck uses a bounded ~38 KiB temporary excitation copy; account for callback stack size when embedding. One audio owner mutates each voice; send changes through an application command queue. Reset clears all vibration and deterministic history. Persist StringConfig or ModalConfig plus application trigger/seed policy. Unknown fields/schema mismatches reject; delay/resonator state is not a preset. Earlier synth/sampler APIs are unchanged. Build replacements off-thread for modal topology changes and retain the prior voice on failure.

## Runnable example and validation

`cargo run -p soniclaw-dsp-suite --offline -- physical NEW_DIRECTORY` writes picked-string, friction-excited-string and inharmonic metallic modal WAVs with measured callback allocation counts. The modal demo is a processor driven by one impulse, not a source with stored samples. Unit fixtures check approximate 220 Hz string tuning, decaying energy, reset silence, invalid updates and modal −60 dB decay after 0.1 seconds. Release logs are in docs/validation/v0_95_0/. Audio examples demonstrate algorithms, not realism or listening acceptance; actual instrument design needs tuning, envelopes, bodies and regression listening.
