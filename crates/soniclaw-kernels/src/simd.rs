//! Fixed-width SIMD values with safe loading and control-thread backend selection.
//!
//! Packed f32 vectors have eight lanes and packed f64 vectors have four lanes
//! on every backend. Narrower SIMD implementations process two halves.

use super::{Backend, Error, Kernel};
use std::ops::{BitAnd, BitOr, BitXor, Not};

/// A CPU-checked execution backend for user-defined packed DSP.
#[derive(Clone, Copy, Debug)]
pub struct Simd(Backend);
impl Simd {
    pub fn new(backend: Backend) -> Result<Self, Error> {
        Kernel::new(backend).map(|_| Self(backend))
    }
    pub fn best() -> Self {
        Self(Kernel::best().backend())
    }
    pub fn backend(self) -> Backend {
        self.0
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Add,
    Subtract,
    Multiply,
    Divide,
}

macro_rules! lanes {
    ($vector:ident, $mask:ident, $scalar:ty, $lanes:literal, $all:expr, $backend:ident) => {
        /// A packed floating-point value. Lane order matches the loaded slice.
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $vector([$scalar; $lanes]);

        /// Lane mask. The least significant bit corresponds to lane zero.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub struct $mask(u8);

        impl $vector {
            pub const LANES: usize = $lanes;
            pub const fn from_array(values: [$scalar; $lanes]) -> Self {
                Self(values)
            }
            pub const fn to_array(self) -> [$scalar; $lanes] {
                self.0
            }
            pub const fn splat(value: $scalar) -> Self {
                Self([value; $lanes])
            }
            /// Read the first full vector. Unaligned slices are supported.
            pub fn load(input: &[$scalar]) -> Result<Self, Error> {
                Ok(Self(
                    *input.first_chunk::<$lanes>().ok_or(Error::LengthMismatch)?,
                ))
            }
            /// Load a tail with zero padding. The slice may contain zero lanes.
            pub fn load_partial(input: &[$scalar]) -> Result<Self, Error> {
                let mut storage = [0.; $lanes];
                let prefix = storage
                    .get_mut(..input.len())
                    .ok_or(Error::LengthMismatch)?;
                prefix.copy_from_slice(input);
                Ok(Self(storage))
            }
            /// Store the vector into the first lanes of the supplied slice.
            pub fn store(self, output: &mut [$scalar]) -> Result<(), Error> {
                let target = output
                    .first_chunk_mut::<$lanes>()
                    .ok_or(Error::LengthMismatch)?;
                *target = self.0;
                Ok(())
            }
            /// Store exactly the number of requested tail lanes.
            pub fn store_partial(self, output: &mut [$scalar]) -> Result<(), Error> {
                let source = self.0.get(..output.len()).ok_or(Error::LengthMismatch)?;
                output.copy_from_slice(source);
                Ok(())
            }

            #[inline]
            pub fn add(self, rhs: Self, simd: Simd) -> Self {
                self.arithmetic(rhs, simd, Operation::Add)
            }
            #[inline]
            pub fn sub(self, rhs: Self, simd: Simd) -> Self {
                self.arithmetic(rhs, simd, Operation::Subtract)
            }
            #[inline]
            pub fn mul(self, rhs: Self, simd: Simd) -> Self {
                self.arithmetic(rhs, simd, Operation::Multiply)
            }
            #[inline]
            pub fn div(self, rhs: Self, simd: Simd) -> Self {
                self.arithmetic(rhs, simd, Operation::Divide)
            }

            #[inline]
            fn arithmetic(self, rhs: Self, simd: Simd, operation: Operation) -> Self {
                match simd.0 {
                    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                    Backend::Sse2 => {
                        // SAFETY: Simd can be created only with a supported CPU backend.
                        Self(unsafe { sse2::$backend(&self.0, &rhs.0, operation) })
                    }
                    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
                    Backend::Avx2 => {
                        // SAFETY: Simd checked CPU feature availability.
                        Self(unsafe { avx2::$backend(&self.0, &rhs.0, operation) })
                    }
                    #[cfg(target_arch = "aarch64")]
                    Backend::Neon => {
                        // SAFETY: Simd checked CPU feature availability.
                        Self(unsafe { neon::$backend(&self.0, &rhs.0, operation) })
                    }
                    _ => Self(std::array::from_fn(|i| {
                        let a = self.0[i];
                        let b = rhs.0[i];
                        match operation {
                            Operation::Add => a + b,
                            Operation::Subtract => a - b,
                            Operation::Multiply => a * b,
                            Operation::Divide => a / b,
                        }
                    })),
                }
            }

            /// Lane-wise comparisons use ordinary IEEE floating-point comparisons.
            pub fn eq_mask(self, rhs: Self) -> $mask {
                self.compare(rhs, |a, b| a == b)
            }
            pub fn lt_mask(self, rhs: Self) -> $mask {
                self.compare(rhs, |a, b| a < b)
            }
            pub fn finite_mask(self) -> $mask {
                self.compare(Self::splat(0.), |a, _| a.is_finite())
            }
            fn compare(self, rhs: Self, predicate: impl Fn($scalar, $scalar) -> bool) -> $mask {
                let mut bits = 0;
                for (i, (a, b)) in self.0.into_iter().zip(rhs.0).enumerate() {
                    bits |= u8::from(predicate(a, b)) << i;
                }
                $mask(bits)
            }
            pub fn abs(self) -> Self {
                Self(self.0.map(<$scalar>::abs))
            }
            /// Reduce in ascending lane order, independent of backend.
            pub fn sum(self) -> $scalar {
                self.0.into_iter().sum()
            }
        }
        impl $mask {
            pub const fn from_bits(bits: u8) -> Self {
                Self(bits & $all)
            }
            pub const fn bits(self) -> u8 {
                self.0
            }
            pub const fn any(self) -> bool {
                self.0 != 0
            }
            pub const fn all(self) -> bool {
                self.0 == $all
            }

            /// Select from the first vector for true lanes, the second for false lanes.
            pub fn select(self, yes: $vector, no: $vector) -> $vector {
                $vector(std::array::from_fn(|i| {
                    if self.0 & (1 << i) != 0 {
                        yes.0[i]
                    } else {
                        no.0[i]
                    }
                }))
            }
        }
        impl BitAnd for $mask {
            type Output = Self;
            fn bitand(self, rhs: Self) -> Self {
                Self(self.0 & rhs.0)
            }
        }
        impl BitOr for $mask {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }
        impl BitXor for $mask {
            type Output = Self;
            fn bitxor(self, rhs: Self) -> Self {
                Self(self.0 ^ rhs.0)
            }
        }
        impl Not for $mask {
            type Output = Self;
            fn not(self) -> Self {
                Self::from_bits(!self.0)
            }
        }
    };
}
lanes!(F32x8, Mask8, f32, 8, 0xff, f32x8);
lanes!(F64x4, Mask4, f64, 4, 0x0f, f64x4);

// Each hardware kernel loads and stores within fixed-width arrays. These
// lengths are exact multiples of its register width on all supported targets.
#[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
macro_rules! packed {
    ($name:ident, $feature:literal, $sample:ty, $lanes:literal, $width:literal,
     $load:ident, $store:ident, $add:ident, $sub:ident, $mul:ident, $div:ident) => {
        #[target_feature(enable = $feature)]
        pub(super) unsafe fn $name(
            a: &[$sample; $lanes],
            b: &[$sample; $lanes],
            op: Operation,
        ) -> [$sample; $lanes] {
            let mut output = [0.; $lanes];
            for i in (0..$lanes).step_by($width) {
                unsafe {
                    let a = $load(a.as_ptr().add(i));
                    let b = $load(b.as_ptr().add(i));
                    let value = match op {
                        Operation::Add => $add(a, b),
                        Operation::Subtract => $sub(a, b),
                        Operation::Multiply => $mul(a, b),
                        Operation::Divide => $div(a, b),
                    };
                    $store(output.as_mut_ptr().add(i), value);
                }
            }
            output
        }
    };
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod sse2 {
    use super::Operation;
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    packed!(
        f32x8,
        "sse2",
        f32,
        8,
        4,
        _mm_loadu_ps,
        _mm_storeu_ps,
        _mm_add_ps,
        _mm_sub_ps,
        _mm_mul_ps,
        _mm_div_ps
    );
    packed!(
        f64x4,
        "sse2",
        f64,
        4,
        2,
        _mm_loadu_pd,
        _mm_storeu_pd,
        _mm_add_pd,
        _mm_sub_pd,
        _mm_mul_pd,
        _mm_div_pd
    );
}
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx2 {
    use super::Operation;
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    packed!(
        f32x8,
        "avx2",
        f32,
        8,
        8,
        _mm256_loadu_ps,
        _mm256_storeu_ps,
        _mm256_add_ps,
        _mm256_sub_ps,
        _mm256_mul_ps,
        _mm256_div_ps
    );
    packed!(
        f64x4,
        "avx2",
        f64,
        4,
        4,
        _mm256_loadu_pd,
        _mm256_storeu_pd,
        _mm256_add_pd,
        _mm256_sub_pd,
        _mm256_mul_pd,
        _mm256_div_pd
    );
}
#[cfg(target_arch = "aarch64")]
mod neon {
    use super::Operation;
    use std::arch::aarch64::*;
    packed!(
        f32x8, "neon", f32, 8, 4, vld1q_f32, vst1q_f32, vaddq_f32, vsubq_f32, vmulq_f32, vdivq_f32
    );
    packed!(
        f64x4, "neon", f64, 4, 2, vld1q_f64, vst1q_f64, vaddq_f64, vsubq_f64, vmulq_f64, vdivq_f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backends_match_scalar_and_keep_wide_precision() {
        let control = Simd::new(Backend::Scalar).unwrap();
        let a = F32x8::from_array([0.0, 3., -4., 0.5, 7., -1., f32::INFINITY, f32::NAN]);
        let b = F32x8::from_array([2., -0., 2., -2., 0.1, 1., 1., 1.]);
        let x = F64x4::from_array([1. + f64::EPSILON, -7., 0., 0.5]);
        let y = F64x4::from_array([1., 0.1, f64::INFINITY, -2.]);
        for backend in [Backend::Scalar, Backend::Sse2, Backend::Avx2, Backend::Neon] {
            let Ok(simd) = Simd::new(backend) else {
                continue;
            };
            for op in [
                Operation::Add,
                Operation::Subtract,
                Operation::Multiply,
                Operation::Divide,
            ] {
                for (v, r) in a
                    .arithmetic(b, simd, op)
                    .to_array()
                    .into_iter()
                    .zip(a.arithmetic(b, control, op).to_array())
                {
                    assert!(v.to_bits() == r.to_bits() || (v.is_nan() && r.is_nan()));
                }
                for (v, r) in x
                    .arithmetic(y, simd, op)
                    .to_array()
                    .into_iter()
                    .zip(x.arithmetic(y, control, op).to_array())
                {
                    assert!(v.to_bits() == r.to_bits() || (v.is_nan() && r.is_nan()));
                }
            }
            assert_eq!(x.sub(y, simd).to_array()[0], f64::EPSILON);
            let measured = allocation_counter::measure(|| {
                std::hint::black_box(a.mul(b, simd).add(a, simd));
            });
            assert_eq!(measured.count_total, 0);
        }
    }

    #[test]
    fn tails_and_lane_masks_handle_edge_cases() {
        let input = [0., -0., 1., 2., f64::NAN];
        let v = F64x4::load(&input[1..]).unwrap();
        let zeros = F64x4::splat(0.);
        assert_eq!(v.eq_mask(zeros), Mask4::from_bits(1));
        assert_eq!(v.finite_mask().bits(), 7);
        assert_eq!((!Mask4::from_bits(1)).bits(), 14);
        assert_eq!(
            v.finite_mask().select(v, zeros).to_array(),
            [-0., 1., 2., 0.]
        );
        assert!(!v.lt_mask(zeros).any());
        assert!(F64x4::load(&input[2..]).is_err());
        let tail = F64x4::load_partial(&[0.125, 0.25]).unwrap();
        let mut output = [1.; 5];
        tail.store_partial(&mut output[..2]).unwrap();
        assert_eq!(output, [0.125, 0.25, 1., 1., 1.]);
        assert!(tail.store_partial(&mut output).is_err());
        assert_eq!(F32x8::load_partial(&[]).unwrap().sum(), 0.);
    }
}
