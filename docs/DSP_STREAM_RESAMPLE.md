# Continuous-ratio streaming resampling

Build immutable SincTable once off the audio thread and share it by Arc. Create a StreamResampler with channels, a bounded ring capacity, an initial source-frames-per-output-frame increment and a slew per output frame. For fixed-rate conversion use input_rate/output_rate. For varispeed use set_increment on the same audio owner. A phase step and a cutoff table blend change continuously as the accepted increment slews. The shared table is about 1.3 MiB.

push returns accepted input frames. Keep the unaccepted suffix and retry after pulling output. pull writes only the returned prefix. A zero return before finish signals insufficient lookahead. After the last accepted input call finish and pull until is_drained. Start and finish use zero padding and the end of output is the first position at or beyond the input length. Use reset for a new stream. There is 40-source-frame lookahead without block-quantized buffering. Do not destroy the last table reference on the audio thread.

Limits are 1–64 channels, 128–65536 frames of ring capacity, at most 2097152 stored samples, and increments 0.25–4. Malformed or nonfinite pushes reject before mutation. A full ring is recoverable by pulling. Persist source/output rate and slew in application state rather than ring position or samples. Existing sampler state and PreparedSinc presets are unchanged. A finite window entails passband and stopband tradeoffs, particularly near 4×. The example writes a varispeed sine file and checks render allocations. Tests verify buffered DC, flush length and input partition invariance.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- stream-resample NEW_DIRECTORY`. [Executed checks](validation/v0_104_0/REPORT.md).
