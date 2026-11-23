# Loudness-targeted mastering renders

Call `soniclaw::dsp::mastering::render_master` with interleaved float PCM, source
sample rate and channels, and `Options`. The first pass measures integrated
K-weighted loudness and reconstructed peaks. The second calculates a bounded
gain, applies the existing lookahead limiter, flushes the tail, and removes
the lookahead delay. A measured overshoot triggers a small global peak trim.
It then measures the actual output and reports the input and output LUFS and
dBTP, gain adjustments, and whether the result is within 0.25 LU of the target
and below the measured ceiling.

```sh
cargo run -p soniclaw-dsp-suite -- mastering output/master-fixture
cargo run -p soniclaw-dsp-suite -- mastering output/song mix.wav -14 -1
```

The example writes a 32-bit float WAV at the original rate and a JSON report.
It is safe to use an output directory distinct from the source. Audition
limited material and rerender with a lower target or more crest-factor
headroom if required. A peak ceiling or boost cap can make requested
loudness unattainable. The measured report makes that visible. Peak values
are estimates based on the existing four-phase detector. When exporting to
another codec or sample rate, remeasure that version.

Call from a worker/control thread. Input is mono or stereo at 8–192 kHz.
The loudness meter analyzes non-48 kHz input via an internal resample.
Peak analysis and the exported audio remain at the original rate.
The library accepts up to 16,777,216 source frames and up to ten minutes.
It rejects nonfinite/invalid data and returns an explicit error for
material too short or quiet for an integrated measurement. Settings
accept −45 to −6 LUFS and a −20 to −0.1 dBTP ceiling. The output has the same
number of frames and channels as the input. Serialize Options and the
report with your offline render job or store them beside the WAV.
Existing online limiters and session state require no migration.

[Executed checks](validation/v0_129_0/REPORT.md).
