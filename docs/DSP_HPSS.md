# Harmonic and percussive separation

Call `hpss::separate` on mono or stereo float samples. The offline process
compares across-time median power with across-frequency median power.
A shared mask derived from both stereo channels scales the original
STFT, then normalized overlap-add reconstructs the tonal part. The
percussive part is the complementary output. Both stems retain the
original sample timeline and their sum reconstructs the input to float
precision.

```sh
cargo run -p soniclaw-dsp-suite -- hpss generated-stems
cargo run -p soniclaw-dsp-suite -- hpss loop-stems drum-loop.wav
```

The example writes original, harmonic and percussive float WAVs with
a recombination check. Tune the FFT and median radii for longer-lived
harmonics or short attacks. Instruments with both properties can land
in both stems. For long inputs, process overlapping chunks and discard
their edges to limit boundaries from affecting the result.

Run on a worker or control thread. A run accepts one or two channels,
up to 8,388,608 source frames with a separate 8,388,608 spectral-cell
budget, 256–4096-point windows and radii of 1–16 bins or hops.
Invalid bounds or PCM produce an error before emitting stems. If the
spectral budget rejects a run, use larger hops or smaller chunks.
Persist analysis settings and asset references or export the WAVs.
The earlier pitch-guided harmonic/residual module remains available.

[Executed checks](validation/v0_134_0/REPORT.md).
