# Soniclaw ecosystem roadmap

v0.1 established the processing/device/UI foundation. v0.2 added duplex sessions, recording, timed automation/buses, CLAP export and reusable controls. v0.3 implements code across the next six areas: optimization/benchmarks, embedded editors, compatibility tooling, effects, multisampling and additional format/release tooling.

## Current work delivered

See the README feature table and docs/VALIDATION.md for exact implementation and verification scope. New modules have behavior and tests rather than placeholder crate names. CLAP and VST3 are compiled/validated on Linux. The AUv2 integration is a macOS build recipe, not a verified macOS binary.

## Next acceptance priorities

1. Native macOS/Windows builds and GUI/DAW tests, AUv2 validation and signing/installer execution.
2. Real DAW scan, sidechain, automation and project-save/reopen sessions on all target platforms.
3. Physical interface loopback, buffer changes, clock drift, unplug/replug and disk-full recording recovery.
4. Profile remaining adapter/DSP costs on controlled hardware and compare release-to-release regression ranges.
5. Automated live effect settings beyond output gain, smoother filter transitions, high-quality resampling and stronger spectral/aliasing tests.
6. Per-voice disk streaming, seekable transport, richer sample-map editing and full MPE/note expression.
7. Timeline/project editing, undo/redo, complete bank/rack state schemas and migrations.
8. Public documentation hosting, crate/name reservation and publishing, compatibility guarantees and contributor releases.

Keep optional platform code separate, unsafe FFI narrow and documented, and IDs stable once products are distributed. Never infer platform/host validation from a build flag or a configured CI job.
