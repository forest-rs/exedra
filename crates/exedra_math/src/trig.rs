// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Trigonometry backend: libm whenever that feature is enabled, otherwise the
//! standard library. Unlike `sqrt`, these functions are not correctly rounded,
//! so the two backends may differ in the last bits; constructive evaluation
//! enables libm explicitly to keep recipe arithmetic stable.

#[cfg(feature = "libm")]
pub(crate) use libm::{acos, acosf, cos, sin, sincos, sincosf};

#[cfg(all(feature = "std", not(feature = "libm")))]
mod std_backend {
    extern crate std;

    pub(crate) fn sin(value: f64) -> f64 {
        value.sin()
    }

    pub(crate) fn cos(value: f64) -> f64 {
        value.cos()
    }

    pub(crate) fn sincos(value: f64) -> (f64, f64) {
        value.sin_cos()
    }

    pub(crate) fn sincosf(value: f32) -> (f32, f32) {
        value.sin_cos()
    }

    pub(crate) fn acos(value: f64) -> f64 {
        value.acos()
    }

    pub(crate) fn acosf(value: f32) -> f32 {
        value.acos()
    }
}

#[cfg(all(feature = "std", not(feature = "libm")))]
pub(crate) use std_backend::{acos, acosf, cos, sin, sincos, sincosf};
