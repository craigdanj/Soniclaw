# Processor composition and latency-aware mixing

Use `soniclaw::dsp::composition` (or `soniclaw_dsp::composition` directly) with the existing interleaved `AudioProcessor` trait.

## Build a parallel effect

Put processors in a `ProcessorChain` with `push`, then call `prepare` with the application's audio format. Stages run in insertion order and receive the same block-relative MIDI and transport context. Call `process_checked` for in-place processing or `process_separate` with a separate identically shaped input slice. Unprepared, mismatched, and oversized calls return an error through these two methods and leave the output untouched. The trait entry point clears invalid buffers.

Wrap the wet processor in `Parallel::new(inner, MixingRule::Linear, 0.5, 64)`. The wet processor must report its fixed integer latency after preparation. The wrapper delays the dry copy by that amount, giving a coherent blend even when the compressor looks ahead. Setting the mix to zero fades to aligned dry audio while the wet state keeps running. `EqualPower` is available for less correlated paths and can raise the level of coherent signals. Phase differences from dispersive filters can remain after integer alignment.

```sh
cargo run -p soniclaw-dsp-suite -- composition output/composition
cargo build -p soniclaw-parallel-compressor --release
```

The first command renders stereo dry, half-wet and fully wet WAVs with five milliseconds of lookahead. The second builds a CLAP example. On Linux, copy `target/release/libsoniclaw_parallel_compressor.so` as `Soniclaw Parallel Compressor.clap` to a folder scanned by your CLAP host. The Wet Mix parameter is available through the host's generic UI and automation. The plug-in reports 240 samples of latency at 48 kHz. The workspace includes no platform-specific packaging for this example.

## Reuse a mono processor

`Duplicated::new(Arc::new(coefficients), factory)` calls the factory during preparation with a cloned handle to the same immutable coefficients. Each returned mono processor owns its history and receives a one-channel spec and the same context. This works for independent channel effects. Use a multichannel processor when the detector or state should be linked between channels. All instances must report the same latency.

## Lifecycle and recovery

Build topology and allocate on the control thread. Own each prepared instance on one processing thread. `reset` clears delay and child state. Change bus layout, maximum block size, or latency by preparing again away from processing. If a child changes its reported latency unexpectedly the parallel wrapper clears the buffer until preparation can be repeated. `DryWetMixer` can also align an independently rendered wet block. It accepts up to 64 channels and 4,194,304 samples of delay storage. The parallel wrapper applies the same sample bound to its scratch block. The mono adapter allows at most 1,048,576 scratch frames.

Existing effects, product IDs, and saved sessions require no migration. Serialize your own stable mix/processor controls and reconstruct runtime state on load. Delay history and smoothing progress are transient. The CLAP example saves Wet Mix through the regular plug-in parameter state.

[Executed verification](validation/v0_115_0/REPORT.md) covers channel isolation, crossfade continuity, allocation-free processing, separate-buffer equivalence and impulse alignment.
