// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reusable transform wrappers for [`crate::ScalarField`] implementations.

use exedra_spatial::Aabb;

use crate::{ProvenanceField, ScalarField, SpecializableField};
use exedra_math::{add, dot, normalize, scale, sub};

/// Translation wrapper for any scalar field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Translate<F> {
    field: F,
    offset: [f32; 3],
}

/// Uniform scale wrapper for any scalar field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UniformScale<F> {
    field: F,
    factor: f32,
}

/// Rigid local-to-world transform for field wrappers.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RigidTransform3 {
    origin: [f32; 3],
    x_axis: [f32; 3],
    y_axis: [f32; 3],
    z_axis: [f32; 3],
}

/// Rigid-transform wrapper for any scalar field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Transform3<F> {
    field: F,
    transform: RigidTransform3,
}

impl<F> Translate<F> {
    /// Creates a translated field wrapper.
    #[must_use]
    pub const fn new(field: F, offset: [f32; 3]) -> Self {
        Self { field, offset }
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the translation offset.
    #[must_use]
    pub const fn offset(&self) -> [f32; 3] {
        self.offset
    }
}

impl<F> UniformScale<F> {
    /// Creates a uniformly scaled field wrapper.
    ///
    /// `factor` must be finite and strictly positive.
    #[must_use]
    pub fn new(field: F, factor: f32) -> Option<Self> {
        if factor.is_finite() && factor > 0.0 {
            Some(Self { field, factor })
        } else {
            None
        }
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the uniform scale factor.
    #[must_use]
    pub const fn factor(&self) -> f32 {
        self.factor
    }
}

impl RigidTransform3 {
    /// Returns the identity transform.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            z_axis: [0.0, 0.0, 1.0],
        }
    }

    /// Returns a pure translation transform.
    #[must_use]
    pub const fn translated(origin: [f32; 3]) -> Self {
        Self {
            origin,
            ..Self::identity()
        }
    }

    /// Creates a rigid transform from orthonormal local axes and an origin.
    ///
    /// The provided axes are normalized internally. Construction fails when the
    /// axes are degenerate, non-finite, or not mutually orthogonal.
    #[must_use]
    pub fn new(
        origin: [f32; 3],
        x_axis: [f32; 3],
        y_axis: [f32; 3],
        z_axis: [f32; 3],
    ) -> Option<Self> {
        let x_axis = normalize(x_axis)?;
        let y_axis = normalize(y_axis)?;
        let z_axis = normalize(z_axis)?;
        if abs(dot(x_axis, y_axis)) > 1.0e-4
            || abs(dot(x_axis, z_axis)) > 1.0e-4
            || abs(dot(y_axis, z_axis)) > 1.0e-4
        {
            return None;
        }
        Some(Self {
            origin,
            x_axis,
            y_axis,
            z_axis,
        })
    }

    /// Returns the world-space origin of the local frame.
    #[must_use]
    pub const fn origin(&self) -> [f32; 3] {
        self.origin
    }

    /// Returns the local x axis in world space.
    #[must_use]
    pub const fn x_axis(&self) -> [f32; 3] {
        self.x_axis
    }

    /// Returns the local y axis in world space.
    #[must_use]
    pub const fn y_axis(&self) -> [f32; 3] {
        self.y_axis
    }

    /// Returns the local z axis in world space.
    #[must_use]
    pub const fn z_axis(&self) -> [f32; 3] {
        self.z_axis
    }

    /// Maps a world-space point into local coordinates.
    #[must_use]
    pub fn world_to_local_point(&self, point: [f32; 3]) -> [f32; 3] {
        let delta = sub(point, self.origin);
        [
            dot(delta, self.x_axis),
            dot(delta, self.y_axis),
            dot(delta, self.z_axis),
        ]
    }

    /// Maps a local-space vector into world coordinates.
    #[must_use]
    pub fn local_to_world_vector(&self, vector: [f32; 3]) -> [f32; 3] {
        add(
            add(scale(self.x_axis, vector[0]), scale(self.y_axis, vector[1])),
            scale(self.z_axis, vector[2]),
        )
    }

    fn local_bounds_for_world_aabb(&self, bounds: &Aabb) -> Option<Aabb> {
        let mut minimum = [f32::INFINITY; 3];
        let mut maximum = [f32::NEG_INFINITY; 3];
        for corner in corners(bounds) {
            let local = self.world_to_local_point(corner);
            minimum = min3(minimum, local);
            maximum = max3(maximum, local);
        }
        Aabb::new(minimum, maximum)
    }
}

