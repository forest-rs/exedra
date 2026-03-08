// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Internal float math helpers with `std`/`libm` backends.

pub(crate) trait FloatExt {
    fn acos_ext(self) -> Self;
    fn cos_ext(self) -> Self;
    fn sqrt_ext(self) -> Self;
}

#[cfg(feature = "std")]
impl FloatExt for f32 {
    #[inline]
    fn acos_ext(self) -> Self {
        self.acos()
    }

    #[inline]
    fn cos_ext(self) -> Self {
        self.cos()
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
    fn cos_ext(self) -> Self {
        libm::cosf(self)
    }

    #[inline]
    fn sqrt_ext(self) -> Self {
        libm::sqrtf(self)
    }
}
