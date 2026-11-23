# Reusable floating-point SIMD types

Use `soniclaw::kernels::simd` to compose custom DSP with `F32x8` and `F64x4`. Vector widths and lane numbering are stable across backends. For example, one vector can hold eight f32 synth voices or four f64 samples. A prepared `Simd` token selects scalar, SSE2, AVX2 or AArch64 NEON. On narrower SIMD instruction sets, each operation processes two hardware registers.

```rust
use soniclaw::kernels::simd::{F64x4, Simd};

fn accelerate(pcm: &mut [f64], simd: Simd) {
    let half = F64x4::splat(0.5);
    for chunk in pcm.chunks_mut(F64x4::LANES) {
        let x = F64x4::load_partial(chunk).expect("valid lane count");
        x.mul(half, simd)
            .store_partial(chunk)
            .expect("valid lane count");
    }
}
```

- `Simd::best()` chooses a supported backend with existing CPU feature detection. `Simd::new(backend)` lets a caller request a particular backend and returns `UnsupportedBackend` if unavailable. Select it off the callback and reuse the copyable token.
- `load` and `store` operate on the first full vector of an unaligned slice. A short slice is rejected. `load_partial` zero-pads a tail and `store_partial` writes only the requested lanes. An overlong tail returns `LengthMismatch` before modifying its destination.
- Arithmetic supports add, subtract, multiply and divide per lane. There is no implicit fused multiply-add. NaN propagation follows the platform's floating-point rules. Comparisons follow IEEE semantics, so comparisons involving NaN return false. Use `finite_mask` if your processor needs to replace NaNs and infinities.
- `Mask8` and `Mask4` support bit operations and lane selection. Use `lt_mask`, `eq_mask`, `abs` and `sum` when composing a processor. Mask operations, selection and ordered reduction use portable lane logic, while arithmetic uses the selected hardware backend. A zero-padded tail participates in comparisons and reductions unless you mask those lanes.

```sh
cargo run -p soniclaw-dsp-suite --locked -- simd-vectors /tmp/soniclaw-simd
```

The example builds a custom clipping processor, compares it to scalar sample-by-sample output, exercises an odd tail and writes its output as WAV. It also records an f64 difference that f32 cannot represent near unity. Backend parity tests cover each available x86 backend, signed zero, NaN, infinity and boundary handling. Confirm the platform backends exercised in the [release verification](validation/v0_146_0/REPORT.md).

## Ownership and compatibility

Use the vector values on the stack with a prepared copyable backend token. Arithmetic, loads, masks and reductions do not allocate. Public array values are safe to move across threads. When implementing an audio callback processor, allocate any persistent audio state in `prepare` as usual. Scalar fallback works when no hardware SIMD backend is available. Existing optimized `Kernel` slice APIs keep their interfaces. Save your processor controls and initialization values as before. CPU backend choice and vector shape are runtime implementation details and do not require saved-state migration.

Compare throughput with the existing slice operations using the actual block and channel layout before choosing an implementation. The examples verify sample results and allocation behavior. The CPU token itself does not promise a speedup for every DSP algorithm.

[Rust architecture intrinsics](https://doc.rust-lang.org/stable/core/arch/)
