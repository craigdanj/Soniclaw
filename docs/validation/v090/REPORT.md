# Soniclaw v0.9.0 Linux verification — 2026-11-17

Rust 1.100.0, Linux x86_64. These are fresh local results. No physical audio device was opened and no remote CI result is claimed.

| Check | Result |
|---|---|
| Final workspace tests, all features | **151 passed**, zero failures, including doc tests |
| New registry integration tests | 11 passed: external pack discovery/audio, no callback heap work, state migration, exact host parameter Arc reuse, failed-restore preservation, metadata/settings bounds, timed automation partition independence, repeated-construction cleanup, bundle/background load, modulation, custom editor/fallback, and ProjectSession edit/undo/redo |
| New device manager policy tests | 8 passed with mock devices: rates/channels/buffers/formats, loss/reconnect timing, stream errors, explicit fallback/return to preferred output, disabled retry, stop cancellation, default output changes, preference validation/persistence and duplicate-name occurrence |
| Workbench editor actions | Registered menu add, save/reload and existing bundle/recovery actions passed without opening audio hardware |
| Source quality | Formatting; workspace/all-target/all-feature Clippy with warnings denied; documentation with warnings denied passed |
| CLAP ABI smoke tests | Legacy, project, DSP Lab and all four workbench products passed |
| Prepared DSP audit | **48** cases across multiband, Poly Motion, sampler and the registered Tape Colour node: finite output, zero callback heap allocations/deallocations and stopped-state transactions |
| Independent host fixture | Separate main/audio lifecycle, timed parameters, MIDI fallback with 19 output events, main callback, bounds transaction and state round trip passed; host wrapper callback allocations zero |
| Registry walkthrough | Dry/processed WAV, version 1-to-2 migration, host control identity and pack/unpack/reconstruction passed |
| Project workflow regression | Portable sampler bundle, background checkpoint/recovery load and prepared audio render passed |
| Native panels | Audio settings, registered Tape editor, graph and sample-map panels captured with Xvfb/software OpenGL; the new settings/editor screens visually inspected |
| Templates | App, CLAP and sampler defaults checked under a parent path containing spaces |
| Historical records | 134 released notes/evidence files preserved byte for byte from the v0.8 archive |

`checks.json` records the runtime/build sequence and its initial 150-test suite. `final-checks.json` and `final-tests.log` record the final 151-test suite after adding registry retention to ProjectSession. Native captures and raw audit/host JSON accompany this report. CI YAML and Python tools parsed successfully; the configured registry walkthrough has not been run remotely.

Run `cargo test --workspace --all-features --locked`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo doc --workspace --all-features --no-deps --locked` for source validation. Use `cargo run -p soniclaw-sdk-tools -- --registry-demo /new/folder` and `--audit /new/audit.json` for the walkthrough/audit. Other exact commands are in checks.json; adjust checkout/output paths. The archived registry walkthrough is in examples/sdk-tools/registry-rendered.

## Scope and limitations

Mock device tests establish the state machine and control-thread ownership behavior, not physical driver reliability. Native UI captures show zero device-open attempts. Actual interface unplug/replug, buffer negotiation, latency, backend-specific enumeration, device clocks and Windows/macOS runtime require target hardware. This release manages output; existing duplex input/recording remains a separate API.

Requested buffers are not measured round-trip latency. Name/occurrence selectors are not universal hardware IDs. Reconnection rebuilds DSP/voice/transport history; it is not guaranteed seamless. Preferences are synced explicit writes, not power-loss-safe transactions.

Registry code is compiled/trusted. Custom projects require the corresponding registration and executable code; the portable bundle does not include code. Configuration migrations do not automatically migrate exported parameter names/ranges. The simple Tape Colour example is not a calibrated or oversampled tape model. Registry editors are egui callbacks, not hosted native CLAP editors.

Debug audit timings include shared-machine scheduling/overruns; no fresh CPU comparison or JUCE performance/parity claim is made. Real DAW sessions, arbitrary third-party compatibility, physical devices, Windows/macOS runtime, fresh VST3/AUv2 wrappers and signing/installers remain pending. Existing historical measurements retain their original scope.
