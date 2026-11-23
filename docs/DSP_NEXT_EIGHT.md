# Reusable DSP additions: 0.93–0.100

These eight modules extend the framework independently of Studio. Each minor release has its own guide, release note and executed validation folder. The original oversampling, waveshaper, filter and compiled modulation additions remain available; see [DSP roadmap](DSP_ROADMAP.md).

| Release | Reusable API | Example command | Detailed guide |
|---|---|---|---|
| 0.93.0 | `oscillators::Oscillator` | `oscillators` | [Modulated source engines](DSP_OSCILLATORS.md) |
| 0.94.0 | `streaming::PitchShifter::set_formants`, `pitch_quality::Quality` | `pitch` | [Pitch envelope and quality controls](DSP_PITCH_QUALITY.md) |
| 0.95.0 | `physical::{WaveguideString, ModalBank}` | `physical` | [Physical voice building blocks](DSP_PHYSICAL.md) |
| 0.96.0 | `amp_models::Amp` | `amp` | [Generic circuit-inspired chains](DSP_AMP_MODELS.md) |
| 0.97.0 | `spatial::{MatrixConvolution, HybridReverb}` | `spatial` | [Four-path IR and hybrid room processing](DSP_SPATIAL.md) |
| 0.98.0 | `dynamics::Dynamics` | `dynamics` | [Linked and external-key dynamics](DSP_DYNAMICS.md) |
| 0.99.0 | `sample_playback::RegionPlayer` | `playback` | [Advanced resident region playback](DSP_PLAYBACK.md) |
| 0.100.0 | `prepared_resample::PreparedSinc` | `optimization` | [Prepared coefficient kernels](DSP_OPTIMIZATION.md) |

All API paths above are under `soniclaw_dsp`; the facade also exposes this crate as `soniclaw::dsp`. Fetch pinned dependencies with `cargo fetch --locked`, then render a selected command with:

```sh
cargo run -p soniclaw-dsp-suite --locked --offline -- oscillators NEW_DIRECTORY
cargo run --release -p soniclaw-dsp-suite --locked --offline -- optimization ANOTHER_NEW_DIRECTORY
```

Each output directory must be new. It contains float WAVs and a JSON report. The suite checks finite/nonzero output and measured callback allocations; the optimized interpolation comparison records phase agreement, memory/backend and local elapsed times. Construction, source generation, WAV I/O and measurement reporting run outside callbacks. No audio hardware or GUI is needed. Platform development libraries are still required to build the entire workspace; consult the root README for desktop setup.

The downloadable audio collection includes an offline listening page, PCM16 previews and unmodified original WAVs. Previews only attenuate peaks above 0.9, and their exact gains are reported; the DSP processors themselves are not automatically normalized or limited. Earlier 0.89–0.92 original renders are retained alongside fresh 0.100.0 renders of the eight commands. The [final validation](validation/v0_100_0/REPORT.md) and [all-example report](validation/v0_100_0/all-dsp-examples.json) record executed checks.

## Integration contracts

Prepare and own one mutable processor per voice/signal path. Use application command queues for controls and retire replaced processors/assets off the audio callback. Explicitly compensate reported converter, pitch, lookahead and convolution delays. Most runtime controls are block-held; smoothing, click-free topology changes and polyphonic allocation remain host/instrument responsibilities unless an API explicitly provides them. Each guide specifies schema, persistence, bounds and recovery rather than silently changing old presets.

Envelope/physical/circuit models are approximations with synthetic evidence, not commercial product emulations. Prepared sinc is opt-in: non-band rates use conservative 1/2/4/8 cutoff banks and can lose bandwidth or change timbre across a band boundary. Choose the existing exact sinc path for continuous pitch sweeps where that tradeoff is unsuitable. Human listening, real hardware stress, third-party DAW acceptance and unexecuted platforms are separate validation work.