impl<F> Transform3<F> {
    /// Creates a rigidly transformed field wrapper.
    #[must_use]
    pub const fn new(field: F, transform: RigidTransform3) -> Self {
        Self { field, transform }
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the rigid transform.
    #[must_use]
    pub const fn transform(&self) -> RigidTransform3 {
        self.transform
    }
}

impl<F: ScalarField> ScalarField for Translate<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        self.field
            .eval_interval(&translate_bounds(bounds, neg3(self.offset))?)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = sub(*point, self.offset);
            }
            self.field
                .eval_points(points_to_local(points, &local_points), out);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = sub(*point, self.offset);
            }
            self.field
                .eval_gradients(points_to_local(points, &local_points), out);
        }
    }
}

impl<F: ScalarField> ScalarField for UniformScale<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let scaled = scale_bounds(bounds, 1.0 / self.factor)?;
        self.field
            .eval_interval(&scaled)
            .map(|interval| [interval[0] * self.factor, interval[1] * self.factor])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = scale(*point, 1.0 / self.factor);
            }
            self.field
                .eval_points(points_to_local(points, &local_points), out);
            for value in out.iter_mut() {
                *value *= self.factor;
            }
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = scale(*point, 1.0 / self.factor);
            }
            self.field
                .eval_gradients(points_to_local(points, &local_points), out);
            for gradient in out.iter_mut() {
                gradient[0] *= self.factor;
            }
        }
    }
}

impl<F: ScalarField> ScalarField for Transform3<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let local_bounds = self.transform.local_bounds_for_world_aabb(bounds)?;
        self.field.eval_interval(&local_bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = self.transform.world_to_local_point(*point);
            }
            self.field
                .eval_points(points_to_local(points, &local_points), out);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut local_points = [[0.0; 3]; 16];
        for (points, out) in points.chunks(16).zip(out.chunks_mut(16)) {
            for (index, point) in points.iter().enumerate() {
                local_points[index] = self.transform.world_to_local_point(*point);
            }
            self.field
                .eval_gradients(points_to_local(points, &local_points), out);
            for gradient in out.iter_mut() {
                let world_gradient =
                    self.transform
                        .local_to_world_vector([gradient[1], gradient[2], gradient[3]]);
                gradient[1..4].copy_from_slice(&world_gradient);
            }
        }
    }
}

impl<F: SpecializableField> SpecializableField for Translate<F> {
    type Specialized = Translate<F::Specialized>;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        self.field
            .specialize(&translate_bounds(bounds, neg3(self.offset))?)
            .map(|field| Translate::new(field, self.offset))
    }
}

impl<F: SpecializableField> SpecializableField for UniformScale<F> {
    type Specialized = UniformScale<F::Specialized>;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        let scaled = scale_bounds(bounds, 1.0 / self.factor)?;
        self.field
            .specialize(&scaled)
            .and_then(|field| UniformScale::new(field, self.factor))
    }
}

impl<F: SpecializableField> SpecializableField for Transform3<F> {
    type Specialized = Transform3<F::Specialized>;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        let local_bounds = self.transform.local_bounds_for_world_aabb(bounds)?;
        self.field
            .specialize(&local_bounds)
            .map(|field| Transform3::new(field, self.transform))
    }
}

impl<F: ProvenanceField> ProvenanceField for Translate<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.field
            .eval_interval_with_provenance(&translate_bounds(bounds, neg3(self.offset))?)
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(sub(point, self.offset))
    }
}

impl<F: ProvenanceField> ProvenanceField for UniformScale<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let scaled = scale_bounds(bounds, 1.0 / self.factor)?;
        self.field
            .eval_interval_with_provenance(&scaled)
            .map(|(interval, provenance)| {
                (
                    [interval[0] * self.factor, interval[1] * self.factor],
                    provenance,
                )
            })
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(scale(point, 1.0 / self.factor))
    }
}

impl<F: ProvenanceField> ProvenanceField for Transform3<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let local_bounds = self.transform.local_bounds_for_world_aabb(bounds)?;
        self.field.eval_interval_with_provenance(&local_bounds)
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field
            .point_provenance(self.transform.world_to_local_point(point))
    }
}

fn points_to_local<'a>(points: &[[f32; 3]], local_points: &'a [[f32; 3]; 16]) -> &'a [[f32; 3]] {
    &local_points[..points.len()]
}

fn translate_bounds(bounds: &Aabb, offset: [f32; 3]) -> Option<Aabb> {
    Aabb::new(add(bounds.min, offset), add(bounds.max, offset))
}

