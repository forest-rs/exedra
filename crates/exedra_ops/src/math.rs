// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal float math helpers with `std`/`libm` backends.

pub(crate) trait FloatExt {
    fn atan2_ext(self, x: Self) -> Self;
    fn rem_euclid_ext(self, rhs: Self) -> Self;
    fn sqrt_ext(self) -> Self;
}

#[cfg(feature = "std")]
impl FloatExt for f32 {
    #[inline]
    fn atan2_ext(self, x: Self) -> Self {
        self.atan2(x)
    }

    #[inline]
    fn rem_euclid_ext(self, rhs: Self) -> Self {
        self.rem_euclid(rhs)
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        self.sqrt()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f32 {
    #[inline]
    fn atan2_ext(self, x: Self) -> Self {
        libm::atan2f(self, x)
    }

    #[inline]
    fn rem_euclid_ext(self, rhs: Self) -> Self {
        let rhs_abs = libm::fabsf(rhs);
        let mut remainder = self % rhs;
        if remainder < 0.0 {
            remainder += rhs_abs;
        }
        remainder
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        libm::sqrtf(self)
    }
}

#[cfg(feature = "std")]
impl FloatExt for f64 {
    #[inline]
    fn atan2_ext(self, x: Self) -> Self {
        self.atan2(x)
    }

    #[inline]
    fn rem_euclid_ext(self, rhs: Self) -> Self {
        self.rem_euclid(rhs)
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        self.sqrt()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f64 {
    #[inline]
    fn atan2_ext(self, x: Self) -> Self {
        libm::atan2(self, x)
    }

    #[inline]
    fn rem_euclid_ext(self, rhs: Self) -> Self {
        let rhs_abs = if rhs < 0.0 { -rhs } else { rhs };
        let mut remainder = self % rhs;
        if remainder < 0.0 {
            remainder += rhs_abs;
        }
        remainder
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        libm::sqrt(self)
    }
}
