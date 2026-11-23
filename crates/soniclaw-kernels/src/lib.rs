//! Runtime dispatch selected off the callback. Unsafe code is confined to checked architecture kernels.
mod denormals;
pub mod simd;
pub use denormals::{with_denormals_flushed, DenormalGuard};

#[derive(Clone, Copy, Debug)]
pub struct ClampRange {
    minimum: f32,
    maximum: f32,
}
impl ClampRange {
    pub fn new(minimum: f32, maximum: f32) -> Option<Self> {
        (minimum.is_finite() && maximum.is_finite() && minimum <= maximum)
            .then_some(Self { minimum, maximum })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Scalar,
    Sse2,
    Avx2,
    Neon,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnsupportedBackend,
    LengthMismatch,
    InvalidGain,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl Backend {
    pub fn supported(self) -> bool {
        match self {
            Self::Scalar => true,
            Self::Sse2 => {
                #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                {
                    std::is_x86_feature_detected!("sse2")
                }
                #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
                {
                    false
                }
            }
            Self::Avx2 => {
                #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                {
                    std::is_x86_feature_detected!("avx2")
                }
                #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
                {
                    false
                }
            }
            Self::Neon => {
                #[cfg(target_arch = "aarch64")]
                {
                    std::arch::is_aarch64_feature_detected!("neon")
                }
                #[cfg(not(target_arch = "aarch64"))]
                {
                    false
                }
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Kernel {
    backend: Backend,
}
macro_rules! dispatch{($self:expr,$op:ident,$($arg:expr),*)=>{match $self.backend{Backend::Scalar=>scalar::$op($($arg),*),#[cfg(any(target_arch="x86",target_arch="x86_64"))]Backend::Sse2=>{
// SAFETY: Kernel::new checked CPU support; public methods validated slice lengths.
unsafe{sse::$op($($arg),*)}},#[cfg(any(target_arch="x86",target_arch="x86_64"))]Backend::Avx2=>{
// SAFETY: Kernel::new checked CPU support; public methods validated slice lengths.
unsafe{avx::$op($($arg),*)}},#[cfg(target_arch="aarch64")]Backend::Neon=>{
// SAFETY: Kernel::new checked CPU support; public methods validated slice lengths.
unsafe{neon::$op($($arg),*)}},_=>unreachable!("validated architecture backend")}}}
impl Default for Kernel {
    fn default() -> Self {
        Self {
            backend: Backend::Scalar,
        }
    }
}
impl Kernel {
    pub fn new(backend: Backend) -> Result<Self, Error> {
        if backend.supported() {
            Ok(Self { backend })
        } else {
            Err(Error::UnsupportedBackend)
        }
    }
    pub fn best() -> Self {
        for b in [Backend::Avx2, Backend::Sse2, Backend::Neon] {
            if let Ok(k) = Self::new(b) {
                return k;
            }
        }
        Self::default()
    }
    pub fn backend(self) -> Backend {
        self.backend
    }
    pub fn gain(self, data: &mut [f32], gain: f32) -> Result<(), Error> {
        if !gain.is_finite() {
            return Err(Error::InvalidGain);
        }
        dispatch!(self, gain, data, gain);
        Ok(())
    }
    pub fn mix_add(self, dst: &mut [f32], src: &[f32], gain: f32) -> Result<(), Error> {
        if dst.len() != src.len() {
            return Err(Error::LengthMismatch);
        }
        if !gain.is_finite() {
            return Err(Error::InvalidGain);
        }
        dispatch!(self, mix, dst, src, gain);
        Ok(())
    }
    pub fn multiply(self, dst: &mut [f32], src: &[f32]) -> Result<(), Error> {
        if dst.len() != src.len() {
            return Err(Error::LengthMismatch);
        }
        dispatch!(self, multiply, dst, src);
        Ok(())
    }

    /// Clip finite and infinite values to the range, preserving NaNs and in-range signed zero.
    pub fn clamp(self, data: &mut [f32], range: ClampRange) {
        dispatch!(self, clamp, data, range);
    }

    /// Products round to f32; accumulation follows source order in f64 on every backend.
    pub fn dot(self, a: &[f32], b: &[f32]) -> Result<f64, Error> {
        if a.len() != b.len() {
            return Err(Error::LengthMismatch);
        }
        Ok(dispatch!(self, dot, a, b))
    }
    pub fn interleave(self, dst: &mut [f32], left: &[f32], right: &[f32]) -> Result<(), Error> {
        if left.len() != right.len() || left.len().checked_mul(2) != Some(dst.len()) {
            return Err(Error::LengthMismatch);
        }
        dispatch!(self, interleave, dst, left, right);
        Ok(())
    }
}
mod scalar {
    use super::ClampRange;

