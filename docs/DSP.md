# Effects toolkit

All processors implement `AudioProcessor`. Allocate and validate in `prepare`, process in place, and clear history with `reset`. Configuration constructors/setters are control-thread operations under exclusive ownership; this release does not provide automated controls for every effect setting. The reference rack demonstrates a prepared chain, with an automated output gain.

| Processor | Algorithm and limits |
|---|---|
| `Gain` | 10 ms linear ramp; settled scalar multiplication and unity/mute specializations |
| `LowPass` | RBJ biquad; cutoff/Q coefficients cached until changed; denormal state cleanup |
| `ParametricEq` | Up to 32 RBJ low/high-pass, peaking or shelving bands; per-channel state |
| `Compressor` | Linked peak detector, soft knee, attack/release and makeup gain |
| `PeakLimiter` | Instant-attack, linked sample-peak ceiling with release; no lookahead or true-peak guarantee |
| `NoiseGate` | Linked threshold, attack, hold and release |
| `Chorus` | Fractional delay, 15 ms base, up to 12 ms modulation depth, stereo LFO offset |
| `Reverb` | Four damped combs and two allpasses per channel; configurable nominal RT60 and mix |
| `Convolution` | Uniform FFT partitions; mono IR independently applied to every channel; exact partition-size latency |
| `OversampledDrive` | 2× soft clipping using two 63-tap windowed-sinc filters; 31 base-rate frames of latency |

```rust
use soniclaw::prelude::*;
use soniclaw::dsp::{Compressor, CompressorSettings, EqBand, EqShape, ParametricEq, PeakLimiter};

let mut engine = Engine::default();
engine.push(ParametricEq::new(vec![EqBand {
    shape: EqShape::Peak, frequency: 1200.0, q: 0.8, gain_db: 3.0,
}])?);
engine.push(Compressor::new(CompressorSettings::default())?);
engine.push(PeakLimiter::new(-1.0, 100.0)?);
```

For cabinet IRs, load WAV data off-thread, convert a stereo IR to mono explicitly if needed, then construct `Convolution::new(ir, sample_rate, 128)`. Source IR rate must equal processing rate. Partition sizes are powers of two from 32 to 4096; IRs are bounded to 1,048,576 samples and frequency-domain storage is limited to 512 MiB. Use the existing graph compensation when mixing convolution or oversampled drive with dry/parallel paths. The drive's internal dry/wet paths are already aligned.

Tests compare FFT convolution with direct convolution across multiple partitions and arbitrary callback lengths; check EQ center-frequency gain, compressor ratio, limiter ceiling, chorus delay, reverb reset/tail, drive impulse latency, and allocation-free processing. This is an initial effects toolkit, not a claim of mastering-grade quality or exhaustive acoustic evaluation.
