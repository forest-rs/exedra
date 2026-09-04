// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Adapts Fidget expression shapes to Exedra's implicit-field interface.
//!
//! Start with [`VmField`], which wraps a `fidget::vm::VmShape` and implements
//! [`exedra_isosurface::ScalarField`]. The wrapper reuses interval, value, and
//! gradient evaluator storage across calls. Enable the `jit` feature to obtain
//! `JitField` on native targets.
//!
//! Shapes may depend only on Fidget's `x`, `y`, and `z` axes. Construction
//! rejects additional variables with [`FidgetFieldError::ExtraVariables`]. If
//! Fidget later rejects an evaluation, the infallible scalar-field seam reports
//! no interval or fills point output with NaN rather than inventing values.
//!
//! # Example
//!
//! ```
//! use exedra_fidget::VmField;
//! use exedra_isosurface::ScalarField;
//! use fidget::{context::Tree, vm::VmShape};
//!
//! let x = Tree::x();
//! let y = Tree::y();
//! let z = Tree::z();
//! let sphere = (x.square() + y.square() + z.square()).sqrt() - 1.0;
//! let field = VmField::new(VmShape::from(sphere)).expect("shape uses only x/y/z");
//!
//! let mut values = [0.0; 2];
//! field.eval_points(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &mut values);
//! assert_eq!(values, [-1.0, 1.0]);
//! ```

mod error;
mod field;

pub use error::FidgetFieldError;
#[cfg(all(feature = "jit", not(target_arch = "wasm32")))]
pub use field::JitField;
pub use field::{FidgetField, VmField};
