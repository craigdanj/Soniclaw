# Reed and wind physical modelling

Construct `soniclaw::dsp::reed_waveguide::ReedVoice` with `Controls` and prepare
at the output rate. Set normalized breath pressure above the oscillation
threshold and call the AudioProcessor interface or `next_sample`. Setting
breath to zero allows the bore to decay. Bore length follows frequency;
length changes slew, so large changes can produce a glissando. A smoothed
breath command and the nonlinear movement of the reed set the attack rather
than a sampled or scripted note envelope.

The engine is a single-reed, cylindrical-bore, quarter-wave approximation.
The reflection at the open end inverts pressure; a low-pass feedback filter
represents frequency-dependent loss. Pressure-dependent reflection at the
reed can sustain oscillations or close the reed. Adjust reed slope,
damping, retention, and breath together. Some parameter combinations produce
only an initial puff or go quiet when the reed closes. This instrument
provides clarinet-style synthesis with no calibrated tone holes, register key,
or measured acoustic-body model.

```sh
cargo run -p soniclaw-dsp-suite -- reed-waveguide output/wind
```

The demo renders two reed settings, a pitch change, and a released breath.
For an instrument, map note pitch and pressure or a breath controller to
`set_controls`. Give each polyphonic voice its own prepared bore. Mono and
stereo use the same signal and there is no algorithmic buffering delay.

Prepare off-thread for 8–192 kHz and one or two channels. Processing and
setters use prepared memory. The frequency range is 65–1,400 Hz and normalized
breath is 0–1. Invalid updates leave controls intact; invalid callback shape
produces silence. Reset clears the bore and breath history. Save Controls
and recreate the instrument on load. Existing physical strings, modal banks
and saved state require no migration.

[Executed checks](validation/v0_128_0/REPORT.md).
