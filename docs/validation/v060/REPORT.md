# Soniclaw v0.6.0 verification — 2026-11-16

Linux x86-64, Debian 13/glibc 2.41, Rust 1.100.0. Temporary rootless sysroot
headers/libraries supplied native dependencies. No physical audio device or
real DAW was used. [checks.json](checks.json) records checks and refreshed logs;
[native checks](native-gui-checks.json) records actual capture invocations.

## Completed

- **111 unit/integration/doc tests passed**, zero failures. Final locked/offline
  workspace all-feature tests include the granular zero-velocity note-ID edge fix.
- Workspace formatting and whole-workspace/all-target/all-feature Clippy pass.
  Clippy and all-feature rustdoc used warnings denied; final docs used
  `RUSTDOCFLAGS="-D warnings"`. Default-feature native Workbench builds.
- New prepared allocation fixtures execute all six effect wrappers, fixed-pool
  grains, modulation, repeated long-IR swaps and retirement. Zero allocations and
  zero net allocation change in the measured callback sections; control-side
  retirement is explicitly outside those sections. This is fixture evidence,
  not a proof for arbitrary processors, drivers or operating systems.
- DSP tests cover base/macro separation and bounded routes, synced LFO/MIDI,
  envelope release, neutral multiband recombination/solo, dynamic-EQ tone
  attenuation, deterministic grain block partitioning/pool exhaustion/sustain/
  release, pitch/time duration and tone transposition, Hilbert translation,
  freeze after silence, stereo convolution vs direct reference at both latency
  modes, uniform alignment and crossfade/control retirement behavior.
- State tests retain parameter identity/base values, reject missing modulation
  targets without mutation, clear transient offsets after stopped reconstruction,
  and preserve old state/automation/graph tests. Workbench model tests exercise
  edits, undo, portable bundle reload and finite/audible output from all presets.
- Independent dynamic CLAP loading passes for the existing Gain/Synth, Project
  Rack, DSP Lab FX/Wavetable and Workbench Multiband/Granular products. New
  Workbench exports declare 252 frames of latency at 48 kHz and exercise
  parameter discovery, audio/native note input, complete schema-2 state, partial
  streams and stopped reconstruction. This is an independent small host, not a DAW.
- App, CLAP and sampler templates pass default-feature Cargo checks under a parent
  path containing spaces. Earlier DSP Lab/ecosystem walkthroughs pass again.
- New headless walkthrough renders four project presets with copied WAV assets,
  an IR-crossfade (2048-frame fade, zero latency, one engine retired on control),
  +7-semitone pitch and 1.5× time stretch. Resource-backed presets were constructed
  against the copied render folder. [Audio measurements](rendered-audio.json)
  record the seven generated WAV files. The preview report uses a debug build;
  its CPU values must not be treated as optimized production measurements.
- Three actual native eframe windows rendered under Xvfb with software OpenGL:
  [controls](workbench-controls.png), [modulation](workbench-modulation.png), and
  [granular preset](workbench-granular.png). Screenshots were visually inspected.
  Startup uses a worker preview and opens no audio device. Model interaction tests
  are separate from visual rendering; physical mouse/keyboard/driver/DAW editor
  acceptance is still pending.
- Prior 0.1–0.5 release notes, frozen baseline, historical benchmark and validation
  files remain byte-identical. Source hashes/ZIP integrity and active local docs
  links were verified while packaging.

## Optimized CPU fixtures vs 0.5

[Raw rounds](cpu/callback-fixtures.csv), [method/environment](cpu/summary.json),
build logs and baseline archive SHA accompany `tools/benchmark-dsp.py`. Both
binaries finish compiling before measurement; seven rounds alternate execution
order. Static controls, 48 kHz stereo, 256-frame blocks, 256 warmup/4096 timed
blocks, generic x86-64 target, thin LTO, one codegen unit. Gain is set to 0.5.
Times include buffer copy, wrapper and black-box consumption. Lower is faster.

| Fixture | v0.5 ns/stereo frame | v0.6 ns/stereo frame | Change |
|---|---:|---:|---:|
| fdn_reverb | 153.901 | 142.773 | -7.23% |
| gain | 2.502 | 0.146 | -94.18% |
| low_pass | 7.905 | 7.921 | +0.20% |
| rms_compressor | 26.423 | 11.625 | -56.00% |
| state_variable | 10.520 | 9.919 | -5.71% |

Gain/RMS benefit from settled-block processing; LowPass is essentially flat in
this run. These medians on a shared, unpinned VM are small-fixture measurements,
not universal speedups, a regression gate, dense-modulation results, whole-rack
CPU, DAW/driver latency or a fresh JUCE comparison. Keep the raw variability and
scope when quoting them. New spectral/granular/IR costs need target-hardware work.

## Pending and limits

No physical-device loopback, default-device playback audition, real DAW scan/
reopen/automation, embedded editor interaction, macOS/Windows runtime, fresh
VST3/AUv2 adapter builds/validators, independent acoustic/compliance vectors,
installer/signing/notarization or remote CI execution was performed here.
Linux virtual-display rendering is not evidence for those scenarios.

The [module guide](../../DSP_MODULATION.md), [workbench guide](../../WORKBENCH.md),
[migration](../../MIGRATION_060.md) and [release notes](../../releases/v0.6.0.md)
state algorithmic/resource limitations. All sample assets are synthesized.
