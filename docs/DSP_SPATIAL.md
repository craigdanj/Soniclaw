# True-stereo convolution and hybrid reverb — 0.97.0

## Workflow and API

`soniclaw_dsp::spatial::MatrixConvolution::new([ll, lr, rl, rr], rate, partition, mix)` accepts four mono IRs in output/input order: left←left, left←right, right←left, right←right. It models cross-channel energy; a conventional two-channel IR with independent left/right processing cannot supply those cross paths. Supply four compatible owned/licensed responses, or use zero IRs for missing paths. All paths share uniform partition size and latency; differing IR lengths are allowed. Existing StereoConvolution zero-latency/two-tier and reloadable APIs remain available separately.

`HybridReverb::new(HybridConfig, [ll, lr, rl, rr], rate, partition)` combines the four-path early response with an independently excited eight-line FDN tail. `early_balance` is a linear blend between the two wet paths, then `mix` blends that sum with aligned dry. Both FDN output and dry are delayed by one partition to align with the early convolution. The FDN has its own predelay/reflection timing in addition to this compensation. Feeding early and late in parallel avoids serial coloration but does not automatically derive a matching room from an IR.

Construct/prepare off-thread, then process stereo blocks. `set_mix` or `HybridReverb::set_config` validates before updating the audio-owned state. Setters are block-held; sudden wet/tail/timing changes can click. For an IR or partition change, prepare a replacement off-thread and crossfade externally using the existing reloadable infrastructure or a host processor swap. No background worker, automatic IR capture, convolution morphing or room fitting is added.

## Bounds and timing

Stereo only, 8–192 kHz, max block 8192. IR rate must match processing rate. Partition power-of-two 32–4096; total IR samples across four paths ≤1,048,576, every IR nonempty, finite and absolute value ≤1000. This bounds the four-path memory demand in addition to each existing Convolution storage guard. Matrix latency is exactly one partition; mix=0 still delays dry. No automatic IR normalization is performed; cross paths can increase gain. Matrix finite input is limited to ±16 and nonfinite input to zero. Use an output safety stage when experimenting with IR gains.

Hybrid schema-1 settings: mix and early_balance 0–1, decay 0.1–15 s, damping 500–16000 Hz, predelay 0–200 ms, diffusion and modulation 0–1. Unknown fields/schema mismatch/nonfinite settings reject. Invalid/unprepared callback dimensions silence without state advancement. On prepare failure the new instance is unavailable; retain the previous valid processor at the application level. Reset clears early FFT history, late network and compensation delays.

## Lifecycle, persistence and migration

Four convolution plans/kernels and all scratch/delay buffers allocate during construction/prepare. Process/setters/reset use prepared storage; single audio owner, no callback locks or worker synchronization. Do not release large IR owners in a callback. Persist HybridConfig plus four asset references, IR rate and partition; convolution runtime history is not serialized. Matrix construction arguments are explicit; the application versions its asset mapping. Old two-channel IR presets do not silently become four-path IRs: migrate with `[left, zero, zero, right]` for the same independent-channel semantics. Studio is not required.

## Example and executed checks

`cargo run -p soniclaw-dsp-suite --offline -- spatial NEW_DIRECTORY` writes true-stereo early reflections and a hybrid tail from small synthetic IRs, with JSON latency, peak, finite samples and callback-allocation counts. These are owned fixtures, not recorded acoustic spaces. Tests route separate L/R impulses through all four paths at the declared partition latency, compare arbitrary callback partitions, check aligned dry-only hybrid and a finite nonzero FDN tail, and measure zero processing allocations. See docs/validation/v0_97_0/. No perceptual room realism, universal reverb quality or cross-platform host acceptance is claimed.
