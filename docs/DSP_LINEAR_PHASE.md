# Linear-phase EQ and crossover

Build Crossover with a sample rate, channel count, two ordered cutoffs, transition width and Kaiser attenuation estimate. Each call to split fills three interleaved outputs. The low band uses the lower low-pass, the middle band subtracts the lower response from the upper one, and the high band subtracts the upper response from a pure delay. Their sum matches the input at the reported group delay to floating-point precision.

Wrap a Crossover in Equalizer to apply three nonnegative linear gains on an AudioProcessor path. Configure gains on the audio owner. Smooth changes in the app if automation clicks matter. Design/allocate off the callback. Processing and reset do not allocate. The direct two-kernel runtime is best for short or moderate tap counts. Long linear-phase filters add cost, delay and transient pre-ringing. No extra delay is added beyond the shared FIR group delay.

Persist the crossover recipe and gains. Regenerate coefficients at a new sample rate. Earlier multiband presets are unchanged. Inherits the FIR designer's bounds and requires edge separation at least the transition width. Invalid split lengths reject without changing history. Invalid prepare or gain edits preserve prior settings. Reset clears all histories. The fixture verifies stereo impulse recombination and the example measures a three-tone reconstruction before rendering EQ audio.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- linear-phase NEW_DIRECTORY`. [Executed checks](validation/v0_105_0/REPORT.md).
