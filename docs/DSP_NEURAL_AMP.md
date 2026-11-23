# Neural amp model playback

Parse a .nam file with NamLstm::from_json on the control thread, then prepare a mono AudioProcessor at the exact model rate. Read sample_rate before constructing a graph. A host running another rate can explicitly put streaming sample converters around the model. Missing rate metadata uses 48 kHz. Tune set_calibration in dB on the audio owner. For consistent results, persist the asset identity and calibration settings alongside the preset. The existing modeled amp and cabinet APIs remain available.

The bounded reader supports mono LSTM weights, 1–3 layers, hidden width 1–64, format versions 0.5.0 through 0.5.4 or 0.7.0, and model rates 8–192 kHz. It validates dimensions and the packed weights before construction. No training or WaveNet inference is included. Model input is guarded to ±16 after calibration. A compatible IR can process the result of an amp-only model. Use latency compensation if the rest of the chain adds buffering.

Reset restores the saved recurrent state. Use prewarm explicitly on the control thread if your model requires warm-up, and account for silent samples in a comparison. Processing uses prepared state and a bounded stack workspace. It uses standard sigmoid and tanh, which can differ from an upstream build configured for approximate activations. Invalid model data returns an error and should leave the existing effect in place. A failed prepare preserves the prior spec. The runnable fixture generates owned weights, writes the .nam and the numerical input/output, and checks allocation-free block processing against sample-by-sample evaluation. See the reference comparison in the release validation directory.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- nam-lstm NEW_DIRECTORY`. [Executed checks](validation/v0_113_0/REPORT.md).

The independent fixture compares the generated two-layer mono model against the
upstream C++ LSTM implementation at the commit named in
[nam-reference.json](validation/v0_113_0/nam-reference.json). To repeat it on
little-endian Linux, compile the bundled validation driver
[reference.cpp](validation/v0_113_0/reference.cpp) with upstream
`NAM/{lstm,dsp,activations,linear}.cpp`, C++20, `NAM_SAMPLE_FLOAT`, and
the NAM, Eigen and nlohmann header include paths. Run it with the fixture's
`fixture.nam`, `input.f32le` and an output filename. Compare that output
to `rust.f32le` as 32-bit little-endian float samples. The comparison uses
the saved state without an implicit warm-up.
