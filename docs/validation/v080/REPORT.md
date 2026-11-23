# v0.8.0 fresh Linux verification — 2026-11-17

Rust 1.100.0; Linux x86_64. Local verification only; no remote CI or physical audio device was used.

| Check | Result |
|---|---|
| Final workspace tests, all features | **131 passed**, zero failed, including documentation tests |
| Focused storage/loading tests | 9 passed: bundle/audio round trip, corruption/truncation/trailing data, no-overwrite, traversal/length rejection, recovery integrity/retention/product fallback, worker isolation/latest request, queue/cancel and autosave recovery |
| Editor project actions | Passed async Load, manual checkpoint, Pack, Import, Recover and failed-load preservation without opening an audio device |
| Formatting / Clippy / docs | Passed; workspace/all-target/all-feature lint and documentation warnings denied |
| Source/ABI smoke tests | Legacy, project, DSP Lab and all four workbench CLAP products passed |
| Prepared DSP audits | 36 rate/block cases: finite output, zero callback allocations/deallocations and stopped state transaction checks |
| Independent hosted plug-in | Separate main/audio thread lifecycle, timed parameters, MIDI fallback with 19 echoed events, main callback, bounds transaction and state round trip passed |
| Portable workflow demo | Bundle/import, background checkpoint, background recovery load and prepared one-second sampler WAV passed |
| SDK and DSP walkthroughs | Six preset renders, per-voice comparison, MIDI/SMF, sample slices, live graph replacement, older DSP/ecosystem demos passed |
| Native GUI | Six panels captured with Xvfb/software OpenGL; project sidebar, graph, sample map and voice panels visually inspected |
| Templates | App, CLAP and sampler defaults compiled under a parent path containing spaces |
| Release history | 106 historical release-note/evidence files matched the v0.7 source archive byte for byte |

The final-checks log follows addition of the editor action integration test; its final workspace test log is authoritative for the count above. The earlier checks.json has the full build/runtime sequence. Raw JSON, logs and native captures accompany this report. CI YAML and Python tool syntax parsed successfully; the CI additions have not been run remotely.

Reproducible entry points: `cargo test --workspace --all-features --locked`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo doc --workspace --all-features --no-deps --locked`, and the [workflow guide](../../PROJECT_WORKFLOWS.md) / [SDK guide](../../SDK_070.md). Use a new output path for each walkthrough. CLAP smoke and independent host commands are in checks.json; change paths for your checkout.

Artifacts are in `examples/sdk-tools/workflow-rendered`, `rendered-v080` and `host-rendered-v080`. workflow-rendered contains a portable sampler.soniclaw, an extracted project and recovered-sampler.wav. Archived recovery checkpoints contain the original test root and are execution evidence, not portable recovery files; unpack sampler.soniclaw to create a usable new root.

## Limits

Debug callback timing and screenshot budget percentages include shared-machine scheduling and GUI/render load; overruns occurred. These are correctness/heap audits, not real-time performance guarantees or new JUCE measurements. Historical CPU benchmark data remain unchanged.

Physical audio devices, real DAW editor/reopen/automation acceptance, arbitrary third-party plug-ins, native hosted GUIs, Windows/macOS runtime, fresh VST3/AUv2 wrappers and signing/installers remain untested here. CLAP hosting remains an in-process scoped rendering foundation. Recovery requires external assets; bundle extraction may leave an incomplete new directory after a crash; file sync/hard-link publication does not establish universal power-loss durability. See the workflow guide for detailed bounds and ownership rules.
