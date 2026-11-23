## Implemented in v0.9.0

Managed audio output settings/reconnect and compiled custom DSP registrations are available. Physical-device/DAW/native-platform acceptance, managed duplex reconnect and broader ecosystem tooling remain follow-up work. See [devices](DEVICE_MANAGER.md) and [registry](REGISTRY.md).

## Implemented in v0.8.0

Single-file project bundles, recovery snapshots/autosave and background project loading are now available. Read [project workflows](PROJECT_WORKFLOWS.md).

# Soniclaw ecosystem roadmap

v0.1 established the processing/device/UI foundation. v0.2 added duplex sessions, recording, timed automation/buses, CLAP export and reusable controls. v0.3 added optimization/benchmarks, embedded editors, compatibility tooling, effects, multisampling and additional format/release tooling. v0.4 adds complete-state project factories and undo/redo, automated effects, sinc conversion, seekable/paged disk playback and expressive instruments.

v0.5 adds all eight requested DSP areas with named controls, project nodes and a DSP Lab. See [DSP expansion](DSP_ADVANCED.md).

v0.6 adds modulation, multiband/dynamic EQ, granular/spectral/stereo convolution and the native [DSP Workbench](WORKBENCH.md). Linux virtual-display rendering and current-vs-0.5 DSP CPU fixtures accompany this release; physical interfaces and real DAWs remain acceptance work.

v0.7 delivers per-voice modulation, visual/live graphs, sampler editing, MIDI tools/files, scoped CLAP hosting and diagnostics. See [SDK guide](SDK_070.md).

## Current work delivered

See the README feature table and docs/VALIDATION.md for exact implementation and verification scope. New modules have behavior and tests rather than placeholder crate names. Current source and CLAP export checks are recorded separately from the historical v0.3 Linux CLAP/VST3 validator results. The AUv2 integration is a macOS build recipe, not a verified macOS binary.

## Next acceptance priorities

1. Native macOS/Windows builds and GUI/DAW tests, AUv2 validation and signing/installer execution.
2. Real DAW scan, sidechain, automation and project-save/reopen sessions on all target platforms.
3. Physical interface loopback, buffer changes, clock drift, unplug/replug and disk-full recording recovery.
4. Profile remaining adapter/DSP costs on controlled hardware and compare release-to-release regression ranges.
5. Profile dense automation/sinc/STFT/unison paths, expand transition/alias tests and independent loudness/true-peak compliance vectors, and add optimized kernels.
6. More codecs, disk I/O failure recovery, configurable read-ahead policies and MPE RPN negotiation.
7. Timeline/arrangement editing, richer asset banks and persistent hosted plug-in/editor integration.
8. Public documentation hosting, crate/name reservation and publishing, compatibility guarantees and contributor releases.

Keep optional platform code separate, unsafe FFI narrow and documented, and IDs stable once products are distributed. Never infer platform/host validation from a build flag or a configured CI job.
