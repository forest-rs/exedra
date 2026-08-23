// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Analytic reference fields and simple CSG combinators.

use exedra_spatial::Aabb;

use crate::{ProvenanceField, ScalarField};
use exedra_math::{add, dot, norm, normalize, scale, sub};

/// Analytic sphere signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SphereField {
    /// Sphere center.
    pub center: [f32; 3],
    /// Sphere radius.
    pub radius: f32,
}

/// Axis-aligned box signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoxField {
    /// Box center.
    pub center: [f32; 3],
    /// Positive half extents along each axis.
    ///
    /// [`ScalarField::eval_interval`] returns `None` when the center or half
    /// extents are non-finite, or when any half extent is not positive.
    pub half_extents: [f32; 3],
}

/// Finite cylinder signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CylinderField {
    /// Cylinder center.
    pub center: [f32; 3],
    /// Cylinder axis direction.
    pub axis: [f32; 3],
    /// Cylinder radius.
    pub radius: f32,
    /// Half height along the cylinder axis.
    pub half_height: f32,
}

/// Torus signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TorusField {
    /// Torus center.
    pub center: [f32; 3],
    /// Torus axis direction.
    pub axis: [f32; 3],
    /// Distance from the torus center to the tube center.
    pub major_radius: f32,
    /// Tube radius.
    pub minor_radius: f32,
}

/// Plane half-space signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HalfSpaceField {
    /// One point on the plane boundary.
    pub point: [f32; 3],
    /// Outward half-space normal.
    pub normal: [f32; 3],
}

/// Fixed provenance wrapper for any scalar field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TaggedField<F, P> {
    /// Wrapped field.
    pub field: F,
    /// Constant provenance reported by the wrapper.
    pub provenance: P,
}

/// Which side of a binary combinator contributed the winning value.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OperandSource {
    /// The left operand won.
    Left,
    /// The right operand won.
    Right,
}

/// Provenance payload returned by binary analytic combinators.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BinaryProvenance<P> {
    /// Winning side.
    pub source: OperandSource,
    /// Wrapped operand provenance.
    pub value: P,
    /// Whether both operands contributed in a smooth blend region.
    pub blended: bool,
}

/// Pointwise union of two scalar fields.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Union<A, B> {
    /// Left operand.
    pub left: A,
    /// Right operand.
    pub right: B,
}

/// Pointwise intersection of two scalar fields.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Intersection<A, B> {
    /// Left operand.
    pub left: A,
    /// Right operand.
    pub right: B,
}

/// Pointwise subtraction `left \ right`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Difference<A, B> {
    /// Left operand.
    pub left: A,
    /// Right operand.
    pub right: B,
}

/// Polynomial smooth union with blend radius `k`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SmoothUnion<A, B> {
    /// Left operand.
    pub left: A,
    /// Right operand.
    pub right: B,
    /// Blend radius.
    pub k: f32,
}

impl<A, B> Union<A, B> {
    /// Creates a union combinator.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A, B> Intersection<A, B> {
    /// Creates an intersection combinator.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A, B> Difference<A, B> {
    /// Creates a subtraction combinator.
    #[must_use]
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }
}

impl<A, B> SmoothUnion<A, B> {
    /// Creates a smooth union combinator.
    #[must_use]
    pub const fn new(left: A, right: B, k: f32) -> Self {
        Self { left, right, k }
    }
}

impl ScalarField for SphereField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let nearest = [
            clamp(self.center[0], bounds.min[0], bounds.max[0]),
            clamp(self.center[1], bounds.min[1], bounds.max[1]),
            clamp(self.center[2], bounds.min[2], bounds.max[2]),
        ];
        let furthest = [
            farthest_axis(self.center[0], bounds.min[0], bounds.max[0]),
            farthest_axis(self.center[1], bounds.min[1], bounds.max[1]),
            farthest_axis(self.center[2], bounds.min[2], bounds.max[2]),
        ];
        Some([
            norm(sub(nearest, self.center)) - self.radius,
            norm(sub(furthest, self.center)) - self.radius,
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            out[index] = norm(sub(*point, self.center)) - self.radius;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let delta = sub(*point, self.center);
            let distance = norm(delta);
            out[index][0] = distance - self.radius;
            out[index][1..4].copy_from_slice(&normalize_or_nan(delta));
        }
    }
}

impl ScalarField for BoxField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        exact_box_interval(self, bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            let delta = abs3(sub(*point, self.center));
            let q = sub(delta, self.half_extents);
            out[index] = box_sdf_from_q(q);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let delta = sub(*point, self.center);
            let abs_delta = abs3(delta);
            let q = sub(abs_delta, self.half_extents);
            let outside = max3(q, [0.0; 3]);
            let outside_distance = norm(outside);
            out[index][0] = box_sdf_from_q(q);
            out[index][1..4].copy_from_slice(&box_gradient(delta, q, outside, outside_distance));
        }
    }
}

