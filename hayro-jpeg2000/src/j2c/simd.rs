pub(crate) const SIMD_WIDTH: usize = 8;

#[cfg(feature = "simd")]
mod inner {
    use super::SIMD_WIDTH;
    use fearless_simd::{SimdBase, SimdFloat};
    use std::ops::{Add, AddAssign, DivAssign, Mul, MulAssign, Sub, SubAssign};

    pub(crate) use fearless_simd::{Level, Simd, dispatch};

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct f32x8<S: Simd> {
        inner: fearless_simd::f32x8<S>,
    }

    impl<S: Simd> f32x8<S> {
        #[inline(always)]
        pub(crate) fn from_slice(simd: S, slice: &[f32]) -> Self {
            Self {
                inner: fearless_simd::f32x8::from_slice(simd, slice),
            }
        }

        #[inline(always)]
        pub(crate) fn splat(simd: S, value: f32) -> Self {
            Self {
                inner: fearless_simd::f32x8::splat(simd, value),
            }
        }

        #[inline(always)]
        pub(crate) fn madd(self, scalar: f32, addend: Self) -> Self {
            Self {
                inner: self.inner.madd(scalar, addend.inner),
            }
        }

        #[inline(always)]
        pub(crate) fn floor(self) -> Self {
            Self {
                inner: self.inner.floor(),
            }
        }

        #[inline(always)]
        pub(crate) fn store(self, slice: &mut [f32]) {
            slice[..SIMD_WIDTH].copy_from_slice(&self.inner.val);
        }

        #[inline(always)]
        pub(crate) fn zip_low(self, other: Self) -> Self {
            Self {
                inner: self.inner.zip_low(other.inner),
            }
        }

        #[inline(always)]
        pub(crate) fn zip_high(self, other: Self) -> Self {
            Self {
                inner: self.inner.zip_high(other.inner),
            }
        }
    }

    impl<S: Simd> Add for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn add(self, rhs: Self) -> Self {
            Self {
                inner: self.inner + rhs.inner,
            }
        }
    }

    impl<S: Simd> Sub for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn sub(self, rhs: Self) -> Self {
            Self {
                inner: self.inner - rhs.inner,
            }
        }
    }

    impl<S: Simd> Mul for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn mul(self, rhs: Self) -> Self {
            Self {
                inner: self.inner * rhs.inner,
            }
        }
    }

    impl<S: Simd> Add<f32> for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn add(self, rhs: f32) -> Self {
            Self {
                inner: self.inner + rhs,
            }
        }
    }

    impl<S: Simd> Mul<f32> for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn mul(self, rhs: f32) -> Self {
            Self {
                inner: self.inner * rhs,
            }
        }
    }

    impl<S: Simd> AddAssign for f32x8<S> {
        #[inline(always)]
        fn add_assign(&mut self, rhs: Self) {
            self.inner = self.inner + rhs.inner;
        }
    }

    impl<S: Simd> SubAssign for f32x8<S> {
        #[inline(always)]
        fn sub_assign(&mut self, rhs: Self) {
            self.inner = self.inner - rhs.inner;
        }
    }

    impl<S: Simd> MulAssign<f32> for f32x8<S> {
        #[inline(always)]
        fn mul_assign(&mut self, rhs: f32) {
            self.inner = self.inner * rhs;
        }
    }

    impl<S: Simd> DivAssign<f32> for f32x8<S> {
        #[inline(always)]
        fn div_assign(&mut self, rhs: f32) {
            self.inner = self.inner / rhs;
        }
    }
}

#[cfg(not(feature = "simd"))]
mod inner {
    use super::SIMD_WIDTH;
    use core::marker::PhantomData;
    use std::ops::{Add, AddAssign, DivAssign, Mul, MulAssign, Sub, SubAssign};

    pub(crate) trait Simd: Copy + Clone {}

    #[derive(Copy, Clone)]
    pub(crate) struct ScalarSimd;
    impl Simd for ScalarSimd {}

    pub(crate) struct Level;
    impl Level {
        #[inline(always)]
        pub(crate) fn new() -> Self {
            Level
        }
    }

