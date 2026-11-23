# Reusable DSP roadmap

Focus changed from Studio workflows to framework DSP on 2026-11-20. The first batch follows the previously suggested priorities 1, 2, 3 and 5. Each has its own minor source release, detailed guide, standalone example and validation record. No Studio GUI integration is required to use these APIs.

| Recommendation | Status | Release / guide |
|---|---|---|
| 2×/4×/8× reusable oversampling | Complete | [0.89.0](DSP_OVERSAMPLING.md) |
| Anti-aliased nonlinear processing | Complete | [0.90.0](DSP_NONLINEAR.md) |
| Advanced nonlinear/comb/formant filters | Complete | [0.91.0](DSP_FILTERS.md) |
| Stronger oscillators | Complete | [0.93.0](DSP_OSCILLATORS.md) |
| Compiled modulation and efficient control updates | Complete | [0.92.0](DSP_MODULATION.md) |
| Higher-quality pitch/time processing | Complete | [0.94.0](DSP_PITCH_QUALITY.md) |
| Physical modelling | Complete | [0.95.0](DSP_PHYSICAL.md) |
| Amp/pedal modelling | Complete | [0.96.0](DSP_AMP_MODELS.md) |
| Advanced convolution/reverb | Complete | [0.97.0](DSP_SPATIAL.md) |
| Advanced dynamics | Complete | [0.98.0](DSP_DYNAMICS.md) |
| Sampler DSP improvements | Complete | [0.99.0](DSP_PLAYBACK.md) |
| Broader DSP optimization | Complete | [0.100.0](DSP_OPTIMIZATION.md) |

These are extensions to existing facilities; a pending enhancement does not mean its entire underlying category is absent. Quality evidence covers the explicitly recorded fixtures, not universal sound quality, alias-free processing or worst-case real-time guarantees. Physical devices, third-party acceptance and unexecuted platform checks remain separate.

[Deferred Studio roadmap](DEFERRED_STUDIO_WORK.md). Its earlier proposed version numbers were plans and have been superseded by these DSP releases.
