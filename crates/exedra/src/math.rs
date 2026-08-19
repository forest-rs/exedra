// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal float math helpers with `std`/`libm` backends.

pub(crate) trait FloatExt {
    fn acos_ext(self) -> Self;
    fn ceil_ext(self) -> Self;
    fn cos_ext(self) -> Self;
    fn sin_ext(self) -> Self;
    fn sqrt_ext(self) -> Self;
}

#[cfg(feature = "std")]
impl FloatExt for f32 {
    #[inline]
    fn acos_ext(self) -> Self {
        self.acos()
    }

    #[inline]
    fn ceil_ext(self) -> Self {
        self.ceil()
    }

    #[inline]
    fn cos_ext(self) -> Self {
        self.cos()
    }

    #[inline]
    fn sin_ext(self) -> Self {
        self.sin()
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        self.sqrt()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f32 {
    #[inline]
    fn acos_ext(self) -> Self {
        libm::acosf(self)
    }

    #[inline]
    fn ceil_ext(self) -> Self {
        libm::ceilf(self)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        libm::cosf(self)
    }

    #[inline]
    fn sin_ext(self) -> Self {
        libm::sinf(self)
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        libm::sqrtf(self)
    }
}

#[cfg(feature = "std")]
impl FloatExt for f64 {
    #[inline]
    fn acos_ext(self) -> Self {
        self.acos()
    }

    #[inline]
    fn ceil_ext(self) -> Self {
        self.ceil()
    }

    #[inline]
    fn cos_ext(self) -> Self {
        self.cos()
    }

    #[inline]
    fn sin_ext(self) -> Self {
        self.sin()
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        self.sqrt()
    }
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
impl FloatExt for f64 {
    #[inline]
    fn acos_ext(self) -> Self {
        libm::acos(self)
    }

    #[inline]
    fn ceil_ext(self) -> Self {
        libm::ceil(self)
    }

    #[inline]
    fn cos_ext(self) -> Self {
        libm::cos(self)
    }

    #[inline]
    fn sin_ext(self) -> Self {
        libm::sin(self)
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        libm::sqrt(self)
    }
}
