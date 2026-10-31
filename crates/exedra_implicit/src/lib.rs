// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Implicit fields written once as formulas.
//!
//! A field is a function of space, negative inside a solid and positive
//! outside, whose zero set is the solid's surface. This crate defines fields
//! and what consumers may ask of them; it does not extract surfaces.
//!
//! - A [`Formula`] is written once over any [`Number`], and every question
//!   is answered from that one definition: values in `f64`, exact gradients
//!   in [`Dual`] numbers, bounds over a box in [`Interval`]s, and the
//!   creases its `min`, `max` and `abs` mark.
//! - [`ScalarField`] is the evaluation boundary consumers use. Every formula
//!   is one; fields that cannot be formulas implement it directly and join
//!   formulas through [`Opaque`].
//! - [`Claims`] state how values relate to distance and where the interior
//!   lies. Built-in fields state them from their parameters, and
//!   combinators derive theirs from their parts'.
//!
//! # Example
//!
//! ```
//! use exedra_implicit::{Aabb, Formula, Number, ScalarField};
//!
//! struct Ball;
//!
//! impl Formula for Ball {
//!     fn eval<N: Number>(&self, [x, y, z]: [N; 3]) -> N {
//!         x.hypot(y).hypot(z) - 1.0
//!     }
//! }
//!
//! let mut value = [0.0];
//! Ball.eval_points(&[[0.0, 0.0, 2.0]], &mut value);
//! assert_eq!(value[0], 1.0);
//!
//! let region = Aabb::new([0.5; 3], [0.6; 3]).unwrap();
//! assert!(Ball.eval_interval(&region).contains(0.0));
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_implicit requires either the `std` or `libm` feature");

mod blend;
mod bounds;
pub mod claims;
mod combine;
mod expr;
mod field;
mod formula;
pub mod number;
mod profile;
mod shapes;
mod transform;
mod vector;

pub use blend::SmoothUnionN;
pub use bounds::{Aabb, Aabb2};
pub use claims::{Claims, Metric, ProfileClaims, Support};
pub use combine::{Difference, Intersection, SmoothUnion, Tagged, Union};
pub use expr::{Expr, ProfileExpr};
pub use field::{Crease, ScalarField, Tie};
pub use formula::{Formula, Opaque, Profile};
pub use number::{Dual, Interval, Number};
pub use profile::{Circle, Extrude, HalfPlane, Rectangle, Revolve};
pub use shapes::{Capsule, Cuboid, Cylinder, HalfSpace, RoundCone, Sphere, Torus};
pub use transform::{Placed, Scale, Translate};

/// Why a built-in field's parameters are invalid.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldError {
    /// A coordinate or dimension is NaN or infinite.
    NonFinite {
        /// The parameter's name.
        parameter: &'static str,
    },
    /// A dimension that must be positive is zero or negative.
    NonPositive {
        /// The parameter's name.
        parameter: &'static str,
    },
    /// A dimension that must not be negative is.
    Negative {
        /// The parameter's name.
        parameter: &'static str,
    },
    /// A direction is too short to normalize.
    DegenerateDirection {
        /// The parameter's name.
        parameter: &'static str,
    },
    /// Axes that must be mutually perpendicular are not.
    NotOrthogonal {
        /// The parameter's name.
        parameter: &'static str,
    },
}

impl core::fmt::Display for FieldError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NonFinite { parameter } => {
                write!(f, "field parameter `{parameter}` is not finite")
            }
            Self::NonPositive { parameter } => {
                write!(f, "field parameter `{parameter}` must be positive")
            }
            Self::Negative { parameter } => {
                write!(f, "field parameter `{parameter}` must not be negative")
            }
            Self::DegenerateDirection { parameter } => {
                write!(f, "field parameter `{parameter}` is not a usable direction")
            }
            Self::NotOrthogonal { parameter } => {
                write!(
                    f,
                    "field parameter `{parameter}` has axes that are not perpendicular"
                )
            }
        }
    }
}

impl core::error::Error for FieldError {}

/// Checks that every component is finite.
pub(crate) fn check_finite<const N: usize>(
    parameter: &'static str,
    values: [f64; N],
) -> Result<(), FieldError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(FieldError::NonFinite { parameter })
    }
}

/// Checks that a dimension is finite and positive.
pub(crate) fn check_positive(parameter: &'static str, value: f64) -> Result<(), FieldError> {
    check_finite(parameter, [value])?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(FieldError::NonPositive { parameter })
    }
}

/// Checks that a dimension is finite and not negative.
pub(crate) fn check_nonnegative(parameter: &'static str, value: f64) -> Result<(), FieldError> {
    check_finite(parameter, [value])?;
    if value >= 0.0 {
        Ok(())
    } else {
        Err(FieldError::Negative { parameter })
    }
}

/// Normalizes a direction, checking that it is finite and not degenerate.
pub(crate) fn check_direction(
    parameter: &'static str,
    value: [f64; 3],
) -> Result<[f64; 3], FieldError> {
    check_finite(parameter, value)?;
    // A unit vector is kept exactly, so a direction read back from a field
    // rebuilds the same field.
    if exedra_math::is_unit(value, 4.0 * f64::EPSILON) {
        return Ok(value);
    }
    exedra_math::normalize(value).ok_or(FieldError::DegenerateDirection { parameter })
}
