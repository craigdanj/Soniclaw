# Soniclaw rename (v0.3.0)

The framework is now **Soniclaw**. This is a branding/package rename of the existing v0.3.0 release, not a new DSP feature release. The directory containing Cargo.toml is now `soniclaw/`.

| Previous | Current |
|---|---|
| `sonora` facade / `use sonora::...` | `soniclaw` / `use soniclaw::...` |
| `sonora-*` current crates | `soniclaw-*` |
| `cargo-sonora` / `cargo sonora` | `cargo-soniclaw` / `cargo soniclaw` |
| `SonoraReference` artifact name | `SoniclawReference` |
| `SONORA_EDITOR_TRACE` | `SONICLAW_EDITOR_TRACE` |
| CMake `SONORA_*` build variables | `SONICLAW_*` |

Update Cargo dependency keys/path entries and Rust imports. CLI example:

```sh
cargo install --path tools/cargo-soniclaw
cargo soniclaw new my-effect --kind clap --framework /absolute/path/to/soniclaw
```

The existing example plug-ins retain their `org.sonora.*` product IDs, stable parameter IDs and parameter-state schema. VST3 class identity derives from those stable CLAP IDs; fixed reference bundle IDs, the Windows installer AppId, and AU manufacturer/subtype codes `Snra`/`SnGF` remain unchanged. Display names and generated artifact filenames use Soniclaw. Rescan the renamed artifacts in the host. New generated projects use the `org.soniclaw` namespace and should choose their own product identity before distribution.

The Live FX example saves `soniclaw-session.json`. If it does not exist, startup/Load settings can read the previous `sonora-session.json`; the old file is not overwritten by this migration.

`benchmarks/baseline-v020/` is a frozen, hashed copy of the previous framework. Its crate names and filenames deliberately remain Sonora so it can reproduce the original baseline. Current release-tool commands retain references to that baseline's actual package/binary names. `benchmarks/results/` and existing `docs/validation/` files preserve their original names, measured binaries and recorded outputs; they are historical evidence, not new Soniclaw measurements. Current verification for the rename is recorded separately. The original standalone Sonora/JUCE comparison archive is also a separate historical artifact.

The generic CLAP bundler and macOS package helper default to the new org.soniclaw namespace. To preserve an existing custom macOS CLAP bundle identifier, pass its previous identifier explicitly with `--identifier`.
