# NAM WaveNet playback

Use `soniclaw::dsp::nam_wavenet::NamWaveNet::from_json` on a control thread.
Check `sample_rate()` before preparing and use a separate resampler around
the model if the application's rate differs. The engine is mono and preserves
continuous dilated-convolution histories across callback boundaries. Put a
cabinet convolver downstream for amp-only models.

The supported classic WaveNet configuration uses 1–3 arrays, mono conditioning,
ungated Tanh layers, legacy head size/bias keys and no post-stack head.
Each array has 1–32 channels and 1–16 layers with tap counts up to 8 and
dilations up to 4096. Total receptive field is capped at 65,536 samples.
Input files are capped at 4 MiB. Common A1 variants using this configuration
are eligible. Gated, conditioned-DSP, slimmable and newer A2 schemas require
additional implementations. The final gain comes from the packed last weight.

```sh
cargo run -p soniclaw-dsp-suite -- nam-wavenet fixture-output
cargo run --release -p soniclaw-dsp-suite -- nam-wavenet capture-output capture.nam
```

The example renders at 48 kHz and saves a WAV, the validated model, paired
little-endian PCM binaries and a timed callback run. Use an optimized build
for performance measurements, and compare captures at the same rate, block
size and model size. The supplied fixture uses deterministic owned weights.

Preparation or reset clears history to zero. If a model benefits from
prewarming, feed silence before enabling it. Reset belongs to the single
audio owner, with cost proportional to the validated history size.
Invalid input samples become zero, invalid callback shape is silenced, and
malformed models fail before installation. Persist the model reference and
external input/output calibration. Existing LSTM presets remain compatible.

[Executed checks and upstream comparison](validation/v0_130_0/REPORT.md).
