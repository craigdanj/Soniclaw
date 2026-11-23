# Reusable multi-rate oversampling — 0.89.0

## Workflow and purpose

Use `soniclaw_dsp::oversampling` in a synthesizer, effect, instrument or standalone audio application. No Studio, device, plug-in host or GUI dependency is required. Existing `OversampledDrive` remains available with its original 2× behavior/31-frame latency; this release adds a general processor and closure API rather than changing its sound.

Choose `Factor::{Two,Four,Eight}` and `Quality::{Balanced,High}` before playback. Balanced uses 65-tap half-band FIRs; High uses 129 taps. Increasing factor/quality costs CPU and latency. Rebuild off the audio thread to change either. The converter is for integer-factor internal processing, not arbitrary playback-rate conversion.

## Public API and example

```rust
use soniclaw_core::{AudioBuffer, ProcessSpec};
use soniclaw_dsp::oversampling::{Config,Factor,Quality,Oversampler};
let spec = ProcessSpec { sample_rate:48000., max_block_frames:128, channels:2 };
let mut os = Oversampler::new(Config { schema:1, factor:Factor::Four, quality:Quality::High }, spec)?;
// In a callback, with an existing interleaved base-rate buffer:
os.process_with(&mut buffer, |high_rate_buffer, high_rate_spec| {
    for x in high_rate_buffer.samples_mut() { *x = (*x * 4.).tanh(); }
})?;
# Ok::<(), soniclaw_core::Error>(())
```

The snippet illustrates integration; the complete runnable example is `cargo run -p soniclaw-dsp-oversampling --offline -- NEW_DIRECTORY`. It writes stereo 2×/4×/8× drive WAV files and a report with measured callback allocations and reported latencies.

`Oversampled<P: AudioProcessor>::new(inner, config)` wraps a compatible processor. Call `prepare` on a control thread; the inner processor receives a multiplied sample rate and maximum frame count. MIDI offsets and `transport.sample_position` are multiplied by the factor; tempo and playing state retain their meaning. Up to 1024 events are copied into prepared storage. `inner_mut()` permits application-specific controls under the inner processor's threading rules. `process_checked()` returns errors; the `AudioProcessor::process` adapter clears output on error and records `last_error()`. Avoid mutating topology through `inner_mut` after preparation.

## Latency and signal behavior

The conversion uses cascaded 2× linear-phase half-band interpolation/decimation, zero insertion with gain correction, persistent per-channel histories and phase-consistent downsampling. Small coefficients that are mathematically zero are omitted from the FIR sum. Integer base-rate conversion delays are:

| Quality | 2× | 4× | 8× |
|---|---:|---:|---:|
| Balanced | 32 | 48 | 56 |
| High | 64 | 96 | 112 |

The wrapper adds `inner.latency_frames()/factor`. Fractional base-rate inner delays are rejected, so latency compensation never silently rounds a fractional value. These FIRs have a transition band near Nyquist: identity rendering is filtered, not a bit-transparent bypass. Applications mixing a separate dry path must delay it by the reported latency. Flush sufficient zero input for FIR tails in offline renders. Oversampling reduces some aliasing paths; it cannot guarantee zero aliasing for every nonlinear processor.

## Ownership, persistence, bounds and errors

Construction and `prepare` allocate; processing, reset and histories use prepared storage. The closure's own allocation, locking and native calls remain the caller's responsibility. Each engine has a single audio owner; send controls through an appropriate application command/atomic layer. No shared global state is used. `reset` clears histories. Runtime configuration changes require a newly prepared engine, and an application must retire the old one off the callback.

`Config` is serde-compatible, schema 1, with factor/quality snake-case names and rejection of unknown fields. Persist configuration, not FIR/DSP histories; instantiate fresh on reopen. Old presets and old drive behavior are unchanged.

The multiplied specification must pass `ProcessSpec::validate` (at most 1.536 MHz, 64 channels), with no more than 4,194,304 samples per high-rate work buffer. Two work buffers plus bounded FIR histories are prepared. All size arithmetic is checked. Inner processors must accept the resulting high-rate spec; some existing processors intentionally have lower rate ceilings and will reject preparation. Invalid input shape/size/nonfinite values return errors before advancing FIR history. Nonfinite rendered output is silenced and converter history reset. Inner processors remain responsible for recovering their own state. No callback fallback allocates.

## Validation and reference

Executed checks and the example report are in `docs/validation/v0_89_0/`. Tests cover all factors/qualities, measured impulse peak vs latency, impulse DC sum, exact callback-partition equivalence, channel isolation, reset, malformed/oversized configuration, MIDI/clock retiming and callback allocation counts. These checks do not establish a universal stop-band specification, CPU speedup or hardware/DAW acceptance. The next nonlinear release adds explicit spectral comparison fixtures.

For multistage interpolation/decimation background, see Crochiere and Rabiner, [Interpolation and Decimation of Digital Signals (1981)](https://web.ece.ucsb.edu/Faculty/Rabiner/ece259/Reprints/179_interpolation_decimation.pdf). The implementation here is independently written and is not a copy of the paper's code.

## 0.90.0 integration clarification

Reported conversion latency is the audio input-to-output path. MIDI offsets are scaled within their callback; the wrapper does not automatically queue MIDI to compensate the upsampler's input group delay. Use it directly for input-processing effects. A generator that replaces its input must add the upsampler-equivalent delay to its generated audio/event path, or use an application-specific source adapter, before claiming the full reported path latency. This release does not provide that source adapter.
