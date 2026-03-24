// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Implicit-field seams and extraction-facing data types for Exedra.
//!
//! Current scope:
//! - [`ScalarField`] as the base field-evaluation contract,
//! - [`ScalarField2d`] for profile-space field construction,
//! - [`SpecializableField`] for region-local simplification,
//! - [`ProvenanceField`] for extraction-time semantic tagging,
//! - Hermite bridge types in [`hermite`],
//! - analytic reference fields and CSG combinators in [`analytic`],
//! - analytic 2D reference profiles in [`analytic2d`],
//! - lifting operators in [`lift`],
//! - reusable transform wrappers in [`transform`],
//! - a first dual-contouring extraction path in [`mod@dual_contour`].
//!
//! This crate intentionally starts at the evaluation boundary. It does not yet
//! define a full implicit scene/domain model.

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_isosurface requires either the `std` or `libm` feature");

pub mod analytic;
pub mod analytic2d;
pub mod bounds2;
pub mod dual_contour;
pub mod hermite;
pub mod lift;
pub mod transform;

use exedra_spatial::Aabb;

pub use bounds2::Aabb2;
pub use dual_contour::{
    DualContourError, DualContourParams, DualContourResult, DualContourStats, dual_contour,
    dual_contour_with_regions,
};
pub use hermite::{
    CellHermiteData, CellHermiteIntersection, EdgeIntersectionError, EdgeSearchParams,
    HermiteIntersection, locate_edge_intersection,
};
pub use lift::{Extrude, Revolve};
pub use transform::{RigidTransform3, Transform3, Translate, UniformScale};

/// A scalar field that can be evaluated for isosurface extraction.
///
/// Implementors provide distance-like values whose zero level set defines the
/// extracted surface. Negative values are inside, positive values are outside.
pub trait ScalarField {
    /// Evaluates conservative interval bounds for one spatial region.
    ///
    /// Returning `None` means the field cannot provide bounds for this query
    /// and the caller should fall back to uncullable subdivision.
    ///
    /// The returned interval must conservatively contain every field value in
    /// `bounds`.
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]>;

    /// Evaluates field values at `points`, writing one output per point.
    ///
    /// `out.len()` must equal `points.len()`.
    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]);

    /// Evaluates field values and gradients at `points`.
    ///
    /// Each output row is `[value, dx, dy, dz]`.
    ///
    /// `out.len()` must equal `points.len()`.
    ///
    /// NaN gradients are permitted at non-differentiable points.
    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]);
}

/// A two-dimensional scalar field used for profile-based construction.
///
/// This mirrors [`ScalarField`] closely enough that lifting operators such as
/// extrusion and revolution can stay generic over profile fields without
/// introducing a distinct scene-graph layer.
pub trait ScalarField2d {
    /// Evaluates conservative interval bounds for one 2D region.
    fn eval_interval(&self, bounds: &Aabb2) -> Option<[f32; 2]>;

    /// Evaluates field values at `points`, writing one output per point.
    fn eval_points(&self, points: &[[f32; 2]], out: &mut [f32]);

    /// Evaluates field values and gradients at `points`.
    ///
    /// Each output row is `[value, dx, dy]`.
    fn eval_gradients(&self, points: &[[f32; 2]], out: &mut [[f32; 3]]);
}

/// Optional extension for fields that can simplify themselves for a region.
pub trait SpecializableField: ScalarField {
    /// Specialized field type produced for one region.
    type Specialized: ScalarField;

    /// Returns a cheaper field specialized for `bounds`, when worthwhile.
    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized>;
}

/// Optional extension for fields that expose extraction-time provenance.
pub trait ProvenanceField: ScalarField {
    /// Provenance value carried by the field.
    type Provenance;

    /// Evaluates conservative bounds plus provenance for one region.
    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)>;

    /// Returns provenance at one point.
    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance;
}

#[cfg(test)]
mod tests {
    use super::{
        Aabb2, ScalarField, ScalarField2d, analytic::SphereField, analytic2d::CircleField2d,
    };
    use exedra_spatial::Aabb;

    #[test]
    fn scalar_field_is_object_safe() {
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let field: &dyn ScalarField = &sphere;
        let bounds = Aabb::new([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]).expect("valid bounds");

        assert!(field.eval_interval(&bounds).is_some());
    }

    #[test]
    fn sphere_field_interval_is_conservative_for_sample_points() {
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let bounds = Aabb::new([-0.5, -0.5, -0.5], [0.75, 0.25, 0.5]).expect("valid bounds");
        let interval = sphere
            .eval_interval(&bounds)
            .expect("reference sphere should provide intervals");

        let samples = [
            bounds.min,
            [bounds.max[0], bounds.min[1], bounds.min[2]],
            [bounds.min[0], bounds.max[1], bounds.min[2]],
            [bounds.min[0], bounds.min[1], bounds.max[2]],
            bounds.max,
            bounds.center(),
        ];
        let mut values = [0.0; 6];
        sphere.eval_points(&samples, &mut values);

        for value in values {
            assert!(value >= interval[0] - 1.0e-5);
            assert!(value <= interval[1] + 1.0e-5);
        }
    }

    #[test]
    fn sphere_field_gradients_match_surface_direction() {
        let sphere = SphereField {
            center: [1.0, 2.0, 3.0],
            radius: 2.0,
        };
        let points = [[3.0, 2.0, 3.0], [1.0, 2.0, 3.0]];
        let mut out = [[0.0; 4]; 2];
        sphere.eval_gradients(&points, &mut out);

        assert_eq!(out[0][0], 0.0);
        assert!((out[0][1] - 1.0).abs() <= 1.0e-5);
        assert!(out[0][2].abs() <= 1.0e-5);
        assert!(out[0][3].abs() <= 1.0e-5);

        assert!(out[1][1].is_nan());
        assert!(out[1][2].is_nan());
        assert!(out[1][3].is_nan());
    }

    #[test]
    fn scalar_field_2d_is_object_safe() {
        let circle = CircleField2d {
            center: [0.0, 0.0],
            radius: 1.0,
        };
        let field: &dyn ScalarField2d = &circle;
        let bounds = Aabb2::new([-0.5, -0.5], [0.5, 0.5]).expect("valid bounds");

        assert!(field.eval_interval(&bounds).is_some());
    }
}
