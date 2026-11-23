# Real-input FFT and analysis windows

Use `RealFft::new(size)` from `soniclaw::dsp::advanced::spectral`, then allocate a real time buffer of `size()` and a complex spectrum of `bins()`. For N samples the spectrum has N/2+1 bins, including DC and Nyquist. The frequency of bin k is k times sample rate divided by N.

`forward` returns an unnormalized DFT. To round-trip, pass the spectrum to `inverse`, which normalizes the result and requires real DC and Nyquist bins. A half-size packed FFT uses the same prepared RustFFT backend as the pre-existing complex processor.

Use `magnitudes` for one-sided linear peak amplitudes. It corrects for the window's coherent gain and doubles interior bins, leaving DC and Nyquist undoubled. A 0.5-amplitude sinusoid centered on an interior bin reads 0.5. Set `window` to `None` for the rectangular case. Off-bin energy spreads across bins; flat-top minimizes amplitude loss at the nearest bin, while Blackman–Harris trades frequency resolution for sidelobe suppression. Magnitudes are peak amplitude, so convert to dBFS with 20 log10 of the amplitude when full scale is 1.

`AnalysisWindow` offers rectangular, triangular, Hann, Hamming, Blackman, Blackman–Harris, flat-top and Kaiser. Use periodic tables for FFT analysis and symmetric tables when windowing an FIR kernel. Unity coherent-gain normalization is available for stand-alone `apply`; `magnitudes` calibrates either kind of coefficient table. Kaiser beta accepts 0–24.

```sh
cargo run -p soniclaw-dsp-suite -- real-fft output/real-fft
```

The example measures centered and half-bin-shifted tones across four windows and writes a one-sided CSV spectrum.

## Lifecycle and recovery

Plan, build windows, and allocate buffers off the audio thread. Reuse the same plan on one processing thread, or use one plan per concurrent stream. The transform has no inter-block history. The FFT range is 16 to 32,768 power-of-two samples. The window constructor permits all lengths in that range. Bad dimensions, nonfinite input, or complex DC/Nyquist values on inverse return an error before writing output. Resizing requires a new plan.

Legacy `FftProcessor`, `WindowKind`, and STFT entry points keep their original API and normalization. Persist analysis configuration in your application and rebuild tables on load. There are no saved project, parameter or format migrations.

[Executed verification](validation/v0_118_0/REPORT.md) exercises complex-reference equivalence, reconstruction, magnitude calibration and the allocation contract.
