// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fields moved, scaled and turned in space.

use exedra_math::Placement3;

use crate::bounds::outward;
use crate::claims::Claims;
use crate::formula::Formula;
use crate::number::Number;
use crate::vector::{max3, min3, offset};
use crate::{FieldError, check_finite, check_positive};

/// A field moved by an offset.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Translate<F> {
    field: F,
    offset: [f64; 3],
}

impl<F> Translate<F> {
    /// `field` moved by `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the offset is not finite.
    pub fn new(field: F, offset: [f64; 3]) -> Result<Self, FieldError> {
        check_finite("offset", offset)?;
        Ok(Self { field, offset })
    }

    /// The field moved.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// How far it is moved.
    #[must_use]
    pub const fn offset(&self) -> [f64; 3] {
        self.offset
    }
}

impl<F: Formula> Formula for Translate<F> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.field.eval(offset(p, self.offset))
    }

    fn claims(&self) -> Claims {
        let claims = self.field.claims();
        Claims {
            metric: claims.metric,
            support: claims.support.map(|b| {
                outward(
                    core::array::from_fn(|i| b.min[i] + self.offset[i]),
                    core::array::from_fn(|i| b.max[i] + self.offset[i]),
                )
            }),
        }
    }
}

/// A field scaled uniformly about the origin.
///
/// Values scale with the field, so an exact distance stays exact.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Scale<F> {
    field: F,
    factor: f64,
}

impl<F> Scale<F> {
    /// `field` scaled by `factor` about the origin.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the factor is not finite and positive.
    pub fn new(field: F, factor: f64) -> Result<Self, FieldError> {
        check_positive("factor", factor)?;
        Ok(Self { field, factor })
    }

    /// The field scaled.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// The scale factor.
    #[must_use]
    pub const fn factor(&self) -> f64 {
        self.factor
    }
}

impl<F: Formula> Formula for Scale<F> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        self.field.eval(p.map(|c| c / self.factor)) * self.factor
    }

    fn claims(&self) -> Claims {
        let claims = self.field.claims();
        let factor = self.factor;
        Claims {
            // `s f(p / s)` changes as fast as `f` does.
            metric: claims.metric,
            support: claims
                .support
                .map(|b| outward(b.min.map(|v| v * factor), b.max.map(|v| v * factor))),
        }
    }
}

/// A field turned and moved by a rigid placement.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Placed<F> {
    field: F,
    placement: Placement3,
}

/// How far a placement's axes may stray from orthonormal.
const RIGID_TOLERANCE: f64 = 1.0e-9;

impl<F> Placed<F> {
    /// `field`, defined in local coordinates, placed in space by
    /// `placement`, which carries local points to world points.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the placement is not finite, or not a
    /// rotation and translation: its axes must be unit length and mutually
    /// perpendicular to within `1e-9`, and right-handed.
    ///
    /// The axes are made orthonormal before use, so evaluation, claims and
    /// [`placement`](Self::placement) all describe one rigid motion.
    pub fn new(field: F, placement: Placement3) -> Result<Self, FieldError> {
        if !placement.is_finite() {
            return Err(FieldError::NonFinite {
                parameter: "placement",
            });
        }
        let axes = axes(&placement);
        if !axes
            .iter()
            .all(|axis| exedra_math::is_unit(*axis, RIGID_TOLERANCE))
            || !exedra_math::is_orthogonal_frame(axes, RIGID_TOLERANCE)
            || placement.linear_determinant() <= 0.0
        {
            return Err(FieldError::NotOrthogonal {
                parameter: "placement",
            });
        }
        Ok(Self {
            field,
            placement: rigid(&placement, axes),
        })
    }

    /// The field placed.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// The placement, local to world.
    #[must_use]
    pub const fn placement(&self) -> Placement3 {
        self.placement
    }
}

/// `placement` with nearly orthonormal `axes` made orthonormal: the first
/// axis kept in direction, the second made perpendicular to it, and the
/// third their cross product.
fn rigid(placement: &Placement3, axes: [[f64; 3]; 3]) -> Placement3 {
    let x = exedra_math::normalize(axes[0]).unwrap_or(axes[0]);
    let along = exedra_math::dot(axes[1], x);
    let y = exedra_math::normalize(core::array::from_fn(|i| axes[1][i] - along * x[i]))
        .unwrap_or(axes[1]);
    let z = exedra_math::cross(x, y);
    let rows = &placement.rows;
    Placement3::from_axes(x, y, z, [rows[0][3], rows[1][3], rows[2][3]])
}

