# Developer and release workflow

## Release notes and version checklist

Maintain [release notes](releases/README.md) for **every major and minor release**,
including 0.x minors. Keep changes under Unreleased in `CHANGELOG.md` as work
lands; this requirement is part of the release process.

Before packaging a versioned release:

1. Set the shared workspace version and update version references and the lockfile.
2. Create `docs/releases/vX.Y.Z.md` from [the template](releases/TEMPLATE.md).
   Describe implemented changes, breaking APIs, migration steps, dependencies,
   product/parameter IDs and state compatibility, actual validation and limits.
3. Move the relevant Unreleased entries into a dated changelog section, add
   the release to [the index](releases/README.md), and link it from the README.
4. Run the applicable validation and link its evidence. Mark untested platforms,
   hardware and DAW scenarios as pending; preserve historical measurements.
5. Include the notes and changelog in the source archive and use the same notes
   for any release announcement. Creating notes does not publish a release.

Keep released notes intact. Add dated corrections or follow-up sections for
later information. Patch releases
need a changelog entry; create a separate note when compatibility or migration
needs explanation. Documentation-only maintenance does not require a version bump.

## Development commands

The source workspace is pre-1.0. Keep product/parameter IDs stable and version state schemas before changing saved formats. `cargo-soniclaw` creates app, effect and sampler projects against a local checkout.

```sh
cargo install --path tools/cargo-soniclaw
cargo soniclaw new my-app --kind app --framework /path/to/soniclaw
cargo soniclaw new my-effect --kind clap --framework /path/to/soniclaw
cargo soniclaw new my-sampler --kind sampler --framework /path/to/soniclaw
cargo soniclaw doctor --framework /path/to/soniclaw
cargo soniclaw check --framework /path/to/soniclaw
```

Generated plug-in projects enable an embedded editor by default. Their `--no-default-features` build is headless. Generated manifests use local paths and their own workspace boundary. Nothing is published to crates.io by the generator.

`cargo soniclaw build-formats`, `cargo soniclaw compat` and `cargo soniclaw bundle` forward their arguments to the corresponding Python tools. Run those tools with `--help` for inputs. The framework's format recipe currently targets the shared reference factory; adapt its imported Rust archive/name/identities for a generated product.

`python3 tools/validate.py` runs formatting, Clippy, tests and documentation checks into a local log directory. Optional `--clap-validator`, `--vst3-validator` and `--vst3` arguments add external checks. Target-system native dependencies are still required.

## Performance regression checks

The frozen v0.2 baseline contains byte-identical crate sources plus a matching small reference effect. Use the same independent CLAP host, Rust toolchain, generic CPU target and optimized thin-LTO configuration for the baseline and current release. Run them sequentially with other builds stopped. The preserved v0.3 results are historical; v0.5 benchmarks have not been rerun as part of this feature release.

```sh
cargo build --release --manifest-path benchmarks/baseline-v020/Cargo.toml -p sonora-reference-baseline
cargo build --release -p soniclaw-reference
python3 tools/run-compat.py --library benchmarks/baseline-v020/target/release/libsonora_reference_baseline.so --clap-sdk vendor/formats/clap --output benchmarks/results/v020
python3 tools/run-compat.py --library target/release/libsoniclaw_reference.so --clap-sdk vendor/formats/clap --output benchmarks/results/v050
python3 tools/compare-benchmarks.py benchmarks/results/v020/callback-batches.csv benchmarks/results/v050/callback-batches.csv
```

`--fail-regression-percent N` is available for an optional gate on a controlled runner. It is not enabled on shared CI/VMs, where normal variance can exceed small thresholds. Raw per-round measurements, compiler/environment data and comparison limits accompany the results. This is a Soniclaw-before/after comparison; it is not a fresh JUCE benchmark.

## Packages

```sh
python3 tools/collect-licenses.py --format-deps vendor/formats --output dist/licenses
python3 tools/package-release.py build/formats/Release/SoniclawReference.vst3 --licenses dist/licenses
# Linux: also create a .deb; does not install it
python3 tools/package-release.py build/formats/Release/SoniclawReference.vst3 --licenses dist/licenses --installer --output dist/installer-check
```

The package helper stages an already-built artifact, copies first-party licenses and an optional collected dependency-notice directory, writes SHA-256 metadata and makes a portable ZIP. It refuses an existing release directory. Use a Linux baseline compatible with your target distributions; building on newer glibc does not make an older-system-compatible binary. The Debian package is a developer packaging path; verify its runtime dependencies and installation behavior on the target distribution before distribution.

On macOS, `--installer` invokes `pkgbuild`; `--sign-identity <identity>` explicitly invokes `codesign` for the plug-in. Installer signing/notarization and distribution credentials are not automatic. On Windows, `packaging/windows/Soniclaw.iss` provides an Inno Setup VST3 bundle installer recipe. Windows/macOS recipes have not been executed in this Linux environment.

The CI configuration checks Linux/macOS/Windows source builds and generated projects. A workflow file is not evidence of successful remote runs. No signing identity, certificate or developer account is included, and nothing has been installed into the user's DAW directories or published.

## v0.6 portable DSP CPU fixtures

`tools/benchmark-dsp.py --baseline /path/to/extracted-v0.5/soniclaw --output /new/results`
compiles identical Gain/LowPass/SVF/RMS/FDN fixtures against two checkouts before
alternating seven optimized measurement rounds. Raw CSV, compiler/environment,
method and medians are in [v0.6 evidence](validation/v060/cpu/summary.json).
These are static-control DSP microbenchmarks on a shared VM, not a new JUCE,
CLAP-host, driver or dense-modulation performance comparison.
