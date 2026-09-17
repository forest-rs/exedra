// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal float math helpers with `std`/`libm` backends.

pub(crate) trait FloatExt {
    fn atan2_ext(self, other: Self) -> Self;
    fn cos_ext(self) -> Self;
    #[cfg(test)]
    fn sin_ext(self) -> Self;
}

#[cfg(feature = "std")]
impl FloatExt for f32 {
    #[inline]
    fn atan2_ext(self, other: Self) -> Self {
        self.atan2(other)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        self.cos()
    }

    #[inline]
    #[cfg(test)]
    fn sin_ext(self) -> Self {
        self.sin()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f32 {
    #[inline]
    fn atan2_ext(self, other: Self) -> Self {
        libm::atan2f(self, other)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        libm::cosf(self)
    }

    #[inline]
    #[cfg(test)]
    fn sin_ext(self) -> Self {
        libm::sinf(self)
    }
}

#[cfg(feature = "std")]
impl FloatExt for f64 {
    #[inline]
    fn atan2_ext(self, other: Self) -> Self {
        self.atan2(other)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        self.cos()
    }

    #[inline]
    #[cfg(test)]
    fn sin_ext(self) -> Self {
        self.sin()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f64 {
    #[inline]
    fn atan2_ext(self, other: Self) -> Self {
        libm::atan2(self, other)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        libm::cos(self)
    }

    #[inline]
    #[cfg(test)]
    fn sin_ext(self) -> Self {
        libm::sin(self)
    }
}