/// The placement's local axes in world coordinates: its matrix columns.
fn axes(placement: &Placement3) -> [[f64; 3]; 3] {
    core::array::from_fn(|column| core::array::from_fn(|row| placement.rows[row][column]))
}

impl<F: Formula> Formula for Placed<F> {
    fn eval<N: Number>(&self, p: [N; 3]) -> N {
        let rows = &self.placement.rows;
        let d = offset(p, [rows[0][3], rows[1][3], rows[2][3]]);
        // The inverse rotation is the transpose: local coordinates are the
        // offset along each axis.
        let local = core::array::from_fn(|axis| {
            d[0] * rows[0][axis] + d[1] * rows[1][axis] + d[2] * rows[2][axis]
        });
        self.field.eval(local)
    }

    fn claims(&self) -> Claims {
        let claims = self.field.claims();
        Claims {
            metric: claims.metric,
            support: claims.support.map(|b| {
                let corners = b.corners().map(|c| self.placement.transform_point(c));
                let min = corners.iter().copied().reduce(min3)?;
                let max = corners.iter().copied().reduce(max3)?;
                // Rounding in placing a point and in carrying it back, in
                // proportion to the coordinates' size, not the box's.
                let size = corners
                    .iter()
                    .chain(&b.corners())
                    .flatten()
                    .fold(0.0_f64, |size, v| size.max(v.abs()));
                let slack = 64.0 * f64::EPSILON * size;
                outward(min.map(|v| v - slack), max.map(|v| v + slack))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use exedra_math::Placement3;

    use super::{Placed, Scale, Translate};
    use crate::{Cuboid, Formula, Metric, Sphere, Support};

    #[test]
    fn transforms_move_values_and_claims() {
        let sphere = Sphere::new([0.0; 3], 1.0).unwrap();
        let moved = Translate::new(sphere, [2.0, 0.0, 0.0]).unwrap();
        assert_eq!(moved.eval([2.0, 0.0, 1.0]), 0.0);
        let Support::Bounded(bounds) = moved.claims().support else {
            panic!("bounded");
        };
        assert!(bounds.contains([3.0, 0.0, 0.0]) && !bounds.contains([-0.5, 0.0, 0.0]));

        let scaled = Scale::new(sphere, 3.0).unwrap();
        assert!((scaled.eval([0.0, 4.0, 0.0]) - 1.0).abs() < 1e-15);
        assert_eq!(scaled.claims().metric, Metric::ExactDistance);

        let cuboid = Cuboid::new([0.0; 3], [2.0, 0.5, 0.5]).unwrap();
        let turned = Placed::new(
            cuboid,
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 0.0, 0.0, 1.0),
        )
        .unwrap();
        // The long axis now runs along y.
        assert!(turned.eval([0.0, 1.9, 1.0]) < 0.0);
        assert!(turned.eval([1.9, 0.0, 1.0]) > 0.0);
        let Support::Bounded(bounds) = turned.claims().support else {
            panic!("bounded");
        };
        assert!(bounds.contains([0.0, 2.0, 1.0]) && bounds.max[0] < 0.6);
    }

    #[test]
    fn nearly_rigid_placements_are_made_rigid() {
        // Accepted, though stretched by 4e-10 along x.
        let stretch = 1.0 + 4e-10;
        let placement = Placement3::from_axes(
            [stretch, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0; 3],
        );
        let far = Sphere::new([1e9, 0.0, 0.0], 0.1).unwrap();
        let placed = Placed::new(far, placement).unwrap();
        let Support::Bounded(bounds) = placed.claims().support else {
            panic!("bounded");
        };
        // Wherever evaluation puts the interior, the claims contain it.
        for x in [1e9 - 0.09, 1e9, 1e9 + 0.09] {
            let p = [x, 0.0, 0.0];
            assert!(placed.eval(p) < 0.0, "{x}");
            assert!(bounds.contains(p), "{bounds:?} misses {x}");
        }
        assert_eq!(placed.placement().rows[0][0], 1.0);
    }

    #[test]
    fn placements_must_be_rigid() {
        let sphere = Sphere::new([0.0; 3], 1.0).unwrap();
        let stretched =
            Placement3::from_axes([2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
        assert!(Placed::new(sphere, stretched).is_err());
        let mirrored =
            Placement3::from_axes([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]);
        assert!(Placed::new(sphere, mirrored).is_err());
        assert!(Scale::new(sphere, 0.0).is_err());
        assert!(Translate::new(sphere, [f64::INFINITY, 0.0, 0.0]).is_err());
    }
}
