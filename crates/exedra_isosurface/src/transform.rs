// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Wrappers that move a field in space or adjust its values.
//!
//! Three operations that look alike differ in what they promise:
//!
//! - [`UniformScale`] scales the geometry. Values scale with it, so distances
//!   stay distances and the support scales about the origin.
//! - [`ScaleValue`] multiplies values by a positive factor. The surface does
//!   not move; only the metric claim changes, by the same factor.
//! - [`Offset`] subtracts a distance from values, moving the surface along
//!   the field. It is a physical offset only as far as the values are
//!   distances, so its support claim depends on the metric claim.
//! - [`Shell`] hollows a shape into a skin around its surface, with the
//!   same dependence on distances.

use crate::spatial::Aabb;

use crate::capability::{Capabilities, FieldCapabilities, Metric, Support, outward};
use crate::{
    FieldError, ProvenanceField, ScalarField, SpecializableField, check_finite, check_positive,
};
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

/// Wrapper multiplying a field's values by a positive factor.
///
/// The zero set, and so the extracted surface, is unchanged. Use it to bring
/// a field's values into a distance-like range, or to weight it within a
/// blend; use [`UniformScale`] to resize the shape.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ScaleValue<F> {
    field: F,
    factor: f32,
}

/// Wrapper subtracting a distance from a field's values.
///
/// A positive distance grows the shape and a negative one shrinks it. With
/// exact distances the surface moves by exactly that distance; with a
/// Lipschitz bound `L` it moves outward by at least `distance / L`, but how
/// far beyond is not known, so a grown shape claims no support.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Offset<F> {
    field: F,
    distance: f32,
}

