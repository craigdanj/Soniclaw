# Modulatable fractional delay

Create a DelayLine with channel count, maximum and initial delay in samples, a slew limit in samples per frame, and a linear or cubic interpolator. Use tick with one input and output frame. It moves the main tap towards its target and writes the incoming frame exactly once. tap reads extra positions independently before tick. The latest completed frame is at age one.

Use the same audio owner for set_delay, tick and reset. Only construction allocates. A bounded slew controls changes in read-head speed and can produce audible pitch movement. For a transparent large jump, crossfade two independently read taps in the application. Cubic interpolation can overshoot and should be gain-budgeted in feedback. A delay of 2–max leaves enough causal history for both interpolators.

Save requested delay, slew and interpolation choice in app state. Recalculate sample counts at another rate. Earlier effect preset schemas are unchanged. Bounds are 1–64 channels, maximum 3–262144 samples, up to 4194304 stored samples, and slew 0.00001–1. Invalid frames do not advance the line. Reset clears history and jumps to the accepted target. The example modulates a cubic tap at audio rate and measures callback allocations. The regression fixture checks channel isolation, integer timing and slew.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- fractional-delay NEW_DIRECTORY`. [Executed checks](validation/v0_102_0/REPORT.md).
