# Advanced creative filters — 0.91.0

## Workflow and public API

`soniclaw_dsp::creative_filters` adds `NonlinearSvf`, `CombFilter` and `FormantFilter` as standalone `AudioProcessor` implementations. Use them directly in instruments/effects or in the reusable oversampling wrapper; no Studio or device dependency is involved. Existing SVF/ladder/reverb implementations retain their sound and interfaces.

Construct with the appropriate serde configuration, prepare a `ProcessSpec`, then render interleaved buffers. `set(config)` validates all fields before updating targets, without allocating. Call it on the audio owner or deliver application commands there. Persist configuration and create/prepare a fresh processor after reopening. Schema 1 rejects unknown fields; histories are not part of presets.

Run `cargo run -p soniclaw-dsp-filters --offline -- NEW_DIRECTORY`. The example renders all three processors and a 4× oversampled nonlinear SVF, writes WAV files, checks finite output and records callback allocations/latency. It also verifies reset and partition behavior through the unit tests.

## Nonlinear state-variable filter

`NonlinearConfig` selects low-pass, band-pass, high-pass or notch, cutoff 10–20000 Hz, resonance 0–0.98 and drive 0–16. Actual cutoff is limited to 0.45 of the prepared sample rate. Drive zero takes the analytical linear path. Otherwise the input and damping feedback use tanh. A monotonic one-dimensional equation is solved using six bounded-cost Newton iterations per channel/sample:

`(1+g²)b + g*k*tanh(d*b)/d = s1 + g*(x-s2)`.

Here `g=tan(pi*cutoff/rate)`, `k=2*(1-resonance)+0.02`, and `s1/s2` are trapezoidal-integrator states. Internal states and output have ±16 safety bounds and denormal cleanup. This is a creative nonlinear TPT filter with its own sound, not a fitted model of a particular analog circuit. Nonlinearities can alias at base rate; compose with oversampling when appropriate. Changing cutoff abruptly can still be audible; feed smoothed controls for sweeps.

## Tunable damped comb

`CombConfig` provides 20–20000 Hz tuning, signed feedback −0.98–0.98, damping 0–20000 Hz and wet mix 0–1. Delay reads use linear fractional interpolation; the positive interpolation weights and bounded feedback avoid the gain overshoot associated with some higher-order interpolation filters. Zero damping Hz explicitly disables the damping filter. The ring stores input plus feedback and has a ±16 safety bound.

Frequency changes use a 10 ms pole on delay length, producing pitch glide rather than instantaneous retuning. The delay is at least two samples. Fractional interpolation and damping alter high-frequency decay/tuning; this is not a lossless physical string model. The creative delay is part of the effect, so the processor reports zero scheduling latency rather than pretending that frequency-dependent group delay is a fixed compensation amount.

## Morphing formant bank

`FormantConfig.vowel` moves continuously across five illustrative vowel-like settings: A/E/I/O/U at positions 0–4. A ±24-semitone shift transposes the resonances. Three parallel TPT band-pass filters use center tables `[800,1150,2900]`, `[400,1700,2600]`, `[350,2000,2800]`, `[450,800,2830]`, `[325,700,2530]` Hz. These are creative presets, not a speech-production model or universal human-vowel measurements.

Prepared target coefficients interpolate with a 20 ms smoothing pole; no tan/pow is evaluated per audio sample. Centers are limited to 0.4 of sample rate, damping stays positive and states/output have safety bounds. Mix is 0–1. This filter also reports zero fixed scheduling latency.

## Lifecycle, limits and recovery

All three allocate in construction/prepare; audio processing and set/reset allocate nothing. One audio thread owns each instance. Core rate/channel limits permit high-rate composition. Comb memory is capped at 4,194,304 ring samples and all size arithmetic is checked before allocation; a large high-rate/multichannel configuration may therefore be rejected. Ring capacity is based on the 20 Hz minimum.

Wrong channel counts, oversized blocks and nonfinite input are silenced before histories advance. Invalid configuration returns an error without changing targets. Reset clears histories and snaps smoothing to the prepared targets. Changing rate/channels requires preparation off the callback. Gain limiting is an intentional protection in extreme creative settings; it is not transparent limiting suitable for mastering.

## Validation

`docs/validation/v0_91_0/` records executed checks. Fixtures cover tuned comb impulse repeats/feedback decay, exact callback partition equivalence, driven high-resonance cutoff sweeps, vowel morphs, zero allocations, finite output and invalid-control rejection. The audio example exposes the produced sounds for listening review. Broad subjective sound-quality, hardware, external DAW and Windows/macOS runtime acceptance remain pending.

## 0.92.0 drive-limit refinement

Nonlinear SVF input saturation is normalized by tanh(drive), making very small positive drive approach the drive-zero linear path continuously. The feedback equation and protection bounds are unchanged. A continuity fixture checks this limit.