/// Wrapper hollowing a field into a shell `thickness` either side of its
/// surface: `|value| - thickness`.
///
/// Like [`Offset`], the shell's extent follows from the values only as far
/// as they are distances.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Shell<F> {
    field: F,
    thickness: f32,
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
    /// # Errors
    ///
    /// Returns [`FieldError`] unless `factor` is finite and positive.
    pub fn new(field: F, factor: f32) -> Result<Self, FieldError> {
        check_positive("factor", factor)?;
        Ok(Self { field, factor })
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

impl<F> ScaleValue<F> {
    /// Creates a value-scaling wrapper.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless `factor` is finite and positive; a
    /// negative factor would swap inside and outside.
    pub fn new(field: F, factor: f32) -> Result<Self, FieldError> {
        check_positive("factor", factor)?;
        Ok(Self { field, factor })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the value factor.
    #[must_use]
    pub const fn factor(&self) -> f32 {
        self.factor
    }
}

impl<F> Offset<F> {
    /// Creates an offset wrapper.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless `distance` is finite.
    pub fn new(field: F, distance: f32) -> Result<Self, FieldError> {
        check_finite("distance", [distance])?;
        Ok(Self { field, distance })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the offset distance: positive grows the shape.
    #[must_use]
    pub const fn distance(&self) -> f32 {
        self.distance
    }
}

impl<F> Shell<F> {
    /// Creates a shell wrapper.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless `thickness` is finite and positive.
    pub fn new(field: F, thickness: f32) -> Result<Self, FieldError> {
        check_positive("thickness", thickness)?;
        Ok(Self { field, thickness })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// Returns the half thickness of the shell.
    #[must_use]
    pub const fn thickness(&self) -> f32 {
        self.thickness
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
    /// The provided axes are normalized internally.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when the origin or an axis is not finite, an
    /// axis is too short to normalize, or two axes are not perpendicular
    /// within `1e-4` in cosine.
    pub fn new(
        origin: [f32; 3],
        x_axis: [f32; 3],
        y_axis: [f32; 3],
        z_axis: [f32; 3],
    ) -> Result<Self, FieldError> {
        check_finite("origin", origin)?;
        let axis = |parameter, axis: [f32; 3]| {
            check_finite(parameter, axis)?;
            normalize(axis).ok_or(FieldError::DegenerateDirection { parameter })
        };
        let x_axis = axis("x_axis", x_axis)?;
        let y_axis = axis("y_axis", y_axis)?;
        let z_axis = axis("z_axis", z_axis)?;
        if abs(dot(x_axis, y_axis)) > 1.0e-4
            || abs(dot(x_axis, z_axis)) > 1.0e-4
            || abs(dot(y_axis, z_axis)) > 1.0e-4
        {
            return Err(FieldError::NotOrthogonal { parameter: "axes" });
        }
        Ok(Self {
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

    /// A world box containing every point that evaluation maps into
    /// `local`.
    ///
    /// Evaluation maps a world point `p` to `M (p - origin)`, where the rows
    /// of `M` are the axes. The axes are orthogonal only to `1e-4`, so `M` is
    /// not quite a rotation and its transpose not quite its inverse; far from
    /// the origin that difference is large. The corners go through the true
    /// inverse in `f64`, and the box widens by the `f32` rounding of the
    /// forward map.
    fn world_bounds(&self, local: &Aabb) -> Option<Aabb> {
        let rows = [self.x_axis, self.y_axis, self.z_axis].map(|row| row.map(f64::from));
        let inverse = inverse3(rows)?;
        let origin = self.origin.map(f64::from);
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        let mut magnitude = origin.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        for corner in corners(local) {
            let corner = corner.map(f64::from);
            let world: [f64; 3] = core::array::from_fn(|i| origin[i] + dot(inverse[i], corner));
            for i in 0..3 {
                minimum[i] = minimum[i].min(world[i]);
                maximum[i] = maximum[i].max(world[i]);
            }
            magnitude = world.iter().fold(magnitude, |m, v| m.max(v.abs()));
        }
        // Forming `p - origin` and each dot product rounds a few times, each
        // by at most an ulp of the coordinates involved; mapped back through
        // the inverse, whose rows are within a few percent of unit length,
        // that is a few ulps of the largest coordinate.
        let margin = 16.0 * f64::from(f32::EPSILON) * magnitude;
        #[expect(clippy::cast_possible_truncation, reason = "outward rounding follows")]
        let narrow = |v: f64| v as f32;
        outward(
            minimum.map(|v| narrow(v - margin)),
            maximum.map(|v| narrow(v + margin)),
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
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        Capabilities {
            metric: inner.metric,
            support: inner.support.translated(self.offset),
        }
    }

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
    fn capabilities(&self) -> FieldCapabilities {
        // Values scale with the geometry, so distances stay distances.
        let inner = self.field.capabilities();
        Capabilities {
            metric: inner.metric,
            support: inner.support.scaled(self.factor),
        }
    }

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

impl<F: ScalarField> ScalarField for ScaleValue<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        let metric = match inner.metric {
            #[expect(clippy::float_cmp, reason = "only an exact identity keeps exactness")]
            Metric::ExactDistance if self.factor == 1.0 => Metric::ExactDistance,
            metric => metric.scaled(self.factor),
        };
        Capabilities {
            metric,
            support: inner.support,
        }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        self.field
            .eval_interval(bounds)
            .map(|[low, high]| [low * self.factor, high * self.factor])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        self.field.eval_points(points, out);
        for value in out {
            *value *= self.factor;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        self.field.eval_gradients(points, out);
        for row in out {
            *row = row.map(|v| v * self.factor);
        }
    }
}

impl<F: ScalarField> ScalarField for Offset<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        let distance = self.distance;
        // Subtracting a constant keeps every Lipschitz bound. Exactness
        // survives only outside the shape: inside a grown concave shape, or
        // anywhere near a shrunk one's corners, the nearest surface point is
        // no longer `distance` farther away.
        let metric = match inner.metric {
            Metric::ExactDistance if distance == 0.0 => Metric::ExactDistance,
            metric => metric
                .lipschitz()
                .map_or(Metric::Unknown, Metric::Lipschitz),
        };
        let support = if distance <= 0.0 {
            // Values below a nonpositive distance are already negative.
            inner.support.shrunk()
        } else {
            match (inner.metric, inner.support) {
                (Metric::ExactDistance, support) => support.grown(distance),
                (_, support @ (Support::Unbounded | Support::Unknown)) => support,
                // Without exact distances, values may stay below `distance`
                // arbitrarily far out, and an empty interior may gain one.
                _ => Support::Unknown,
            }
        };
        Capabilities { metric, support }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        self.field
            .eval_interval(bounds)
            .map(|[low, high]| [low - self.distance, high - self.distance])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        self.field.eval_points(points, out);
        for value in out {
            *value -= self.distance;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        self.field.eval_gradients(points, out);
        for row in out {
            row[0] -= self.distance;
        }
    }
}

impl<F: ScalarField> ScalarField for Shell<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        // The shell lies within `thickness` of the surface, which an exact
        // distance keeps within its support; folding by `|.|` keeps any
        // Lipschitz bound but not exactness.
        let support = match (inner.metric, inner.support) {
            (Metric::ExactDistance, Support::Unbounded) => Support::Unknown,
            (Metric::ExactDistance, support) => support.grown(self.thickness),
            _ => Support::Unknown,
        };
        Capabilities {
            metric: inner
                .metric
                .lipschitz()
                .map_or(Metric::Unknown, Metric::Lipschitz),
            support,
        }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let [low, high] = self.field.eval_interval(bounds)?;
        let near = if low <= 0.0 && high >= 0.0 {
            0.0
        } else {
            low.abs().min(high.abs())
        };
        Some([
            near - self.thickness,
            low.abs().max(high.abs()) - self.thickness,
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        self.field.eval_points(points, out);
        for value in out {
            *value = value.abs() - self.thickness;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        self.field.eval_gradients(points, out);
        for row in out {
            if row[0] < 0.0 {
                *row = row.map(|v| -v);
            }
            row[0] -= self.thickness;
        }
    }
}

impl<F: ScalarField> ScalarField for Transform3<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        let transform = self.transform;
        let support = inner.support.map(|local| transform.world_bounds(&local));
        Capabilities {
            metric: inner.metric,
            support,
        }
    }

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
        self.field.specialize(&scaled).map(|field| UniformScale {
            field,
            factor: self.factor,
        })
    }
}

impl<F: SpecializableField> SpecializableField for ScaleValue<F> {
    type Specialized = ScaleValue<F::Specialized>;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        self.field.specialize(bounds).map(|field| ScaleValue {
            field,
            factor: self.factor,
        })
    }
}

impl<F: SpecializableField> SpecializableField for Offset<F> {
    type Specialized = Offset<F::Specialized>;

    fn specialize(&self, bounds: &Aabb) -> Option<Self::Specialized> {
        self.field.specialize(bounds).map(|field| Offset {
            field,
            distance: self.distance,
        })
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

impl<F: ProvenanceField> ProvenanceField for ScaleValue<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.field
            .eval_interval_with_provenance(bounds)
            .map(|([low, high], provenance)| ([low * self.factor, high * self.factor], provenance))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(point)
    }
}

impl<F: ProvenanceField> ProvenanceField for Offset<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.field
            .eval_interval_with_provenance(bounds)
            .map(|([low, high], provenance)| {
                ([low - self.distance, high - self.distance], provenance)
            })
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(point)
    }
}

impl<F: ProvenanceField> ProvenanceField for Shell<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let interval = self.eval_interval(bounds)?;
        Some((interval, self.field.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(point)
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

/// The inverse of a 3x3 matrix by cofactors, or `None` when singular.
fn inverse3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let cofactor = |r: usize, c: usize| {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (c0, c1) = ((c + 1) % 3, (c + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let determinant = (0..3).map(|c| m[0][c] * cofactor(0, c)).sum::<f64>();
    if !determinant.is_normal() {
        return None;
    }
    // The inverse is the transposed cofactor matrix over the determinant.
    Some(core::array::from_fn(|r| {
        core::array::from_fn(|c| cofactor(c, r) / determinant)
    }))
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
    use crate::spatial::Aabb;

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
