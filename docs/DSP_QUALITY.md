# Audio-quality regression tools (0.42)

## Workflow and runnable suite

`cargo run -p soniclaw-quality-suite -- NEW_DIRECTORY` executes twelve deterministic reference checks and writes `report.json`, `measurements.json` and editable `limits.json`. Pass a second argument pointing to a limits JSON to apply product-specific thresholds. A failed limit writes its report and exits nonzero; malformed input/measurement errors also fail. This command is included in the native release workflow; that hosted workflow has not been executed here.

The standalone `soniclaw-quality` crate is reexported as `soniclaw::quality`. It provides coherent sine/noise/log-sweep generation, waveform statistics, a normalized one-sided spectrum, THD/THD+N/SNR/noise measurements, arbitrary spur-bin ratios, alias-bin folding, generic mono processor rendering, stepped-frequency response and versioned regression thresholds. Use these APIs with a product's own prepared AudioProcessor and test signal; the example is a representative reference suite, not exhaustive coverage of every Soniclaw processor.

```rust
use soniclaw::quality::{coherent_sine, Spectrum};
let input = coherent_sine(8192, 113, 0.5, 0.0).unwrap();
let measurement = Spectrum::coherent(&input).unwrap().distortion(&[113], 16).unwrap();
assert!(measurement.thdn_db < -100.0);
```

## Measurement definitions and assumptions

Spectral input must contain an integer number of tone cycles at the declared FFT bins. No window is applied; leakage from noncoherent audio is counted as residual noise/distortion. The real one-sided spectrum doubles positive-bin power except DC/Nyquist and normalizes by FFT size squared, agreeing with time-domain mean-square power. DC is excluded from AC distortion/noise. THD counts integer harmonic bins through the specified maximum order that remain below Nyquist. THD+N counts all non-fundamental AC bins. SNR excludes fundamental and counted harmonic bins. `noise_dbfs` reports the remaining unweighted RMS power relative to full scale: a peak-unity sine has -3.0103 dBFS RMS.

`ratio_db(reference_bins,measured_bins)` reports summed measured power relative to summed reference power; use it for explicitly selected IMD/alias products. Reference/measured sets must be nonempty, unique, in range and disjoint. `alias_bin` folds a harmonic bin through Nyquist. The tools do not infer fundamentals, distinguish a harmonic from an unrelated tone, or implement a named IEC/AES/DIN weighting/IMD standard. Ratios/noise use a finite -300 dB floor for serialization, not a claim of 300 dB measurement resolution. The FFT and input are f32; accumulation/statistics are f64.

`statistics` reports DC, RMS, AC RMS, peak and count of samples at/above unity. `response(factory,rate,frequencies,warmup,measure)` creates a fresh processor per tone, renders a 0.25-peak sine and measures post-warmup AC RMS gain. Select warmup long enough for processor latency/tails; arbitrary/noncoherent measurement lengths can bias the result. `render` prepares a mono processor, preserves continuous state across bounded chunks, and rejects nonfinite/excessive output. A processor that represents an internal error as silence needs a gain/energy threshold to detect that failure.

## Reference checks

The shipped suite measures pure coherent sine THD+N, a known -40 dB second harmonic, deterministic additive noise SNR/floor, selected cubic two-tone IMD products, built-in 4 kHz low-pass response at 100/4000/12000 Hz, the third folded alias of naive versus built-in 2x-oversampled soft clipping after settling, an impulse peak/tail and filtered logarithmic sweep RMS. The alias comparison is one tone/drive setting, not a blanket alias rejection specification. Default thresholds include numerical tolerance and test the intended behavior; maintain product-specific baselines and review sound changes before accepting threshold updates.

## Lifecycle, persistence, limits and errors

All APIs allocate/perform FFT/CPU work and run offline on a control/testing worker, never an audio callback. Signals, responses and reports are caller-owned. Threshold `Suite` schema 1 validates unique names, finite min/max and <=256 limits. JSON is <=64 KiB. Evaluation requires exactly the named finite measurements; missing/unexpected values fail rather than silently skipping a regression. Reports store each actual value, limits and pass state. Existing sound/project formats do not change; save baseline versions with source revisions and sample-rate/FFT configuration.

Bounds: FFT sizes 16–32768 powers of two, harmonic order 2–64, <=1024 selected bins, generated noise/sweeps <=1 Mi samples, statistics/render <=4 Mi samples, input/output magnitude <=1,000,000, 8–192 kHz, blocks 1–8192, <=64 response frequencies and at most four seconds each of warmup/measurement. Invalid values return core Error before analysis or report publication. Rendering can mutate the supplied processor before an output failure; discard/reprepare that instance before retry. Keep previous baseline/report files and write a new output directory for each run.

## Validation and scope

Tests calibrate known harmonic and noise levels, Parseval energy, alias folding, malformed spectra/signals, log sweeps, JSON thresholds and explicit failure. The runnable suite tests real built-in DSP; an additional run deliberately tightens a threshold to verify a failed report/nonzero exit. See the release evidence for exact measured values. Hardware ADC/DAC measurements, external DAW/null tests, perceptual listening and standards certification remain separate acceptance work.
