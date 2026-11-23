# First-order TPT filters

Use `soniclaw_dsp::first_order_tpt::FirstOrderTpt` for a one-pole low-pass, high-pass or all-pass where cutoff can change on every frame. The modes share one topology-preserving integrator and select complementary outputs. Low-pass plus high-pass reproduces the input. The all-pass uses low-pass minus high-pass and has unity magnitude at fixed cutoff.

Prepare with the graph's `ProcessSpec`. For static operation use `AudioProcessor::process`. Use `set_cutoff` and `set_mode` between blocks under the processor owner's exclusive access. For audio-rate changes, supply one prepared or preallocated cutoff value in Hz per frame to `process_modulated`. Each frame's cutoff is shared by its channels, with independent signal history for each channel. The next static block uses the last cutoff set through `set_cutoff`.

```sh
cargo run -p soniclaw-dsp-suite --locked -- first-order-tpt /tmp/soniclaw-tpt
```

The example sweeps the same input through all three modes and writes three WAVs with allocation checks. Unit tests check fixed-cutoff all-pass energy, complementary sum, channel isolation, transactionality on invalid input and block-partition independence.

## Lifecycle, limits and migration

Prepare or reprepare off the callback. Process and update cutoff under exclusive ownership without allocating. Use `reset` for a fresh history after a seek or restart. A cutoff must be positive and at most 0.49 times the prepared sample rate. Invalid modulation, unprepared use or a mismatched block returns a typed error before modifying samples through `process_modulated`. The infallible `AudioProcessor` entry silences a malformed block. A nonfinite sample clears its own channel history and is replaced by silence.

Cutoff and mode changes are immediate and can be audible. Smooth controls upstream when desired. Time-varying all-pass coefficients change phase over time and fixed-cutoff gain guarantees apply only to fixed settings. Keep mode and cutoff in application state and rebuild the histories after load. There is no new saved schema or migration.

[Release validation](validation/v0_142_0/REPORT.md)
