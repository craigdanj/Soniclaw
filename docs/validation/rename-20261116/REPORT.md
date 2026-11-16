# Soniclaw rename verification — 2026-11-16

The v0.3.0 framework was renamed from Sonora to Soniclaw. These checks validate the renamed source; this is a rename-only update.

- Cargo metadata resolves all 27 workspace packages under their new Soniclaw names.
- `cargo fmt --all --check` passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passed.
- `cargo test --workspace --all-features --locked`: 55 tests passed, zero failures.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked` passed.
- The renamed CLI and CLAP demo compiled. Exported-library smoke checks passed for Soniclaw Gain and Soniclaw Synth, including the CLAP bundle produced by the renamed bundler.
- All three CLI templates passed Cargo checks from a parent directory containing spaces. The app used its default native configuration; CLAP and sampler used `--no-default-features` for headless checks.
- The frozen baseline source, historical benchmark measurements, and pre-existing validation evidence remain byte-identical to the original archive.

Environment: Debian Linux x86-64, Rust 1.100.0. Native development headers were provided through a temporary sysroot. No physical audio device or DAW session was used. Native VST3/AU builds, platform installers and performance benchmarks were not rerun for this rename; their earlier results remain historical evidence.

Existing demo product IDs, parameter-state identities and fixed platform identifiers remain unchanged for compatibility. See [rename guide](../../RENAMING.md).

Logs in this directory contain the actual commands' output. Absolute build paths in those logs refer to the verification environment.
