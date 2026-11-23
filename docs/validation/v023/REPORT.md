# Soniclaw 0.23.0 verification

Executed 2026-11-17, Linux x86_64, Rust 1.100.0 (0ef1bc386), rootless native headers, Xvfb and software OpenGL.

## Workspace and tools

- **202 Rust tests passed**, zero failures, including partition-invariant/zero-allocation callback and bounded-state persistence tests. [Strict workspace checks](final-checks/checks.json) include formatting, all-target/all-feature Clippy with warnings denied, all-feature tests, documentation with warnings denied and example builds. Full logs are adjacent.
- **11 Python tool tests passed**, including child-probe success, native failure exit and timeout behavior. [Log](python-tests.log).
- Source versions, release-note sequence and maintained documentation links passed the [source contract](accepted/source-contract/report.json). Target-machine operator acceptance remains pending.

## Actual VST3 binary

The pinned CLAP-wrapper and VST3 SDK recipe built a real Linux VST3 bundle from the reference Rust static library. [Build log](vst3-bundle2.log), [upstream revisions](format-build.json), [binary digest](native-bundle.json). Native artifacts/dependencies are not installed globally or included as compiled output in this source ZIP.

The child-process [factory probe](accepted/vst3-probe.json) enumerated four audio classes. The native [processing report](accepted/vst3-processing/report.json) verifies:

- Common product selection and normalized Mode/Gain controls; gain-only audio at 0.4.
- A gain change at sample 128 leaves samples 0–127 at 0.4, then starts the processor's expected smoothing ramp (sample 128 about 0.400833; final sample about 0.506667).
- Opaque state/checksum round trip restores output at 0.4; corrupted state is rejected before native restore.
- First buffered nonzero output at **1024 frames**, zero callback allocations, zero input/output drops, zero underruns and zero native errors in the paced 32-block run.
- Retaining a callback after owner stop/destruction yields silence.
- MIDI note at offset seven drives the native drum product; AllNotesOff is delivered, callback allocations remain zero, and energy persists over multiple blocks. Tail energy is about 390.97, with zero repeated restart notifications. This guards the integration patch that avoids latency-only restart cycles resetting instrument voices.

The actual VST3 editor opened, closed and reopened under Xvfb. [Editor report](editor-accepted/vst3-editor-demo/report.json), [screenshot](editor-accepted/vst3-editor.png); the image was visually inspected. The [workbench VST3 panel](workbench/vst3-panel.png) also rendered and was inspected. These are render/lifecycle checks, not automated coverage of every UI interaction.

## Earlier-feature regression checks

After the ring-buffer dependency update, CLAP host ABI, direct callback/owner revocation, full host-session state/bypass, separate sidechain bus processing, WAV/AIFF/FLAC native formats, MIDI scheduling/output mocks and tempo-map/native transport demos all passed. [Exact commands/results](accepted/checks.json). Hardware I/O was not opened for these checks.

## Scope and limitations

The VST3 host uses the included MIT vst3-host 0.11.1 backend with a documented integration patch. Its default device/process-isolation features are disabled. Live native processing remains trusted/in-process and uses a buffered stereo bridge; 1024 frames at 48 kHz adds about 21.33 ms before device/native latency. There is no direct VST3 mode, mixed-format rack session, VST3 auxiliary-bus mapping or complete tempo-map/MPE/SysEx integration.

No physical audio/MIDI device, commercial plug-in collection, external DAW, macOS or Windows runtime acceptance was performed. Paced debug checks are correctness evidence, not a performance guarantee or a fresh JUCE comparison. See the [detailed guide](../../VST3_HOSTING.md) for workflow, lifecycle, persistence, error recovery and backend attribution.
