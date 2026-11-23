# Cabinet IR alignment and blending

Use `cabinet_blend::estimate` on two mono, equal-rate cabinet responses.
The estimator searches for the strongest normalized correlation over the
first 8192 frames, including inverted polarity, then estimates a
fractional offset from neighboring lags. A positive advance moves a
late candidate earlier. Inspect correlation before applying a suggestion
to two different cabinets. Strong resonances can yield ambiguous lags.

Pass the measured or manual advance and a polarity switch into `blend`.
Set independent linear gains, balance, and requested frame count.
The output uses the reference origin, with missing time filled by zero.
A fractional shift uses the existing windowed-sinc interpolator.
Pad or reposition the inputs first if important candidate content would
land before the reference origin. This aligns one bulk delay, preserving
the underlying responses and their remaining phase differences.

```sh
cargo run -p soniclaw-dsp-suite -- cabinet-blend synthetic-cab
cargo run -p soniclaw-dsp-suite -- cabinet-blend measured-cab close.wav edge.wav
```

The fixture recovers deliberately introduced delay and polarity reversal.
It also renders an original synthetic plucked signal through the previous
release's WaveNet processor and blended cabinet convolution. The file mode
writes a float WAV with the original rate and a JSON alignment report.

Both functions run offline. Each IR can contain up to 1,048,576 frames,
with a maximum configurable offset of 4096 samples. Malformed or silent
measurements return an error; keep the previous IR. Do inference and
convolution with their existing prepared processors. Persist the source
assets and blend settings or store the resulting WAV. Earlier capture
and convolution projects require no migration.

[Executed checks](validation/v0_133_0/REPORT.md).
