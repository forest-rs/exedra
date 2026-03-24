// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fidget-backed implementations of the `exedra_isosurface` field seams.

mod error;
mod field;

pub use error::FidgetFieldError;
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
pub use field::JitField;
pub use field::{FidgetField, VmField};
