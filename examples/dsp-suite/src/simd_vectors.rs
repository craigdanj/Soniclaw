use super::*;
use soniclaw::kernels::{
    simd::{F32x8, F64x4, Simd},
    Backend,
};

struct Clip(Simd);
impl AudioProcessor for Clip {
    fn prepare(&mut self, spec: ProcessSpec) -> std::result::Result<(), Error> {
        spec.validate().map(|_| ())
    }
    fn process(&mut self, audio: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let low = F32x8::splat(-0.7);
        let high = F32x8::splat(0.7);
        let drive = F32x8::splat(2.5);
        for chunk in audio.samples_mut().chunks_mut(F32x8::LANES) {
            let sample = F32x8::load_partial(chunk).expect("at most one vector");
            let driven = sample.mul(drive, self.0);
            let bounded = driven.lt_mask(low).select(low, driven);
            let bounded = high.lt_mask(bounded).select(high, bounded);
            bounded.store_partial(chunk).expect("valid output shape");
        }
    }
    fn reset(&mut self) {}
}

pub fn run(out: &Path) -> Result<serde_json::Value> {
    let simd = Simd::best();
    let input: Vec<_> = (0..8195).map(|i| 0.6 * (i as f32 * 0.071).sin()).collect();
    let (audio, output) = render(out, "packed-clip", Box::new(Clip(simd)), input.clone())?;
    assert_eq!(
        output,
        input
            .iter()
            .map(|&x| (x * 2.5).clamp(-0.7, 0.7))
            .collect::<Vec<_>>()
    );
    let v = F64x4::from_array([1. + f64::EPSILON, -0.5, 0.25, 1.]);
    let w = F64x4::from_array([1., -0.25, 0., 1.]);
    let mut wide = F64x4::splat(0.);
    let count = allocation_counter::measure(|| {
        wide = v.sub(w, simd);
    });
    assert_eq!(count.count_total, 0);
    assert_eq!(wide.to_array()[0], f64::EPSILON);
    let checked_backends: Vec<_> = [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Neon]
        .into_iter()
        .filter_map(|backend| Simd::new(backend).ok())
        .map(|v| format!("{:?}", v.backend()))
        .collect();
    Ok(
        serde_json::json!({ "render":audio, "backend":format!("{:?}", simd.backend()),
        "available_backends":checked_backends, "tail_samples":3,
        "f64_residual":wide.to_array()[0], "f64_callback_allocations":count.count_total }),
    )
}