    #[derive(Copy, Clone)]
    #[allow(non_camel_case_types)]
    #[repr(C, align(32))]
    pub(crate) struct f32x8<S: Simd> {
        val: [f32; SIMD_WIDTH],
        _marker: PhantomData<S>,
    }

    impl<S: Simd> f32x8<S> {
        #[inline(always)]
        pub(crate) fn from_slice(_simd: S, slice: &[f32]) -> Self {
            let mut val = [0.0f32; SIMD_WIDTH];
            val.copy_from_slice(&slice[..SIMD_WIDTH]);
            Self {
                val,
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn splat(_simd: S, value: f32) -> Self {
            Self {
                val: [value; SIMD_WIDTH],
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn madd(self, scalar: f32, addend: Self) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = mul_add(self.val[i], scalar, addend.val[i]);
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn floor(self) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i].floor();
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn store(self, slice: &mut [f32]) {
            slice[..SIMD_WIDTH].copy_from_slice(&self.val);
        }

        #[inline(always)]
        pub(crate) fn zip_low(self, other: Self) -> Self {
            Self {
                val: [
                    self.val[0],
                    other.val[0],
                    self.val[1],
                    other.val[1],
                    self.val[2],
                    other.val[2],
                    self.val[3],
                    other.val[3],
                ],
                _marker: PhantomData,
            }
        }

        #[inline(always)]
        pub(crate) fn zip_high(self, other: Self) -> Self {
            Self {
                val: [
                    self.val[4],
                    other.val[4],
                    self.val[5],
                    other.val[5],
                    self.val[6],
                    other.val[6],
                    self.val[7],
                    other.val[7],
                ],
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> Add for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn add(self, rhs: Self) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] + rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> Sub for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn sub(self, rhs: Self) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] - rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> Mul for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn mul(self, rhs: Self) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] * rhs.val[i];
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> Add<f32> for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn add(self, rhs: f32) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] + rhs;
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> Mul<f32> for f32x8<S> {
        type Output = Self;
        #[inline(always)]
        fn mul(self, rhs: f32) -> Self {
            let mut result = [0.0f32; SIMD_WIDTH];
            for i in 0..SIMD_WIDTH {
                result[i] = self.val[i] * rhs;
            }
            Self {
                val: result,
                _marker: PhantomData,
            }
        }
    }

    impl<S: Simd> AddAssign for f32x8<S> {
        #[inline(always)]
        fn add_assign(&mut self, rhs: Self) {
            for i in 0..SIMD_WIDTH {
                self.val[i] += rhs.val[i];
            }
        }
    }

    impl<S: Simd> SubAssign for f32x8<S> {
        #[inline(always)]
        fn sub_assign(&mut self, rhs: Self) {
            for i in 0..SIMD_WIDTH {
                self.val[i] -= rhs.val[i];
            }
        }
    }

    impl<S: Simd> MulAssign<f32> for f32x8<S> {
        #[inline(always)]
        fn mul_assign(&mut self, rhs: f32) {
            for i in 0..SIMD_WIDTH {
                self.val[i] *= rhs;
            }
        }
    }

    impl<S: Simd> DivAssign<f32> for f32x8<S> {
        #[inline(always)]
        fn div_assign(&mut self, rhs: f32) {
            for i in 0..SIMD_WIDTH {
                self.val[i] /= rhs;
            }
        }
    }

    #[inline(always)]
    fn mul_add(a: f32, b: f32, c: f32) -> f32 {
        #[cfg(any(
            all(
                any(target_arch = "x86", target_arch = "x86_64"),
                target_feature = "fma"
            ),
            all(target_arch = "aarch64", target_feature = "neon")
        ))]
        {
            f32::mul_add(a, b, c)
        }
        #[cfg(not(any(
            all(
                any(target_arch = "x86", target_arch = "x86_64"),
                target_feature = "fma"
            ),
            all(target_arch = "aarch64", target_feature = "neon")
        )))]
        {
            a * b + c
        }
    }

    /// Scalar fallback for SIMD dispatch.
    #[macro_export]
    macro_rules! simd_dispatch {
        ($level:expr, $simd:ident => $body:expr) => {{
            let _ = $level;
            let $simd = $crate::j2c::simd::ScalarSimd;
            $body
        }};
    }

    pub(crate) use simd_dispatch as dispatch;
}

pub(crate) use inner::*;
