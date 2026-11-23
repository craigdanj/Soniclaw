# Modelled pedal/amplifier signal chains — 0.96.0

## Workflow and API

Use `soniclaw_dsp::amp_models::{Amp, Config, Controls}` as an AudioProcessor in a guitar/instrument chain. `Amp::new(Config::default())` prepares a 4× High oversampled circuit with 18 dB input drive, balanced tone, moderate sag and −6 dB output. `Amp::with_cabinet(config, impulse, rate, partition)` optionally appends the existing mono-IR Convolution independently to each channel at base rate. The application supplies an owned, licensed IR; no vendor capture, model weights or third-party assets are bundled. This generic nonlinear RC/diode model is not a replica of a named amp/pedal and does not load NAM/neural models.

The signal passes through supply-envelope-dependent gain, a bounded implicit capacitor/diode solve, DC removal, two-band tone filtering, output gain, then dry/wet blend. The cabinet follows that blend, so mix=0 still passes through the cabinet and oversampling converter. Use an external latency-aligned bypass if a true bypass is required. Channels have independent circuit/sag states; a mono cabinet is not true-stereo convolution.

## Solver and timing

The normalized circuit solves `y − state + g × (y − input + diode(y)) = 0`, where g is proportional to sample interval and a 6 kHz RC pole, and diode is an asymmetric pair of exponential currents. At most sixteen safeguarded Newton iterations keep a bracket of ±20; exponent arguments are bounded. It is a backward-Euler circuit-inspired model in normalized units, not measured electronics or a component-accurate SPICE replacement. An input follower with 50 ms smoothing reduces gain for a simple supply-sag effect. Tone blends a 250 Hz low band with a tone-dependent upper lowpass. A 20 Hz DC filter removes bias buildup.

Reported latency is oversampling conversion plus optional IR partition (96 frames at default 4× High; 160 with a 64-frame cabinet partition). Circuit time constants cause additional frequency-dependent phase, not a host compensation delay. Converter filtering changes even mix=0. There is no true-peak output limiter. Oversampling reduces some aliasing but does not make the exponential/nonlinear chain alias-free.

## Controls, ownership and persistence

Schema-1 Config contains Controls and existing schema-1 oversampling Config. Controls bounds: drive 0–36 dB, bias ±0.2, diode asymmetry 0.5–2, tone 0.05–0.95, sag 0–1, output −36–6 dB, wet mix 0–1. Unknown fields and invalid controls are rejected. `set_controls` validates before mutation and preserves capacitor/filter history. Targets are block-held; smooth application automation to avoid zipper noise. Oversampling/cabinet topology changes require a new prepared instance off-thread.

Prepare at 8–192 kHz mono/stereo within the converter's buffer limits. The high-rate circuit uses the converter's elevated rate; IR rate must match the base rate and partition must be a power of two 32–4096. Existing Convolution IR length/storage limits apply. Construction/prepare and IR destruction belong off the callback; rendering/reset/set_controls use prepared storage. Single audio owner, no locks or shared mutable state. Reset clears circuit, converter and cabinet history.

Nonfinite input is replaced with zero and finite input clamped to ±4 before drive; circuit input is bounded to ±20. Invalid/unprepared callback shapes are silenced. Failed configuration leaves current controls intact; failed prepare makes Amp unavailable until a successful prepare. Persist Config plus a separate cabinet asset reference/rate/partition. Runtime sag/filter/cabinet histories are not presets. Earlier Waveshaper/OversampledDrive APIs and product IDs remain unchanged.

## Example and validation

`cargo run -p soniclaw-dsp-suite --offline -- amp NEW_DIRECTORY` generates clean, asymmetric crunch and driven/sagged cabinet examples from an owned synthetic plucked signal, plus JSON latency/finite/zero-processing-allocation checks. The cabinet is a short synthetic decaying impulse, not a recorded speaker. Tests verify implicit equation residuals, symmetric/asymmetric response, partition equality, cabinet timing and reset behavior. See docs/validation/v0_96_0/. No amplifier realism, exact analog response, commercial tone match or unmeasured CPU advantage is implied; audition and profile in your product.
