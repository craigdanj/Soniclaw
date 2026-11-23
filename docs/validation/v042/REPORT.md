# Soniclaw 0.42.0 verification

Executed 2026-11-18 on Linux x86_64. 2 selected Rust tests passed. Formatting, workspace/all-target/all-feature Clippy and documentation passed with warnings denied. [Exact commands and timings](checks.json).

Measurement calibration tests cover known harmonic/noise levels, Parseval normalization, aliases, thresholds and malformed input. The quality-suite example executes twelve real/reference DSP probes; an intentionally failing threshold run is recorded separately. Hosted CI and hardware measurement were not executed.

Physical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.

[Feature guide](../../DSP_QUALITY.md).

## Executed quality suite

All twelve default checks passed at 48 kHz / 8192-point coherent FFT. An intentionally tightened threshold produced a failed JSON check and exit code 1. [Measurements](measurements.json), [checks and limits](report.json), [deliberate failure](deliberate-rejection.log). No hardware/hosted CI was exercised.

| Probe | Value |
|---|---:|
| cubic_imd_db | -38.136259 |
| drive_alias_reduction_db | 81.313079 |
| filtered_sweep_rms | 0.309319759 |
| impulse_peak | 0.233549625 |
| impulse_tail_rms | 0 |
| known_snr_db | 55.7298277 |
| known_thd_db | -39.9999996 |
| lowpass_100_db | -3.73485685e-06 |
| lowpass_cutoff_db | -3.01029906 |
| lowpass_stop_db | -22.9002301 |
| noise_dbfs | -64.760857 |
| pure_thdn_db | -140.844611 |
