# Frozen v0.2.0 benchmark baseline

Crate source files are byte-identical to the v0.2.0 archive; see original-source-sha256.json. Only this small reference application and a minimal workspace manifest were added. The processing modes, parameters and factory match the v0.3 reference. Build with `cargo build --manifest-path benchmarks/baseline-v020/Cargo.toml --release -p sonora-reference-baseline`. This snapshot is for reproducing before/after measurements, not an additional supported framework.