impl ScalarField for CylinderField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        Some(conservative_sampled_interval(self, bounds))
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            let (distance, _) = cylinder_distance_and_gradient(*point, self);
            out[index] = distance;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let (distance, gradient) = cylinder_distance_and_gradient(*point, self);
            out[index][0] = distance;
            out[index][1..4].copy_from_slice(&gradient);
        }
    }
}

impl ScalarField for TorusField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        Some(conservative_sampled_interval(self, bounds))
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            let (distance, _) = torus_distance_and_gradient(*point, self);
            out[index] = distance;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let (distance, gradient) = torus_distance_and_gradient(*point, self);
            out[index][0] = distance;
            out[index][1..4].copy_from_slice(&gradient);
        }
    }
}

impl ScalarField for HalfSpaceField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let normal = normalize(self.normal)?;
        let mut minimum = dot(normal, sub(bounds.min, self.point));
        let mut maximum = minimum;
        for corner in corners(bounds) {
            let value = dot(normal, sub(corner, self.point));
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
        Some([minimum, maximum])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let gradient = normalize_or_nan(self.normal);
        for (index, point) in points.iter().enumerate() {
            out[index] = dot(gradient, sub(*point, self.point));
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let gradient = normalize_or_nan(self.normal);
        for (index, point) in points.iter().enumerate() {
            out[index][0] = dot(gradient, sub(*point, self.point));
            out[index][1..4].copy_from_slice(&gradient);
        }
    }
}

impl<F: ScalarField, P: Copy> ScalarField for TaggedField<F, P> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        self.field.eval_interval(bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        self.field.eval_points(points, out);
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        self.field.eval_gradients(points, out);
    }
}

impl<F: ScalarField, P: Copy> ProvenanceField for TaggedField<F, P> {
    type Provenance = P;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.field
            .eval_interval(bounds)
            .map(|interval| (interval, self.provenance))
    }

    fn point_provenance(&self, _point: [f32; 3]) -> Self::Provenance {
        self.provenance
    }
}

impl<A: ScalarField, B: ScalarField> ScalarField for Union<A, B> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let left = self.left.eval_interval(bounds)?;
        let right = self.right.eval_interval(bounds)?;
        Some([left[0].min(right[0]), left[1].min(right[1])])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        for (index, point) in points.iter().enumerate() {
            self.left
                .eval_points(core::slice::from_ref(point), &mut left_values);
            self.right
                .eval_points(core::slice::from_ref(point), &mut right_values);
            out[index] = left_values[0].min(right_values[0]);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        eval_binary_select_gradients(points, out, &self.left, &self.right, |left, right| {
            if left <= right {
                (left[0], left_branch())
            } else {
                (right[0], right_branch())
            }
        });
    }
}

impl<A: ScalarField, B: ScalarField> ScalarField for Intersection<A, B> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let left = self.left.eval_interval(bounds)?;
        let right = self.right.eval_interval(bounds)?;
        Some([left[0].max(right[0]), left[1].max(right[1])])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        for (index, point) in points.iter().enumerate() {
            self.left
                .eval_points(core::slice::from_ref(point), &mut left_values);
            self.right
                .eval_points(core::slice::from_ref(point), &mut right_values);
            out[index] = left_values[0].max(right_values[0]);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        eval_binary_select_gradients(points, out, &self.left, &self.right, |left, right| {
            if left >= right {
                (left[0], left_branch())
            } else {
                (right[0], right_branch())
            }
        });
    }
}

