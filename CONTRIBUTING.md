# Contributing

Document user-visible changes under Unreleased in `CHANGELOG.md`. Every major
and minor release, including pre-1.0 minors, must have its own release notes in
`docs/releases/`, covering changes, compatibility/migration, actual validation
and known limitations. Follow the template and checklist in `docs/RELEASE.md`;
preserve prior release notes and label later corrections with their date.

Keep processing independent from devices, UI and plug-in formats. Follow the lifecycle and real-time contract in `docs/REALTIME.md`. Design public types around valid ownership and explicit failure; do not hide a mutex behind an audio-friendly name.

Every substantial new audio feature needs a behavioral test: expected impulse/timing/pitch response, deterministic rendering, state round-trip or a real host integration. Include processing allocation checks when a change touches a callback path. A test that just repeats implementation details does not establish audio correctness.

Run formatting, Clippy, tests and documentation checks listed in the README. CI runs those checks across native platform runners; physical audio devices and DAW validation still require separate sessions. Keep the validation record honest about that distinction.

Before adding a dependency, check its maintenance, feature scope, platform support and license. Keep GUI/native dependencies optional. Avoid git-only dependencies in released packages unless there is a recorded reason. Workspace members share the release version until the API boundaries stabilize.

Do not add a placeholder crate solely to fill the roadmap. Implement a small useful module with an example, explain its current limits, and make it compose with existing processors.

Unsafe FFI is permitted only in the dedicated CLAP adapter (including its scoped floating-point control), exported entry-point code, native-window handles in soniclaw-editor and independent host tools. Those crates explicitly opt out of the safe workspace lint and enforce `unsafe_op_in_unsafe_fn`. Every raw-pointer operation must preserve the host ABI's lifetime, buffer and thread invariants. Do not extend that exception to the core, DSP, devices, recorder or portable soniclaw-ui controls.