    pub fn multiply(a: &mut [f32], b: &[f32]) {
        for (a, b) in a.iter_mut().zip(b) {
            *a *= *b;
        }
    }

    pub fn clamp(a: &mut [f32], range: ClampRange) {
        for v in a {
            *v = v.clamp(range.minimum, range.maximum);
        }
    }

    pub fn gain(a: &mut [f32], g: f32) {
        for v in a {
            *v *= g
        }
    }
    pub fn mix(a: &mut [f32], b: &[f32], g: f32) {
        for (a, b) in a.iter_mut().zip(b) {
            *a += *b * g
        }
    }
    pub fn dot(a: &[f32], b: &[f32]) -> f64 {
        a.iter().zip(b).map(|(a, b)| (*a * *b) as f64).sum()
    }
    pub fn interleave(dst: &mut [f32], a: &[f32], b: &[f32]) {
        for (i, (a, b)) in a.iter().zip(b).enumerate() {
            dst[i * 2] = *a;
            dst[i * 2 + 1] = *b;
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod sse {
    use super::ClampRange;

    #[target_feature(enable = "sse2")]
    pub unsafe fn multiply(dst: &mut [f32], src: &[f32]) {
        let mut index = 0;
        while index + 4 <= dst.len() {
            // SAFETY: the dispatcher checked equal lengths and both full vectors fit.
            unsafe {
                let a = _mm_loadu_ps(dst.as_ptr().add(index));
                let b = _mm_loadu_ps(src.as_ptr().add(index));
                _mm_storeu_ps(dst.as_mut_ptr().add(index), _mm_mul_ps(a, b));
            }
            index += 4;
        }
        super::scalar::multiply(&mut dst[index..], &src[index..]);
    }

    #[target_feature(enable = "sse2")]
    pub unsafe fn clamp(data: &mut [f32], range: ClampRange) {
        let low = _mm_set1_ps(range.minimum);
        let high = _mm_set1_ps(range.maximum);
        let mut index = 0;
        while index + 4 <= data.len() {
            // SAFETY: every full vector is inside the exclusively borrowed slice.
            unsafe {
                let input = _mm_loadu_ps(data.as_ptr().add(index));
                let output = _mm_min_ps(high, _mm_max_ps(low, input));
                _mm_storeu_ps(data.as_mut_ptr().add(index), output);
            }
            index += 4;
        }
        super::scalar::clamp(&mut data[index..], range);
    }

    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[target_feature(enable = "sse2")]
    pub unsafe fn gain(a: &mut [f32], gain: f32) {
        let g = _mm_set1_ps(gain);
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: the loop proves a full unaligned vector is inside the exclusively borrowed slice.
            unsafe {
                _mm_storeu_ps(
                    a.as_mut_ptr().add(i),
                    _mm_mul_ps(_mm_loadu_ps(a.as_ptr().add(i)), g),
                );
            }
            i += 4;
        }
        super::scalar::gain(&mut a[i..], gain);
    }
    #[target_feature(enable = "sse2")]
    pub unsafe fn mix(a: &mut [f32], b: &[f32], gain: f32) {
        let g = _mm_set1_ps(gain);
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: equal lengths were checked; both complete vector ranges are in bounds and do not alias mutably.
            unsafe {
                _mm_storeu_ps(
                    a.as_mut_ptr().add(i),
                    _mm_add_ps(
                        _mm_loadu_ps(a.as_ptr().add(i)),
                        _mm_mul_ps(_mm_loadu_ps(b.as_ptr().add(i)), g),
                    ),
                );
            }
            i += 4;
        }
        super::scalar::mix(&mut a[i..], &b[i..], gain);
    }
    #[target_feature(enable = "sse2")]
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f64 {
        let mut i = 0;
        let mut sum = 0.;
        let mut values = [0.; 4];
        while i + 4 <= a.len() {
            // SAFETY: source vectors and temporary destination are complete and in bounds.
            unsafe {
                _mm_storeu_ps(
                    values.as_mut_ptr(),
                    _mm_mul_ps(
                        _mm_loadu_ps(a.as_ptr().add(i)),
                        _mm_loadu_ps(b.as_ptr().add(i)),
                    ),
                );
            }
            for v in values {
                sum += v as f64;
            }
            i += 4;
        }
        for (a, b) in a[i..].iter().zip(&b[i..]) {
            sum += (*a * *b) as f64;
        }
        sum
    }
    #[target_feature(enable = "sse2")]
    pub unsafe fn interleave(dst: &mut [f32], a: &[f32], b: &[f32]) {
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: public lengths guarantee eight destination samples for four samples in each source.
            unsafe {
                let l = _mm_loadu_ps(a.as_ptr().add(i));
                let r = _mm_loadu_ps(b.as_ptr().add(i));
                _mm_storeu_ps(dst.as_mut_ptr().add(i * 2), _mm_unpacklo_ps(l, r));
                _mm_storeu_ps(dst.as_mut_ptr().add(i * 2 + 4), _mm_unpackhi_ps(l, r));
            }
            i += 4;
        }
        super::scalar::interleave(&mut dst[i * 2..], &a[i..], &b[i..]);
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx {
    use super::ClampRange;

    #[target_feature(enable = "avx2")]
    pub unsafe fn multiply(dst: &mut [f32], src: &[f32]) {
        let mut index = 0;
        while index + 8 <= dst.len() {
            // SAFETY: the dispatcher checked equal lengths and both full vectors fit.
            unsafe {
                let a = _mm256_loadu_ps(dst.as_ptr().add(index));
                let b = _mm256_loadu_ps(src.as_ptr().add(index));
                _mm256_storeu_ps(dst.as_mut_ptr().add(index), _mm256_mul_ps(a, b));
            }
            index += 8;
        }
        super::scalar::multiply(&mut dst[index..], &src[index..]);
    }

    #[target_feature(enable = "avx2")]
    pub unsafe fn clamp(data: &mut [f32], range: ClampRange) {
        let low = _mm256_set1_ps(range.minimum);
        let high = _mm256_set1_ps(range.maximum);
        let mut index = 0;
        while index + 8 <= data.len() {
            // SAFETY: every full vector is inside the exclusively borrowed slice.
            unsafe {
                let input = _mm256_loadu_ps(data.as_ptr().add(index));
                let output = _mm256_min_ps(high, _mm256_max_ps(low, input));
                _mm256_storeu_ps(data.as_mut_ptr().add(index), output);
            }
            index += 8;
        }
        super::scalar::clamp(&mut data[index..], range);
    }

    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    #[target_feature(enable = "avx2")]
    pub unsafe fn gain(a: &mut [f32], gain: f32) {
        let g = _mm256_set1_ps(gain);
        let mut i = 0;
        while i + 8 <= a.len() {
            // SAFETY: the loop proves a full unaligned vector is inside the exclusively borrowed slice.
            unsafe {
                _mm256_storeu_ps(
                    a.as_mut_ptr().add(i),
                    _mm256_mul_ps(_mm256_loadu_ps(a.as_ptr().add(i)), g),
                );
            }
            i += 8;
        }
        super::scalar::gain(&mut a[i..], gain);
    }
    #[target_feature(enable = "avx2")]
    pub unsafe fn mix(a: &mut [f32], b: &[f32], gain: f32) {
        let g = _mm256_set1_ps(gain);
        let mut i = 0;
        while i + 8 <= a.len() {
            // SAFETY: equal lengths were checked; both complete vector ranges are in bounds and do not alias mutably.
            unsafe {
                _mm256_storeu_ps(
                    a.as_mut_ptr().add(i),
                    _mm256_add_ps(
                        _mm256_loadu_ps(a.as_ptr().add(i)),
                        _mm256_mul_ps(_mm256_loadu_ps(b.as_ptr().add(i)), g),
                    ),
                );
            }
            i += 8;
        }
        super::scalar::mix(&mut a[i..], &b[i..], gain);
    }
    #[target_feature(enable = "avx2")]
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f64 {
        let mut i = 0;
        let mut sum = 0.;
        let mut values = [0.; 8];
        while i + 8 <= a.len() {
            // SAFETY: source vectors and temporary destination are complete and in bounds.
            unsafe {
                _mm256_storeu_ps(
                    values.as_mut_ptr(),
                    _mm256_mul_ps(
                        _mm256_loadu_ps(a.as_ptr().add(i)),
                        _mm256_loadu_ps(b.as_ptr().add(i)),
                    ),
                );
            }
            for v in values {
                sum += v as f64;
            }
            i += 8;
        }
        for (a, b) in a[i..].iter().zip(&b[i..]) {
            sum += (*a * *b) as f64;
        }
        sum
    }
    #[target_feature(enable = "avx2")]
    pub unsafe fn interleave(dst: &mut [f32], a: &[f32], b: &[f32]) {
        let mut i = 0;
        while i + 8 <= a.len() {
            // SAFETY: public lengths guarantee sixteen destination samples per eight-source-sample vector.
            unsafe {
                let l = _mm256_loadu_ps(a.as_ptr().add(i));
                let r = _mm256_loadu_ps(b.as_ptr().add(i));
                let lo = _mm256_unpacklo_ps(l, r);
                let hi = _mm256_unpackhi_ps(l, r);
                _mm256_storeu_ps(
                    dst.as_mut_ptr().add(i * 2),
                    _mm256_permute2f128_ps(lo, hi, 0x20),
                );
                _mm256_storeu_ps(
                    dst.as_mut_ptr().add(i * 2 + 8),
                    _mm256_permute2f128_ps(lo, hi, 0x31),
                );
            }
            i += 8;
        }
        super::scalar::interleave(&mut dst[i * 2..], &a[i..], &b[i..]);
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use super::ClampRange;

    #[target_feature(enable = "neon")]
    pub unsafe fn multiply(dst: &mut [f32], src: &[f32]) {
        let mut index = 0;
        while index + 4 <= dst.len() {
            // SAFETY: the dispatcher checked equal lengths and both full vectors fit.
            unsafe {
                let a = vld1q_f32(dst.as_ptr().add(index));
                let b = vld1q_f32(src.as_ptr().add(index));
                vst1q_f32(dst.as_mut_ptr().add(index), vmulq_f32(a, b));
            }
            index += 4;
        }
        super::scalar::multiply(&mut dst[index..], &src[index..]);
    }

    #[target_feature(enable = "neon")]
    pub unsafe fn clamp(data: &mut [f32], range: ClampRange) {
        let low = vdupq_n_f32(range.minimum);
        let high = vdupq_n_f32(range.maximum);
        let mut index = 0;
        while index + 4 <= data.len() {
            // SAFETY: every full vector is inside the exclusively borrowed slice.
            unsafe {
                let input = vld1q_f32(data.as_ptr().add(index));
                let bounded = vbslq_f32(vcltq_f32(input, low), low, input);
                let output = vbslq_f32(vcgtq_f32(bounded, high), high, bounded);
                vst1q_f32(data.as_mut_ptr().add(index), output);
            }
            index += 4;
        }
        super::scalar::clamp(&mut data[index..], range);
    }

    use std::arch::aarch64::*;
    #[target_feature(enable = "neon")]
    pub unsafe fn gain(a: &mut [f32], gain: f32) {
        let g = vdupq_n_f32(gain);
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: four samples fit in the mutable slice; NEON unaligned accesses are supported.
            unsafe {
                vst1q_f32(
                    a.as_mut_ptr().add(i),
                    vmulq_f32(vld1q_f32(a.as_ptr().add(i)), g),
                );
            }
            i += 4;
        }
        super::scalar::gain(&mut a[i..], gain);
    }
    #[target_feature(enable = "neon")]
    pub unsafe fn mix(a: &mut [f32], b: &[f32], gain: f32) {
        let g = vdupq_n_f32(gain);
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: checked equal slices contain four complete source/destination samples.
            unsafe {
                vst1q_f32(
                    a.as_mut_ptr().add(i),
                    vaddq_f32(
                        vld1q_f32(a.as_ptr().add(i)),
                        vmulq_f32(vld1q_f32(b.as_ptr().add(i)), g),
                    ),
                );
            }
            i += 4;
        }
        super::scalar::mix(&mut a[i..], &b[i..], gain);
    }
    #[target_feature(enable = "neon")]
    pub unsafe fn dot(a: &[f32], b: &[f32]) -> f64 {
        let mut i = 0;
        let mut sum = 0.;
        let mut values = [0.; 4];
        while i + 4 <= a.len() {
            // SAFETY: both vectors and the four-sample stack destination are in bounds.
            unsafe {
                vst1q_f32(
                    values.as_mut_ptr(),
                    vmulq_f32(vld1q_f32(a.as_ptr().add(i)), vld1q_f32(b.as_ptr().add(i))),
                );
            }
            for v in values {
                sum += v as f64;
            }
            i += 4;
        }
        for (a, b) in a[i..].iter().zip(&b[i..]) {
            sum += (*a * *b) as f64;
        }
        sum
    }
    #[target_feature(enable = "neon")]
    pub unsafe fn interleave(dst: &mut [f32], a: &[f32], b: &[f32]) {
        let mut i = 0;
        while i + 4 <= a.len() {
            // SAFETY: validated two-to-one output length contains both four-sample stores.
            unsafe {
                let l = vld1q_f32(a.as_ptr().add(i));
                let r = vld1q_f32(b.as_ptr().add(i));
                vst1q_f32(dst.as_mut_ptr().add(i * 2), vzip1q_f32(l, r));
                vst1q_f32(dst.as_mut_ptr().add(i * 2 + 4), vzip2q_f32(l, r));
            }
            i += 4;
        }
        super::scalar::interleave(&mut dst[i * 2..], &a[i..], &b[i..]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn available_backends_match_scalar_for_unaligned_tails_and_do_not_allocate() {
        let scalar = Kernel::default();
        for backend in [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Neon] {
            let Ok(k) = Kernel::new(backend) else {
                continue;
            };
            for n in 0..260 {
                let a: Vec<_> = (0..n + 2).map(|i| (i as f32 * 0.13).sin()).collect();
                let b: Vec<_> = (0..n + 2).map(|i| (i as f32 * 0.21).cos()).collect();
                let mut expected = a.clone();
                let mut actual = a.clone();
                scalar
                    .mix_add(&mut expected[1..n + 1], &b[1..n + 1], 0.37)
                    .unwrap();
                let mut stereo = vec![0.; n * 2 + 2];
                let mut stereo_ref = stereo.clone();
                scalar
                    .interleave(&mut stereo_ref[1..n * 2 + 1], &a[1..n + 1], &b[1..n + 1])
                    .unwrap();
                let expected_dot = scalar.dot(&a[1..n + 1], &b[1..n + 1]).unwrap();
                let mut actual_dot = 0.;
                let alloc = allocation_counter::measure(|| {
                    k.mix_add(&mut actual[1..n + 1], &b[1..n + 1], 0.37)
                        .unwrap();
                    k.interleave(&mut stereo[1..n * 2 + 1], &a[1..n + 1], &b[1..n + 1])
                        .unwrap();
                    actual_dot = k.dot(&a[1..n + 1], &b[1..n + 1]).unwrap();
                });
                assert_eq!(alloc.count_total, 0);
                assert_eq!(actual, expected);
                assert_eq!(stereo, stereo_ref);
                assert_eq!(actual_dot, expected_dot);
                scalar.gain(&mut expected[1..n + 1], -0.3).unwrap();
                k.gain(&mut actual[1..n + 1], -0.3).unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn errors_leave_destinations_untouched() {
        let k = Kernel::best();
        let mut dst = [1.; 7];
        assert_eq!(
            k.mix_add(&mut dst, &[0.; 8], 1.),
            Err(Error::LengthMismatch)
        );
        assert_eq!(k.gain(&mut dst, f32::NAN), Err(Error::InvalidGain));
        assert_eq!(
            k.interleave(&mut dst, &[1.; 4], &[2.; 4]),
            Err(Error::LengthMismatch)
        );
        assert_eq!(dst, [1.; 7]);
        assert!(k.dot(&[1.], &[]).is_err());
        for b in [Backend::Sse2, Backend::Avx2, Backend::Neon] {
            assert_eq!(Kernel::new(b).is_ok(), b.supported());
        }
    }
}

#[cfg(test)]
mod block_math_tests {
    use super::*;

    #[test]
    fn multiply_and_clamp_match_scalar_for_unaligned_tails() {
        let src: [f32; 29] = std::array::from_fn(|i| (i as f32 + 1.).recip());
        let mut data: [f32; 29] = std::array::from_fn(|i| (i as f32 - 10.) * 15.);
        data[2] = -0.0;
        data[4] = f32::NAN;
        data[8] = f32::INFINITY;
        data[12] = f32::NEG_INFINITY;
        let range = ClampRange::new(0., 1.).unwrap();
        let mut expected = data;
        Kernel::default()
            .multiply(&mut expected[1..], &src[1..])
            .unwrap();
        Kernel::default().clamp(&mut expected[1..], range);
        for backend in [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Neon] {
            let Ok(kernel) = Kernel::new(backend) else {
                continue;
            };
            let mut observed = data;
            let allocations = allocation_counter::measure(|| {
                kernel.multiply(&mut observed[1..], &src[1..]).unwrap();
                kernel.clamp(&mut observed[1..], range);
            });
            assert_eq!(allocations.count_total, 0);
            for (a, b) in observed.iter().zip(expected) {
                assert!(
                    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()),
                    "{backend:?} {a} != {b}"
                );
            }
        }
    }
}