fn scale_bounds(bounds: &Aabb, factor: f32) -> Option<Aabb> {
    let min = scale(bounds.min, factor);
    let max = scale(bounds.max, factor);
    Aabb::new(min3(min, max), max3(min, max))
}

fn corners(bounds: &Aabb) -> [[f32; 3]; 8] {
    [
        [bounds.min[0], bounds.min[1], bounds.min[2]],
        [bounds.max[0], bounds.min[1], bounds.min[2]],
        [bounds.min[0], bounds.max[1], bounds.min[2]],
        [bounds.max[0], bounds.max[1], bounds.min[2]],
        [bounds.min[0], bounds.min[1], bounds.max[2]],
        [bounds.max[0], bounds.min[1], bounds.max[2]],
        [bounds.min[0], bounds.max[1], bounds.max[2]],
        [bounds.max[0], bounds.max[1], bounds.max[2]],
    ]
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
}

fn neg3(vector: [f32; 3]) -> [f32; 3] {
    [-vector[0], -vector[1], -vector[2]]
}

fn max3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

fn min3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]
}

fn abs(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.abs()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::fabsf(value)
    }
}

#[cfg(test)]
mod tests {
    use exedra_spatial::Aabb;

    use crate::{
        ScalarField,
        analytic::{CylinderField, HalfSpaceField, SphereField},
    };

    use super::{RigidTransform3, Transform3, Translate, UniformScale};

    fn assert_gradient_close(actual: [f32; 4], expected: [f32; 4]) {
        for axis in 0..4 {
            assert!(
                (actual[axis] - expected[axis]).abs() <= 1.0e-5,
                "axis {axis}: actual={} expected={}",
                actual[axis],
                expected[axis]
            );
        }
    }

    #[test]
    fn translated_sphere_moves_values_and_gradients() {
        let field = Translate::new(
            SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 1.0,
            },
            [2.0, -1.0, 0.5],
        );
        let mut values = [0.0; 1];
        let mut gradients = [[0.0; 4]; 1];
        field.eval_points(&[[3.0, -1.0, 0.5]], &mut values);
        field.eval_gradients(&[[3.0, -1.0, 0.5]], &mut gradients);

        assert!(values[0].abs() <= 1.0e-6);
        assert_gradient_close(gradients[0], [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn translated_interval_tracks_shifted_field() {
        let field = Translate::new(
            SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 0.5,
            },
            [1.0, 0.0, 0.0],
        );
        let bounds = Aabb::new([0.5, -0.25, -0.25], [1.5, 0.25, 0.25]).expect("valid bounds");
        let interval = field.eval_interval(&bounds).expect("interval");

        assert!(interval[0] <= -0.49);
        assert!(interval[1] >= 0.0);
    }

    #[test]
    fn rigid_transform_rotates_cylinder() {
        let rotation = RigidTransform3::new(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .expect("orthonormal axes");
        let field = Transform3::new(
            CylinderField {
                center: [0.0, 0.0, 0.0],
                axis: [0.0, 0.0, 1.0],
                radius: 0.5,
                half_height: 0.75,
            },
            rotation,
        );
        let mut gradients = [[0.0; 4]; 1];
        field.eval_gradients(&[[0.0, 0.5, 0.0]], &mut gradients);

        assert!(gradients[0][0].abs() <= 1.0e-5);
        assert_gradient_close(gradients[0], [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn rigid_transform_rotates_half_space_interval() {
        let rotation = RigidTransform3::new(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
        )
        .expect("orthonormal axes");
        let field = Transform3::new(
            HalfSpaceField {
                point: [0.0, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
            },
            rotation,
        );
        let bounds = Aabb::new([-0.25, -0.25, -1.0], [0.25, 0.25, 1.0]).expect("valid bounds");
        let interval = field.eval_interval(&bounds).expect("interval");

        assert!(interval[0] <= -1.0 + 1.0e-5);
        assert!(interval[1] >= 1.0 - 1.0e-5);
    }

    #[test]
    fn uniform_scale_scales_values_without_changing_normals() {
        let field = UniformScale::new(
            SphereField {
                center: [0.0, 0.0, 0.0],
                radius: 1.0,
            },
            2.0,
        )
        .expect("positive scale");
        let mut gradients = [[0.0; 4]; 1];
        field.eval_gradients(&[[2.0, 0.0, 0.0]], &mut gradients);

        assert_gradient_close(gradients[0], [0.0, 1.0, 0.0, 0.0]);
    }
}