impl<A: ScalarField, B: ScalarField> ScalarField for Difference<A, B> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let left = self.left.eval_interval(bounds)?;
        let right = self.right.eval_interval(bounds)?;
        let inverted_right = [-right[1], -right[0]];
        Some([
            left[0].max(inverted_right[0]),
            left[1].max(inverted_right[1]),
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        for (index, point) in points.iter().enumerate() {
            self.left
                .eval_points(core::slice::from_ref(point), &mut left_values);
            self.right
                .eval_points(core::slice::from_ref(point), &mut right_values);
            out[index] = left_values[0].max(-right_values[0]);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        eval_binary_select_gradients(points, out, &self.left, &self.right, |left, right| {
            let negated_right = [-right[0], -right[1], -right[2], -right[3]];
            if left[0] >= negated_right[0] {
                (left[0], left_branch())
            } else {
                (negated_right[0], right_negated_branch())
            }
        });
    }
}

impl<A: ScalarField, B: ScalarField> ScalarField for SmoothUnion<A, B> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let left = self.left.eval_interval(bounds)?;
        let right = self.right.eval_interval(bounds)?;
        Some([
            left[0].min(right[0]) - 0.25 * self.k.max(0.0),
            left[1].min(right[1]),
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        for (index, point) in points.iter().enumerate() {
            self.left
                .eval_points(core::slice::from_ref(point), &mut left_values);
            self.right
                .eval_points(core::slice::from_ref(point), &mut right_values);
            out[index] = smooth_union_value(left_values[0], right_values[0], self.k);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut left = [[0.0_f32; 4]; 1];
        let mut right = [[0.0_f32; 4]; 1];

        for (index, point) in points.iter().enumerate() {
            self.left
                .eval_gradients(core::slice::from_ref(point), &mut left);
            self.right
                .eval_gradients(core::slice::from_ref(point), &mut right);

            let h = smooth_union_blend(left[0][0], right[0][0], self.k);
            out[index][0] = smooth_union_value(left[0][0], right[0][0], self.k);
            if h <= 0.0 {
                out[index][1..4].copy_from_slice(&right[0][1..4]);
            } else if h >= 1.0 {
                out[index][1..4].copy_from_slice(&left[0][1..4]);
            } else {
                let blended = [
                    h * left[0][1] + (1.0 - h) * right[0][1],
                    h * left[0][2] + (1.0 - h) * right[0][2],
                    h * left[0][3] + (1.0 - h) * right[0][3],
                ];
                out[index][1..4].copy_from_slice(&blended);
            }
        }
    }
}

impl<A, B, P> ProvenanceField for Union<A, B>
where
    A: ProvenanceField<Provenance = P>,
    B: ProvenanceField<Provenance = P>,
    P: Copy,
{
    type Provenance = BinaryProvenance<P>;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.eval_interval(bounds)
            .map(|interval| (interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        select_binary_provenance(point, &self.left, &self.right, |left, right| {
            if left <= right {
                (OperandSource::Left, false)
            } else {
                (OperandSource::Right, false)
            }
        })
    }
}

impl<A, B, P> ProvenanceField for Intersection<A, B>
where
    A: ProvenanceField<Provenance = P>,
    B: ProvenanceField<Provenance = P>,
    P: Copy,
{
    type Provenance = BinaryProvenance<P>;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.eval_interval(bounds)
            .map(|interval| (interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        select_binary_provenance(point, &self.left, &self.right, |left, right| {
            if left >= right {
                (OperandSource::Left, false)
            } else {
                (OperandSource::Right, false)
            }
        })
    }
}

impl<A, B, P> ProvenanceField for Difference<A, B>
where
    A: ProvenanceField<Provenance = P>,
    B: ProvenanceField<Provenance = P>,
    P: Copy,
{
    type Provenance = BinaryProvenance<P>;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.eval_interval(bounds)
            .map(|interval| (interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        self.left
            .eval_points(core::slice::from_ref(&point), &mut left_values);
        self.right
            .eval_points(core::slice::from_ref(&point), &mut right_values);
        if left_values[0] >= -right_values[0] {
            BinaryProvenance {
                source: OperandSource::Left,
                value: self.left.point_provenance(point),
                blended: false,
            }
        } else {
            BinaryProvenance {
                source: OperandSource::Right,
                value: self.right.point_provenance(point),
                blended: false,
            }
        }
    }
}

impl<A, B, P> ProvenanceField for SmoothUnion<A, B>
where
    A: ProvenanceField<Provenance = P>,
    B: ProvenanceField<Provenance = P>,
    P: Copy,
{
    type Provenance = BinaryProvenance<P>;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        self.eval_interval(bounds)
            .map(|interval| (interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        let mut left_values = [0.0_f32; 1];
        let mut right_values = [0.0_f32; 1];
        self.left
            .eval_points(core::slice::from_ref(&point), &mut left_values);
        self.right
            .eval_points(core::slice::from_ref(&point), &mut right_values);
        let h = smooth_union_blend(left_values[0], right_values[0], self.k);
        if h >= 0.5 {
            BinaryProvenance {
                source: OperandSource::Left,
                value: self.left.point_provenance(point),
                blended: (0.0..1.0).contains(&h),
            }
        } else {
            BinaryProvenance {
                source: OperandSource::Right,
                value: self.right.point_provenance(point),
                blended: (0.0..1.0).contains(&h),
            }
        }
    }
}

fn eval_binary_select_gradients<A, B, S>(
    points: &[[f32; 3]],
    out: &mut [[f32; 4]],
    left_field: &A,
    right_field: &B,
    selector: S,
) where
    A: ScalarField,
    B: ScalarField,
    S: Fn([f32; 4], [f32; 4]) -> (f32, Branch),
{
    assert_same_len(points.len(), out.len(), "point/gradient");
    let mut left = [[0.0_f32; 4]; 1];
    let mut right = [[0.0_f32; 4]; 1];

    for (index, point) in points.iter().enumerate() {
        left_field.eval_gradients(core::slice::from_ref(point), &mut left);
        right_field.eval_gradients(core::slice::from_ref(point), &mut right);
        let (value, branch) = selector(left[0], right[0]);
        out[index][0] = value;
        match branch {
            Branch::Left => out[index][1..4].copy_from_slice(&left[0][1..4]),
            Branch::Right => out[index][1..4].copy_from_slice(&right[0][1..4]),
            Branch::RightNegated => {
                out[index][1] = -right[0][1];
                out[index][2] = -right[0][2];
                out[index][3] = -right[0][3];
            }
        }
    }
}

fn select_binary_provenance<A, B, P, F>(
    point: [f32; 3],
    left: &A,
    right: &B,
    selector: F,
) -> BinaryProvenance<P>
where
    A: ProvenanceField<Provenance = P>,
    B: ProvenanceField<Provenance = P>,
    P: Copy,
    F: Fn(f32, f32) -> (OperandSource, bool),
{
    let mut left_values = [0.0_f32; 1];
    let mut right_values = [0.0_f32; 1];
    left.eval_points(core::slice::from_ref(&point), &mut left_values);
    right.eval_points(core::slice::from_ref(&point), &mut right_values);
    let (source, blended) = selector(left_values[0], right_values[0]);
    match source {
        OperandSource::Left => BinaryProvenance {
            source,
            value: left.point_provenance(point),
            blended,
        },
        OperandSource::Right => BinaryProvenance {
            source,
            value: right.point_provenance(point),
            blended,
        },
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Branch {
    Left,
    Right,
    RightNegated,
}

fn left_branch() -> Branch {
    Branch::Left
}

fn right_branch() -> Branch {
    Branch::Right
}

fn right_negated_branch() -> Branch {
    Branch::RightNegated
}

fn box_gradient(
    delta: [f32; 3],
    q: [f32; 3],
    outside: [f32; 3],
    outside_distance: f32,
) -> [f32; 3] {
    if outside_distance > 0.0 {
        return [
            signed_component(delta[0], outside[0], outside_distance),
            signed_component(delta[1], outside[1], outside_distance),
            signed_component(delta[2], outside[2], outside_distance),
        ];
    }

    let max_q = q[0].max(q[1].max(q[2]));
    let mut axis = None;
    for (index, component) in q.into_iter().enumerate() {
        if (component - max_q).abs() <= 1.0e-6 {
            if axis.is_some() {
                return [f32::NAN; 3];
            }
            axis = Some(index);
        }
    }

    let Some(axis) = axis else {
        return [f32::NAN; 3];
    };
    let Some(sign) = sign_nonzero(delta[axis]) else {
        return [f32::NAN; 3];
    };
    let mut gradient = [0.0; 3];
    gradient[axis] = sign;
    gradient
}

fn cylinder_distance_and_gradient(point: [f32; 3], cylinder: &CylinderField) -> (f32, [f32; 3]) {
    let Some(axis) = normalize(cylinder.axis) else {
        return (f32::NAN, [f32::NAN; 3]);
    };
    let delta = sub(point, cylinder.center);
    let axial = dot(delta, axis);
    let axial_component = scale(axis, axial);
    let radial = sub(delta, axial_component);
    let radial_length = norm(radial);
    let q = [
        radial_length - cylinder.radius,
        abs(axial) - cylinder.half_height,
    ];
    let outside = [q[0].max(0.0), q[1].max(0.0)];
    let outside_distance = length2(outside);
    let distance = outside_distance + q[0].max(q[1]).min(0.0);

    if outside_distance > 0.0 {
        let radial_dir = normalize_or_nan(radial);
        let axial_dir = scale(axis, sign_nonzero(axial).unwrap_or(f32::NAN));
        let gradient = add(
            scale(radial_dir, outside[0] / outside_distance),
            scale(axial_dir, outside[1] / outside_distance),
        );
        return (distance, gradient);
    }

    if (q[0] - q[1]).abs() <= 1.0e-6 {
        return (distance, [f32::NAN; 3]);
    }
    if q[0] > q[1] {
        return (distance, normalize_or_nan(radial));
    }
    (
        distance,
        scale(axis, sign_nonzero(axial).unwrap_or(f32::NAN)),
    )
}

fn torus_distance_and_gradient(point: [f32; 3], torus: &TorusField) -> (f32, [f32; 3]) {
    let Some(axis) = normalize(torus.axis) else {
        return (f32::NAN, [f32::NAN; 3]);
    };
    let delta = sub(point, torus.center);
    let axial = dot(delta, axis);
    let radial = sub(delta, scale(axis, axial));
    let radial_length = norm(radial);
    let q = [radial_length - torus.major_radius, axial];
    let q_length = length2(q);
    let distance = q_length - torus.minor_radius;

    if q_length == 0.0 || radial_length == 0.0 {
        return (distance, [f32::NAN; 3]);
    }

    let radial_dir = scale(radial, 1.0 / radial_length);
    let gradient = add(
        scale(radial_dir, q[0] / q_length),
        scale(axis, q[1] / q_length),
    );
    (distance, gradient)
}

fn exact_box_interval(field: &BoxField, bounds: &Aabb) -> Option<[f32; 2]> {
    if !field
        .center
        .iter()
        .chain(&field.half_extents)
        .chain(&bounds.min)
        .chain(&bounds.max)
        .all(|value| value.is_finite())
        || field.half_extents.iter().any(|extent| *extent <= 0.0)
        || (0..3).any(|axis| bounds.min[axis] > bounds.max[axis])
    {
        return None;
    }

    let mut nearest_q = [0.0; 3];
    let mut furthest_q = [0.0; 3];
    for axis in 0..3 {
        let center = field.center[axis];
        let nearest_abs = if center < bounds.min[axis] {
            bounds.min[axis] - center
        } else if center > bounds.max[axis] {
            center - bounds.max[axis]
        } else {
            0.0
        };
        let furthest_abs = abs(bounds.min[axis] - center).max(abs(bounds.max[axis] - center));
        nearest_q[axis] = nearest_abs - field.half_extents[axis];
        furthest_q[axis] = furthest_abs - field.half_extents[axis];
    }

    let minimum = box_sdf_from_q(nearest_q);
    let maximum = box_sdf_from_q(furthest_q);
    if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
        return None;
    }
    Some([minimum.next_down(), maximum.next_up()])
}

fn box_sdf_from_q(q: [f32; 3]) -> f32 {
    norm(max3(q, [0.0; 3])) + q[0].max(q[1].max(q[2])).min(0.0)
}

fn conservative_sampled_interval<F: ScalarField>(field: &F, bounds: &Aabb) -> [f32; 2] {
    let mut samples = sample_points(bounds);
    let mut values = [0.0_f32; 9];
    field.eval_points(&samples, &mut values);

    let inflation = norm(sub(bounds.max, bounds.min));
    let mut minimum = values[0];
    let mut maximum = values[0];
    for value in values {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }

    // Reference fields are intended as test oracles first. Inflating the
    // sampled interval keeps the bound conservative even where the exact range
    // is tedious to derive analytically.
    samples.fill([0.0; 3]);
    [minimum - inflation, maximum + inflation]
}

fn sample_points(bounds: &Aabb) -> [[f32; 3]; 9] {
    let corners = corners(bounds);
    [
        corners[0],
        corners[1],
        corners[2],
        corners[3],
        corners[4],
        corners[5],
        corners[6],
        corners[7],
        bounds.center(),
    ]
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

fn smooth_union_value(left: f32, right: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return left.min(right);
    }
    let h = smooth_union_blend(left, right, k);
    right * (1.0 - h) + left * h - k * h * (1.0 - h)
}

fn smooth_union_blend(left: f32, right: f32, k: f32) -> f32 {
    if k <= 0.0 {
        if left <= right { 1.0 } else { 0.0 }
    } else {
        clamp(0.5 + 0.5 * (right - left) / k, 0.0, 1.0)
    }
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
}

fn signed_component(delta: f32, numerator: f32, denominator: f32) -> f32 {
    if numerator == 0.0 {
        0.0
    } else {
        sign_nonzero(delta).unwrap_or(f32::NAN) * numerator / denominator
    }
}

fn sign_nonzero(value: f32) -> Option<f32> {
    if value > 0.0 {
        Some(1.0)
    } else if value < 0.0 {
        Some(-1.0)
    } else {
        None
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max)
}

fn farthest_axis(center: f32, min: f32, max: f32) -> f32 {
    if (center - min).abs() >= (max - center).abs() {
        min
    } else {
        max
    }
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

fn abs3(vector: [f32; 3]) -> [f32; 3] {
    [abs(vector[0]), abs(vector[1]), abs(vector[2])]
}

fn max3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

fn length2(vector: [f32; 2]) -> f32 {
    sqrt(vector[0] * vector[0] + vector[1] * vector[1])
}

fn normalize_or_nan(vector: [f32; 3]) -> [f32; 3] {
    normalize(vector).unwrap_or([f32::NAN; 3])
}

#[cfg(feature = "std")]
fn sqrt(value: f32) -> f32 {
    value.sqrt()
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}

#[cfg(test)]
mod tests {
    use super::{
        BinaryProvenance, BoxField, CylinderField, Difference, HalfSpaceField, Intersection,
        OperandSource, SmoothUnion, SphereField, TaggedField, TorusField, Union, box_sdf_from_q,
    };
    use crate::transform::{RigidTransform3, Transform3, Translate, UniformScale};
    use crate::{ProvenanceField, ScalarField};
    use exedra_spatial::Aabb;

    fn finite_difference_gradient<F: ScalarField>(field: &F, point: [f32; 3]) -> [f32; 3] {
        let epsilon = 1.0e-3;
        let mut output = [0.0_f32; 2];
        let mut gradient = [0.0; 3];
        for axis in 0..3 {
            let mut plus = point;
            let mut minus = point;
            plus[axis] += epsilon;
            minus[axis] -= epsilon;
            field.eval_points(&[plus, minus], &mut output);
            gradient[axis] = (output[0] - output[1]) / (2.0 * epsilon);
        }
        gradient
    }

    fn assert_gradient_matches_finite_difference<F: ScalarField>(field: &F, point: [f32; 3]) {
        let expected = finite_difference_gradient(field, point);
        let mut out = [[0.0_f32; 4]; 1];
        field.eval_gradients(&[point], &mut out);
        for axis in 0..3 {
            assert!(
                (out[0][axis + 1] - expected[axis]).abs() <= 2.0e-2,
                "axis {axis}: analytic={}, finite={}",
                out[0][axis + 1],
                expected[axis]
            );
        }
    }

    fn assert_interval_covers_bruteforce<F: ScalarField>(field: &F, bounds: &Aabb) {
        let interval = field.eval_interval(bounds).expect("interval should exist");
        let mut values = [0.0_f32; 1];
        for x in 0..=8 {
            for y in 0..=8 {
                for z in 0..=8 {
                    let point = [
                        bounds.min[0] + (bounds.max[0] - bounds.min[0]) * (x as f32 / 8.0),
                        bounds.min[1] + (bounds.max[1] - bounds.min[1]) * (y as f32 / 8.0),
                        bounds.min[2] + (bounds.max[2] - bounds.min[2]) * (z as f32 / 8.0),
                    ];
                    field.eval_points(&[point], &mut values);
                    assert!(
                        values[0] >= interval[0] - 1.0e-5,
                        "{point:?} not above lower bound"
                    );
                    assert!(
                        values[0] <= interval[1] + 1.0e-5,
                        "{point:?} not below upper bound"
                    );
                }
            }
        }
    }

    #[test]
    fn box_field_interval_is_conservative() {
        let field = BoxField {
            center: [0.1, -0.2, 0.3],
            half_extents: [0.75, 0.5, 0.25],
        };
        let bounds = Aabb::new([-1.0, -1.0, -1.0], [1.0, 0.75, 0.9]).expect("valid bounds");

        assert_interval_covers_bruteforce(&field, &bounds);
    }

    #[test]
    fn box_field_interval_is_exact_then_expanded_by_one_float() {
        let field = BoxField {
            center: [0.0; 3],
            half_extents: [1.0; 3],
        };
        let outside = Aabb::new([2.0, 0.0, 0.0], [2.0, 0.0, 0.0]).expect("point bounds");
        let inside = Aabb::new([0.0; 3], [0.0; 3]).expect("point bounds");
        let surface = Aabb::new([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]).expect("point bounds");

        assert_eq!(
            field.eval_interval(&outside),
            Some([
                f32::from_bits(1.0_f32.to_bits() - 1),
                f32::from_bits(1.0_f32.to_bits() + 1),
            ])
        );
        assert_eq!(
            field.eval_interval(&inside),
            Some([
                f32::from_bits((-1.0_f32).to_bits() + 1),
                f32::from_bits((-1.0_f32).to_bits() - 1),
            ])
        );
        assert_eq!(
            field.eval_interval(&surface),
            Some([f32::from_bits(0x8000_0001), f32::from_bits(0x0000_0001),])
        );
    }

    #[test]
    fn box_field_general_interval_extrema_are_attained_before_expansion() {
        let field = BoxField {
            center: [0.37, -0.22, 0.41],
            half_extents: [0.83, 0.61, 0.47],
        };
        let bounds = Aabb::new([-1.3, 0.1, -0.7], [1.1, 1.4, 0.2]).expect("query bounds");
        let nearest = [field.center[0], bounds.min[1], bounds.max[2]];
        let farthest = [bounds.min[0], bounds.max[1], bounds.min[2]];
        let nearest_q = [
            (nearest[0] - field.center[0]).abs() - field.half_extents[0],
            (nearest[1] - field.center[1]).abs() - field.half_extents[1],
            (nearest[2] - field.center[2]).abs() - field.half_extents[2],
        ];
        let farthest_q = [
            (farthest[0] - field.center[0]).abs() - field.half_extents[0],
            (farthest[1] - field.center[1]).abs() - field.half_extents[1],
            (farthest[2] - field.center[2]).abs() - field.half_extents[2],
        ];
        let raw = [box_sdf_from_q(nearest_q), box_sdf_from_q(farthest_q)];
        let mut evaluated = [0.0; 2];
        field.eval_points(&[nearest, farthest], &mut evaluated);

        assert_eq!(evaluated, raw);
        assert_eq!(
            field.eval_interval(&bounds),
            Some([raw[0].next_down(), raw[1].next_up()])
        );
    }

    #[test]
    fn box_field_interval_rejects_invalid_primitive_and_query_inputs() {
        let valid = BoxField {
            center: [0.0; 3],
            half_extents: [1.0; 3],
        };
        let bounds = Aabb::new([-1.0; 3], [1.0; 3]).expect("valid bounds");
        for invalid in [
            BoxField {
                center: [f32::NAN, 0.0, 0.0],
                ..valid
            },
            BoxField {
                half_extents: [0.0, 1.0, 1.0],
                ..valid
            },
            BoxField {
                half_extents: [-1.0, 1.0, 1.0],
                ..valid
            },
            BoxField {
                half_extents: [f32::INFINITY, 1.0, 1.0],
                ..valid
            },
        ] {
            assert_eq!(invalid.eval_interval(&bounds), None);
        }

        for invalid in [
            Aabb {
                min: [f32::NAN, -1.0, -1.0],
                max: [1.0; 3],
            },
            Aabb {
                min: [2.0, -1.0, -1.0],
                max: [1.0; 3],
            },
            Aabb {
                min: [-1.0; 3],
                max: [f32::INFINITY, 1.0, 1.0],
            },
        ] {
            assert_eq!(valid.eval_interval(&invalid), None);
        }

        let overflow = Aabb::new(
            [f32::MAX, f32::MAX, f32::MAX],
            [f32::MAX, f32::MAX, f32::MAX],
        )
        .expect("ordered finite bounds");
        assert_eq!(valid.eval_interval(&overflow), None);
    }

    #[test]
    fn exact_box_intervals_compose_through_transforms_and_csg() {
        let left = BoxField {
            center: [-0.37, 0.11, -0.23],
            half_extents: [0.83, 0.61, 0.47],
        };
        let right = BoxField {
            center: [0.41, -0.29, 0.19],
            half_extents: [0.59, 0.73, 0.31],
        };
        let bounds = Aabb::new([-1.3, -1.1, -0.9], [1.5, 1.2, 1.4]).expect("query bounds");

        assert_interval_covers_bruteforce(
            &Translate::new(left, [2.3, -1.7, 0.9]),
            &Aabb {
                min: [
                    bounds.min[0] + 2.3,
                    bounds.min[1] - 1.7,
                    bounds.min[2] + 0.9,
                ],
                max: [
                    bounds.max[0] + 2.3,
                    bounds.max[1] - 1.7,
                    bounds.max[2] + 0.9,
                ],
            },
        );
        let scaled = UniformScale::new(left, 3.25).expect("positive scale");
        assert_interval_covers_bruteforce(
            &scaled,
            &Aabb::new(
                bounds.min.map(|value| value * 3.25),
                bounds.max.map(|value| value * 3.25),
            )
            .expect("scaled bounds"),
        );
        for (scale, offset) in [
            (1.0e-3, [0.0; 3]),
            (1.0, [13.0, -7.0, 5.0]),
            (1.0e3, [128.0, -64.0, 32.0]),
        ] {
            let wrapped = Translate::new(
                UniformScale::new(left, scale).expect("positive scale"),
                offset,
            );
            let transform = |point: [f32; 3]| {
                [
                    point[0] * scale + offset[0],
                    point[1] * scale + offset[1],
                    point[2] * scale + offset[2],
                ]
            };
            let wrapped_bounds =
                Aabb::new(transform(bounds.min), transform(bounds.max)).expect("wrapped bounds");
            assert_interval_covers_bruteforce(&wrapped, &wrapped_bounds);
        }
        let diagonal = core::f32::consts::FRAC_1_SQRT_2;
        let rotation = RigidTransform3::new(
            [1.7, -0.8, 0.6],
            [diagonal, diagonal, 0.0],
            [-diagonal, diagonal, 0.0],
            [0.0, 0.0, 1.0],
        )
        .expect("rigid transform");
        assert_interval_covers_bruteforce(
            &Transform3::new(left, rotation),
            &Aabb::new([0.2, -2.1, -0.3], [3.0, 0.4, 1.9]).expect("world bounds"),
        );

        assert_interval_covers_bruteforce(&Union::new(left, right), &bounds);
        assert_interval_covers_bruteforce(&Intersection::new(left, right), &bounds);
        assert_interval_covers_bruteforce(&Difference::new(left, right), &bounds);

        let transformed_left = Translate::new(left, [0.23, -0.17, 0.31]);
        let transformed_right = UniformScale::new(right, 1.375).expect("positive composite scale");
        let composite_bounds =
            Aabb::new([-1.7, -1.5, -1.3], [1.9, 1.6, 1.8]).expect("composite bounds");
        assert_interval_covers_bruteforce(
            &Union::new(transformed_left, transformed_right),
            &composite_bounds,
        );
        assert_interval_covers_bruteforce(
            &Intersection::new(transformed_left, transformed_right),
            &composite_bounds,
        );
        assert_interval_covers_bruteforce(
            &Difference::new(transformed_left, transformed_right),
            &composite_bounds,
        );
    }

    #[test]
    fn cylinder_field_interval_is_conservative() {
        let field = CylinderField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            radius: 0.8,
            half_height: 0.6,
        };
        let bounds = Aabb::new([-1.2, -1.1, -0.9], [1.0, 1.1, 1.0]).expect("valid bounds");

        assert_interval_covers_bruteforce(&field, &bounds);
    }

    #[test]
    fn torus_field_interval_is_conservative() {
        let field = TorusField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            major_radius: 1.0,
            minor_radius: 0.25,
        };
        let bounds = Aabb::new([-1.5, -0.7, -1.5], [1.5, 0.7, 1.5]).expect("valid bounds");

        assert_interval_covers_bruteforce(&field, &bounds);
    }

    #[test]
    fn primitive_gradients_match_finite_difference() {
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let box_field = BoxField {
            center: [0.0, 0.0, 0.0],
            half_extents: [0.5, 0.75, 0.25],
        };
        let cylinder = CylinderField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            radius: 0.75,
            half_height: 0.8,
        };
        let torus = TorusField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            major_radius: 1.0,
            minor_radius: 0.2,
        };

        assert_gradient_matches_finite_difference(&sphere, [0.8, 0.2, -0.1]);
        assert_gradient_matches_finite_difference(&box_field, [0.9, 0.1, 0.2]);
        assert_gradient_matches_finite_difference(&cylinder, [0.6, 0.2, 0.4]);
        assert_gradient_matches_finite_difference(&torus, [1.1, 0.12, 0.25]);
    }

    #[test]
    fn half_space_gradient_is_constant() {
        let field = HalfSpaceField {
            point: [0.0, 0.5, 0.0],
            normal: [0.0, 2.0, 0.0],
        };
        let mut out = [[0.0_f32; 4]; 1];
        field.eval_gradients(&[[0.0, 1.0, 0.0]], &mut out);

        assert!((out[0][0] - 0.5).abs() <= 1.0e-6);
        assert_eq!(out[0][1], 0.0);
        assert_eq!(out[0][2], 1.0);
        assert_eq!(out[0][3], 0.0);
    }

    #[test]
    fn union_and_difference_choose_expected_provenance() {
        let left = TaggedField {
            field: SphereField {
                center: [-0.4, 0.0, 0.0],
                radius: 0.75,
            },
            provenance: 7_u8,
        };
        let right = TaggedField {
            field: SphereField {
                center: [0.4, 0.0, 0.0],
                radius: 0.75,
            },
            provenance: 11_u8,
        };

        let union = Union::new(left, right);
        let difference = Difference::new(left, right);

        let left_provenance = union.point_provenance([-0.8, 0.0, 0.0]);
        assert_eq!(
            left_provenance,
            BinaryProvenance {
                source: OperandSource::Left,
                value: 7,
                blended: false,
            }
        );

        let difference_provenance = difference.point_provenance([0.7, 0.0, 0.0]);
        assert_eq!(difference_provenance.source, OperandSource::Right);
        assert_eq!(difference_provenance.value, 11);
    }

    #[test]
    fn smooth_union_reports_blended_provenance() {
        let left = TaggedField {
            field: SphereField {
                center: [-0.25, 0.0, 0.0],
                radius: 0.6,
            },
            provenance: 1_u8,
        };
        let right = TaggedField {
            field: SphereField {
                center: [0.25, 0.0, 0.0],
                radius: 0.6,
            },
            provenance: 2_u8,
        };
        let smooth = SmoothUnion::new(left, right, 0.5);

        let provenance = smooth.point_provenance([0.0, 0.0, 0.0]);
        assert!(provenance.blended);
    }

    #[test]
    fn smooth_union_gradient_matches_finite_difference() {
        let field = SmoothUnion::new(
            SphereField {
                center: [-0.25, 0.0, 0.0],
                radius: 0.6,
            },
            SphereField {
                center: [0.25, 0.0, 0.0],
                radius: 0.6,
            },
            0.5,
        );
        assert_gradient_matches_finite_difference(&field, [0.1, 0.15, 0.0]);
    }
}
