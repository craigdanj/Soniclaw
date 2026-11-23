# Soniclaw 0.23.0 validation

Fresh results: [0.23.0 report](validation/v023/REPORT.md).

---

# Soniclaw 0.22.0 validation

Fresh results: [0.22.0 report](validation/v022/REPORT.md).

---

# Soniclaw 0.21.0 validation

Fresh results: [0.21.0 report](validation/v021/REPORT.md).

---

# Soniclaw 0.20.0 validation

Fresh results: [0.20.0 report](validation/v020/REPORT.md).

---

# Soniclaw 0.19.0 validation

Fresh results: [0.19.0 report](validation/v019/REPORT.md).

---

# Soniclaw 0.18.0 validation

Fresh results: [0.18.0 report](validation/v018/REPORT.md).

---

# Soniclaw 0.17.0 validation

Fresh results: [0.17.0 report](validation/v017/REPORT.md).

---

# Soniclaw 0.16.0 validation

Fresh results: [0.16.0 report](validation/v016/REPORT.md).

---

# Soniclaw 0.15.0 validation

Fresh results: [0.15.0 report](validation/v015/REPORT.md).

---

# Soniclaw 0.14.0 validation

Fresh results: [0.14.0 report](validation/v014/REPORT.md).

---

# Soniclaw 0.13.0 validation

Fresh results: [0.13.0 report](validation/v013/REPORT.md).

---

# Soniclaw 0.12.0 validation

Fresh results: [0.12.0 report](validation/v012/REPORT.md).

---

# Soniclaw 0.11.0 validation

Fresh results: [0.11.0 report](validation/v011/REPORT.md).

---

# Soniclaw 0.10.0 validation

Fresh results: [0.10.0 report](validation/v010/REPORT.md).

---

# Soniclaw v0.9.0 validation

Fresh registry, managed-output policy, source/ABI, DSP and native panel checks are recorded in the [v0.9.0 report](validation/v090/REPORT.md). Physical-device and real DAW acceptance remain pending. Older evidence below retains its original scope.

---

# Soniclaw v0.8.0 validation

Fresh project workflow, source/ABI, prepared DSP and native virtual-display results are recorded in the [v0.8.0 report](validation/v080/REPORT.md). Older evidence retains its original scope.

---

# Soniclaw v0.7.0 validation

Fresh results and limits are recorded in the [v0.7.0 report](validation/v070/REPORT.md). Earlier records below retain their original scope.

---

# Soniclaw v0.6.0 validation

Current source/ABI, prepared DSP, native virtual-display and CPU fixture checks
are recorded in the [v0.6.0 report](validation/v060/REPORT.md). Earlier records
below retain their original scope. Physical audio devices, real DAW GUI sessions
and macOS/Windows runtime acceptance remain pending.

---

# Soniclaw v0.5.0 validation

Current Linux DSP/source/ABI verification is recorded in the
[v0.5.0 report](validation/v050/REPORT.md). Earlier reports below remain
historical evidence and are not fresh v0.5 native format or performance results.

---

# Soniclaw v0.4.0 validation

Current Linux source, state/ABI, DSP/spectral, disk-streaming and allocation checks
are recorded in the [v0.4.0 report](validation/v040/REPORT.md). The report separates
completed checks from native-wrapper, GUI, DAW and physical-device work still pending.

The earlier rename and v0.3 validation reports below are preserved historical
records. Their native format and performance measurements are not new v0.4 results.

---

# Soniclaw rename verification — 2026-11-16

The renamed workspace passed 55 Rust tests, Clippy, rustdoc, CLI template checks and exported CLAP demo smoke checks. See the [rename verification report](validation/rename-20261116/REPORT.md) for commands and limits.

The original validation report below is preserved unchanged as historical evidence for the pre-rename Sonora v0.3.0 release. Its native format, editor and benchmark results were not rerun as part of the rename.

---

# Sonora v0.3.0 validation — Linux

Executed on 2026-11-15 in a shared Debian 13 x86-64 environment reporting AMD EPYC 7763 and nine available CPUs. Rust 1.100.0/LLVM 23.1.5 and GCC 14.2.0. No physical audio device or DAW session was used.

## Completed checks

- Formatting, Clippy with warnings denied, **55 Rust unit/integration/doc tests**, and rustdoc with warnings denied passed.
- All three generated app/effect/sampler projects built, including a parent directory containing spaces.
- Original gain/synth exported-ABI smoke tests passed. The native reference drum factory rendered a five-second stereo 48 kHz WAV without audio hardware.
- Upstream CLAP validator: **183 passed, 52 skipped, 1 warning, 0 failed/crashed** across the original and reference libraries. No tests were excluded.
- Steinberg VST3 validator: **188 passed, 0 failed**, across all four reference products.
- Independent host: real sidechain ducking/release, in-place and constant buffers, timed gain, active/partial-stream state restoration, malformed-state rejection, variable/empty blocks, sample-rate reactivation and allocation interposer self-test passed.
- **Zero heap allocations and frees across 9,791,002 timed v0.3 callback executions** (360 batch rows). The frozen v0.2 baseline was measured with the same final host binary.
- All four embedded X11 editors rendered and accepted XTest knob drags. Begin/value/end gestures remained balanced with only one host output event accepted per callback. Restoring state while a GUI value was pending canceled the stale edit and completed the gesture. Resize/scale, hide/show and teardown passed. GUI checks were separate from primary timing.
- The pinned VST3 build script completed. The archive and Debian package helper produced a .deb whose control metadata/structure was read successfully; it was not installed. Collected dependency notices were included in the packaging test.

## 256-frame callback CPU comparison

| Workload | v0.2 µs | v0.3 µs | Speedup |
|---|---:|---:|---:|
| Pass-through | 0.752 | 0.556 | 1.35× |
| Static gain | 1.215 | 0.607 | 2.00× |
| Low-pass + gain | 2.977 | 2.669 | 1.12× |
| Automated gain | 1.244 | 1.104 | 1.13× |
| Automated cutoff | 3.270 | 2.765 | 1.18× |

Full 64–1024-frame measurements and nine-round ranges are in `benchmarks/results/`. These are thread CPU costs with the editor closed, not hardware latency, whole-DAW utilization, a fresh JUCE comparison, or a language-wide conclusion. Shared virtualization, sequential version runs and clock/frequency variation limit small differences.

## Denormal diagnostic

One upstream denormal timing warning remains for the original gain plug-in. The upstream test times random input generation and host work outside the callback as well as processing. The CLAP callback now scopes FTZ/DAZ and restores the host control mode.
The separate callback-only probes generated all input before timing: denormal/normal median CPU ratios were **0.91 for gain** and **0.90 for the reference gain mode**. Both verified zero gain output for subnormal input and unchanged host control bits afterward. The slowdown was not reproduced in this isolated callback measurement; outside-callback work and timing variability are plausible contributors to the upstream warning. The original warning is retained in the JSON report.

## Outstanding target-machine acceptance

Native macOS/Windows builds and runtime tests, AUv2 compilation/auval, real DAW project scan/save/reopen/automation sessions, physical duplex loopback/unplug/replug and actual disk-full recovery remain unverified. CI, AU and installer recipes are supplied as code, not proof that those targets ran. See `docs/COMPATIBILITY.md`.

The multisampler preloads its PCM. The separate disk player is forward-only, same-rate transport, not seekable per-voice sample streaming. The effects toolkit has functional/response/allocation checks, not mastering-grade acoustic certification.
