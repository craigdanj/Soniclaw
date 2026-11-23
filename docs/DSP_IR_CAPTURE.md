# Impulse-response capture

Generate a mono exponential sweep with `soniclaw::dsp::ir_capture::Sweep`.
Play it through the system under measurement, record the output, then pass
the original sweep and recording to `recover`. Use the same sample rate and
clock, keep the signal below clipping, and continue recording through the full
decay. Include enough leading offset in the search range to find the direct
arrival. Longer sweeps and quiet recordings improve the usable dynamic range.

```sh
cargo run -p soniclaw-dsp-suite -- ir-capture output/fixture
cargo run -p soniclaw-dsp-suite -- ir-capture output/source --sweep-only
cargo run -p soniclaw-dsp-suite -- ir-capture output/ir --recover output/source/sweep.wav capture.wav 32768 8192
```

The last two numbers are the requested IR length and the delay search span,
both in samples. Use zero for the delay span to keep the recording time origin.
The CLI accepts mono PCM/float WAV and writes a 32-bit float WAV at the input
rate. Feed its samples and rate to the existing `Convolution`. Capture each
input/output path individually for a stereo matrix. The fixture generates a
known delayed two-tap recording and feeds the recovered IR to the convolver.

`CaptureOptions` controls the IR length, peak-search span, retained lead-in,
tail fade, and regularization floor. `Capture` returns the strongest measured
offset and the window's actual start. Peak search can lock onto a louder echo.
For known timing preserve the origin and trim with your measured offset.
Regularization suppresses noise where the reference has little energy. The
result represents the excited band and the linear response of the system.
A sweep can also expose clipping and nonlinear artifacts in a recording.

Generation supports 8–192 kHz and frequencies strictly below Nyquist.
Recovery uses at most 4,194,304 FFT samples, rejects malformed layouts and
silent measurements, and preserves the measured gain. Adjust the search and
window or rerecord if capture is invalid. Run generation, recovery, and I/O
off the audio thread. Persist the exported WAV, sample rate, and capture
settings if reproducibility matters. Existing convolver APIs and DAW sessions
require no migration.

[Executed checks](validation/v0_122_0/REPORT.md).
