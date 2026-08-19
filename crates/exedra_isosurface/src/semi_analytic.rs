// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in analytic surface projection for known implicit primitives.
//!
//! This module owns primitive identity and deterministic, closed-form
//! projection onto supported primitive boundaries. It does not change
//! [`crate::ScalarField`] semantics, solve arbitrary implicit intersections,
//! or guarantee manifold extraction.

extern crate alloc;

use alloc::{boxed::Box, vec, vec::Vec};

use exedra_spatial::Aabb;

use crate::ScalarField;
use crate::analytic::{BoxField, CylinderField, Difference, Intersection, TaggedField, Union};
use crate::transform::{RigidTransform3, Transform3, Translate, UniformScale};

/// Geometric classification of a semi-analytic projection.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SemiAnalyticFeature {
    /// A smooth surface patch or planar face interior.
    Surface,
    /// A sharp intrinsic primitive edge.
    Edge,
    /// An intrinsic primitive corner.
    Corner,
    /// A transverse intersection between two primitive surfaces.
    IntersectionCurve,
}

/// One deterministic projection onto a known analytic primitive.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SemiAnalyticProjection {
    /// Projected world-space position.
    pub position: [f32; 3],
    /// Stable primitive identity supplied by the field.
    pub primitive: u32,
    /// Geometric feature reached by the projection.
    pub feature: SemiAnalyticFeature,
}

/// Result of attempting to project one dual-contour cell vertex.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SemiAnalyticProjectionOutcome {
    /// Projection succeeded.
    Projected(SemiAnalyticProjection),
    /// The primitive combination has no supported exact feature solver.
    Unsupported,
    /// More than one distinct feature component crosses the cell.
    Ambiguous,
    /// The relevant primitive surfaces are tangent rather than transverse.
    Tangent,
    /// The relevant primitive surface patches are coincident.
    Coincident,
    /// The nearest otherwise-valid feature exceeds the displacement budget.
    OverBudget,
    /// Primitive parameters or the resulting candidate are invalid.
    Invalid,
}

/// An oriented analytic box carried by a semi-analytic field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AnalyticBox {
    /// Stable primitive identity.
    pub primitive: u32,
    /// Box center in world space.
    pub center: [f32; 3],
    /// Orthonormal local axes in world space.
    ///
    /// Fields are public for lightweight transport. Projection validates
    /// finiteness and orthonormality and returns `None` for invalid axes. The
    /// first box/cylinder pair-feature solver is narrower than primitive
    /// projection: it requires this exact identity frame, not a signed or
    /// permuted coordinate frame.
    pub axes: [[f32; 3]; 3],
    /// Positive half extents along `axes`.
    pub half_extents: [f32; 3],
}

/// An analytic finite cylinder carried by a semi-analytic field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AnalyticCylinder {
    /// Stable primitive identity.
    pub primitive: u32,
    /// Cylinder center in world space.
    pub center: [f32; 3],
    /// Cylinder axis direction. Projection normalizes and canonicalizes it.
    pub axis: [f32; 3],
    /// Positive cylinder radius.
    pub radius: f32,
    /// Positive half-height along `axis`.
    pub half_height: f32,
}

/// Supported analytic leaf geometry.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum AnalyticPrimitive {
    /// Oriented box geometry.
    Box(AnalyticBox),
    /// Finite-cylinder geometry.
    Cylinder(AnalyticCylinder),
}

impl AnalyticPrimitive {
    /// Returns the stable primitive identity.
    #[must_use]
    pub const fn primitive(self) -> u32 {
        match self {
            Self::Box(value) => value.primitive,
            Self::Cylinder(value) => value.primitive,
        }
    }

    /// Projects `point` onto the nearest supported primitive boundary.
    #[must_use]
    pub fn project(self, point: [f32; 3]) -> Option<SemiAnalyticProjection> {
        match self {
            Self::Box(value) => project_box(value, point),
            Self::Cylinder(value) => project_cylinder(value, point),
        }
    }
}

/// Optional extraction capability for fields backed by known analytic
/// primitives.
///
/// This trait is deliberately separate from [`ScalarField`]. Existing
/// black-box fields and extraction entry points do not need to implement it.
/// Implementations must use deterministic candidate ordering and return
/// `None` when their geometry is invalid or unsupported.
pub trait SemiAnalyticField: ScalarField {
    /// Projects one candidate dual vertex associated with `cell`.
    ///
    /// Primitive leaves normally project to their nearest boundary. CSG
    /// implementations may additionally use `cell` to select an exact feature
    /// curve. Primitive projection may return a finite boundary point outside
    /// `cell`; extraction callers must reject or explicitly budget such a
    /// displacement before mutating topology.
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection>;

    /// Attempts projection while retaining a typed fallback reason.
    ///
    /// Primitive implementations normally use this default mapping. Composite
    /// fields override it when they can distinguish unsupported, ambiguous,
    /// tangent, coincident, and over-budget feature cells.
    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        self.project_cell_vertex(point, cell).map_or(
            SemiAnalyticProjectionOutcome::Invalid,
            SemiAnalyticProjectionOutcome::Projected,
        )
    }

    /// Returns the primitive that dominates the field at `point`.
    fn primitive_at(&self, point: [f32; 3]) -> u32;

    /// Returns leaf geometry when this field represents one primitive.
    ///
    /// Composite fields return `None`. This extension point allows supported
    /// binary CSG implementations to recognize exact primitive-pair features
    /// without downcasting.
    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        None
    }
}

impl<F: SemiAnalyticField + ?Sized> SemiAnalyticField for &F {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        (**self).project_cell_vertex(point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        (**self).primitive_at(point)
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        (**self).project_cell_vertex_detailed(point, cell)
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        (**self).leaf_primitive()
    }
}

impl<F: SemiAnalyticField + ?Sized> SemiAnalyticField for Box<F> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        (**self).project_cell_vertex(point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        (**self).primitive_at(point)
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        (**self).project_cell_vertex_detailed(point, cell)
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        (**self).leaf_primitive()
    }
}

impl SemiAnalyticField for TaggedField<BoxField, u32> {
    fn project_cell_vertex(&self, point: [f32; 3], _cell: &Aabb) -> Option<SemiAnalyticProjection> {
        self.leaf_primitive()?.project(point)
    }

    fn primitive_at(&self, _point: [f32; 3]) -> u32 {
        self.provenance
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        Some(AnalyticPrimitive::Box(AnalyticBox {
            primitive: self.provenance,
            center: self.field.center,
            axes: identity_axes(),
            half_extents: self.field.half_extents,
        }))
    }
}

impl SemiAnalyticField for TaggedField<CylinderField, u32> {
    fn project_cell_vertex(&self, point: [f32; 3], _cell: &Aabb) -> Option<SemiAnalyticProjection> {
        self.leaf_primitive()?.project(point)
    }

    fn primitive_at(&self, _point: [f32; 3]) -> u32 {
        self.provenance
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        Some(AnalyticPrimitive::Cylinder(AnalyticCylinder {
            primitive: self.provenance,
            center: self.field.center,
            axis: self.field.axis,
            radius: self.field.radius,
            half_height: self.field.half_height,
        }))
    }
}

impl<F: SemiAnalyticField> SemiAnalyticField for Translate<F> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        let offset = self.offset();
        let local_cell = Aabb::new(sub3(cell.min, offset), sub3(cell.max, offset))?;
        map_projection_position(
            self.field()
                .project_cell_vertex(sub3(point, offset), &local_cell),
            |position| add3(position, offset),
        )
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        self.field().primitive_at(sub3(point, self.offset()))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        let offset = self.offset();
        let Some(local_cell) = Aabb::new(sub3(cell.min, offset), sub3(cell.max, offset)) else {
            return SemiAnalyticProjectionOutcome::Invalid;
        };
        map_outcome_position(
            self.field()
                .project_cell_vertex_detailed(sub3(point, offset), &local_cell),
            |position| add3(position, offset),
        )
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        translate_primitive(self.field().leaf_primitive()?, self.offset())
    }
}

impl<F: SemiAnalyticField> SemiAnalyticField for UniformScale<F> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        let factor = self.factor();
        let inv = factor.recip();
        let local_cell = Aabb::new(mul3(cell.min, inv), mul3(cell.max, inv))?;
        map_projection_position(
            self.field()
                .project_cell_vertex(mul3(point, inv), &local_cell),
            |position| mul3(position, factor),
        )
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        self.field()
            .primitive_at(mul3(point, self.factor().recip()))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        let factor = self.factor();
        let inv = factor.recip();
        let Some(local_cell) = Aabb::new(mul3(cell.min, inv), mul3(cell.max, inv)) else {
            return SemiAnalyticProjectionOutcome::Invalid;
        };
        map_outcome_position(
            self.field()
                .project_cell_vertex_detailed(mul3(point, inv), &local_cell),
            |position| mul3(position, factor),
        )
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        scale_primitive(self.field().leaf_primitive()?, self.factor())
    }
}

impl<F: SemiAnalyticField> SemiAnalyticField for Transform3<F> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        let transform = self.transform();
        let local_cell = transform_bounds(cell, transform)?;
        map_projection_position(
            self.field()
                .project_cell_vertex(transform.world_to_local_point(point), &local_cell),
            |position| local_to_world_point(transform, position),
        )
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        self.field()
            .primitive_at(self.transform().world_to_local_point(point))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        let transform = self.transform();
        let Some(local_cell) = transform_bounds(cell, transform) else {
            return SemiAnalyticProjectionOutcome::Invalid;
        };
        map_outcome_position(
            self.field()
                .project_cell_vertex_detailed(transform.world_to_local_point(point), &local_cell),
            |position| local_to_world_point(transform, position),
        )
    }

    fn leaf_primitive(&self) -> Option<AnalyticPrimitive> {
        transform_primitive(self.field().leaf_primitive()?, self.transform())
    }
}

impl<A: SemiAnalyticField, B: SemiAnalyticField> SemiAnalyticField for Union<A, B> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        projected(csg_projection(
            CsgKind::Union,
            &self.left,
            &self.right,
            point,
            cell,
        ))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        csg_projection(CsgKind::Union, &self.left, &self.right, point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        csg_primitive_at(CsgKind::Union, &self.left, &self.right, point)
    }
}

impl<A: SemiAnalyticField, B: SemiAnalyticField> SemiAnalyticField for Intersection<A, B> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        projected(csg_projection(
            CsgKind::Intersection,
            &self.left,
            &self.right,
            point,
            cell,
        ))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        csg_projection(CsgKind::Intersection, &self.left, &self.right, point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        csg_primitive_at(CsgKind::Intersection, &self.left, &self.right, point)
    }
}

impl<A: SemiAnalyticField, B: SemiAnalyticField> SemiAnalyticField for Difference<A, B> {
    fn project_cell_vertex(&self, point: [f32; 3], cell: &Aabb) -> Option<SemiAnalyticProjection> {
        projected(csg_projection(
            CsgKind::Difference,
            &self.left,
            &self.right,
            point,
            cell,
        ))
    }

    fn project_cell_vertex_detailed(
        &self,
        point: [f32; 3],
        cell: &Aabb,
    ) -> SemiAnalyticProjectionOutcome {
        csg_projection(CsgKind::Difference, &self.left, &self.right, point, cell)
    }

    fn primitive_at(&self, point: [f32; 3]) -> u32 {
        csg_primitive_at(CsgKind::Difference, &self.left, &self.right, point)
    }
}

#[derive(Copy, Clone)]
enum CsgKind {
    Union,
    Intersection,
    Difference,
}

enum PairProjection {
    Feature(SemiAnalyticProjection),
    NoFeature,
    Fallback(SemiAnalyticProjectionOutcome),
}

#[derive(Copy, Clone)]
struct FeatureCandidate {
    position: [f32; 3],
    descriptor: u16,
}

fn projected(outcome: SemiAnalyticProjectionOutcome) -> Option<SemiAnalyticProjection> {
    if let SemiAnalyticProjectionOutcome::Projected(value) = outcome {
        Some(value)
    } else {
        None
    }
}

fn csg_projection<A: SemiAnalyticField, B: SemiAnalyticField>(
    kind: CsgKind,
    left: &A,
    right: &B,
    point: [f32; 3],
    cell: &Aabb,
) -> SemiAnalyticProjectionOutcome {
    let left_wins = csg_left_wins(kind, left, right, point);
    let primitive = if left_wins {
        left.primitive_at(point)
    } else {
        right.primitive_at(point)
    };
    if let (Some(left_primitive), Some(right_primitive)) =
        (left.leaf_primitive(), right.leaf_primitive())
    {
        match project_pair_feature(left_primitive, right_primitive, point, cell, primitive) {
            PairProjection::Feature(value) => {
                return SemiAnalyticProjectionOutcome::Projected(value);
            }
            PairProjection::Fallback(reason) => return reason,
            PairProjection::NoFeature => {}
        }
    }
    if left_wins {
        left.project_cell_vertex_detailed(point, cell)
    } else {
        right.project_cell_vertex_detailed(point, cell)
    }
}

fn csg_primitive_at<A: SemiAnalyticField, B: SemiAnalyticField>(
    kind: CsgKind,
    left: &A,
    right: &B,
    point: [f32; 3],
) -> u32 {
    if csg_left_wins(kind, left, right, point) {
        left.primitive_at(point)
    } else {
        right.primitive_at(point)
    }
}

fn csg_left_wins<A: ScalarField, B: ScalarField>(
    kind: CsgKind,
    left: &A,
    right: &B,
    point: [f32; 3],
) -> bool {
    let left_value = field_value(left, point);
    let right_value = field_value(right, point);
    match kind {
        CsgKind::Union => left_value <= right_value,
        CsgKind::Intersection => left_value >= right_value,
        CsgKind::Difference => left_value >= -right_value,
    }
}

fn field_value<F: ScalarField>(field: &F, point: [f32; 3]) -> f32 {
    let mut value = [0.0_f32; 1];
    field.eval_points(&[point], &mut value);
    value[0]
}

fn project_pair_feature(
    left: AnalyticPrimitive,
    right: AnalyticPrimitive,
    point: [f32; 3],
    cell: &Aabb,
    primitive: u32,
) -> PairProjection {
    match (left, right) {
        (AnalyticPrimitive::Box(box_value), AnalyticPrimitive::Cylinder(cylinder))
        | (AnalyticPrimitive::Cylinder(cylinder), AnalyticPrimitive::Box(box_value)) => {
            project_box_cylinder_feature(box_value, cylinder, point, cell, primitive)
        }
        _ => PairProjection::Fallback(SemiAnalyticProjectionOutcome::Unsupported),
    }
}

fn project_box_cylinder_feature(
    box_value: AnalyticBox,
    cylinder: AnalyticCylinder,
    point: [f32; 3],
    cell: &Aabb,
    primitive: u32,
) -> PairProjection {
    if !valid_box(box_value) || !valid_cylinder(cylinder) {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::Invalid);
    }
    if box_value.axes != identity_axes() {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::Unsupported);
    }
    let Some(axis) = coordinate_axis(cylinder.axis) else {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::Unsupported);
    };
    let radial = match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let box_min = sub3(box_value.center, box_value.half_extents);
    let box_max = add3(box_value.center, box_value.half_extents);
    let cylinder_min = cylinder.center[axis] - cylinder.half_height;
    let cylinder_max = cylinder.center[axis] + cylinder.half_height;
    let tolerance = feature_tolerance(box_value, cylinder, cell);
    let mut candidates = Vec::new();
    let mut tangent = false;
    let mut coincident = false;
    let mut descriptor = 0_u16;

    for face_axis in 0..3 {
        for face_coordinate in [box_min[face_axis], box_max[face_axis]] {
            if !within(
                face_coordinate,
                cell.min[face_axis],
                cell.max[face_axis],
                tolerance,
            ) {
                descriptor = descriptor.saturating_add(8);
                continue;
            }
            if face_axis == axis {
                if within(face_coordinate, cylinder_min, cylinder_max, tolerance) {
                    let intervals = [
                        intersect_interval(
                            box_min[radial[0]],
                            box_max[radial[0]],
                            cell.min[radial[0]],
                            cell.max[radial[0]],
                        ),
                        intersect_interval(
                            box_min[radial[1]],
                            box_max[radial[1]],
                            cell.min[radial[1]],
                            cell.max[radial[1]],
                        ),
                    ];
                    if let (Some(first), Some(second)) = (intervals[0], intervals[1]) {
                        candidates.extend(project_clipped_circle(
                            point,
                            cylinder.center,
                            cylinder.radius,
                            axis,
                            face_coordinate,
                            radial,
                            first,
                            second,
                            descriptor,
                            tolerance,
                        ));
                    }
                }
                for cap in [cylinder_min, cylinder_max] {
                    if abs(face_coordinate - cap) <= tolerance
                        && disk_rectangle_overlaps(
                            cylinder.center,
                            cylinder.radius,
                            radial,
                            box_min,
                            box_max,
                            cell,
                            tolerance,
                        )
                    {
                        coincident = true;
                    }
                }
                descriptor = descriptor.saturating_add(8);
                continue;
            }

            let other_radial = if face_axis == radial[0] {
                radial[1]
            } else {
                radial[0]
            };
            let fixed_delta = face_coordinate - cylinder.center[face_axis];
            let discriminant = cylinder.radius * cylinder.radius - fixed_delta * fixed_delta;
            let discriminant_tolerance =
                tolerance * (cylinder.radius + abs(fixed_delta)).max(tolerance);
            if discriminant < -discriminant_tolerance {
                descriptor = descriptor.saturating_add(8);
                continue;
            }
            if abs(discriminant) <= discriminant_tolerance {
                let tangent_coordinate = cylinder.center[other_radial];
                if within(
                    tangent_coordinate,
                    box_min[other_radial],
                    box_max[other_radial],
                    tolerance,
                ) && within(
                    tangent_coordinate,
                    cell.min[other_radial],
                    cell.max[other_radial],
                    tolerance,
                ) && intersect_three_intervals(
                    [box_min[axis], box_max[axis]],
                    [cylinder_min, cylinder_max],
                    [cell.min[axis], cell.max[axis]],
                )
                .is_some()
                {
                    tangent = true;
                }
                descriptor = descriptor.saturating_add(8);
                continue;
            }
            let offset = sqrt(discriminant.max(0.0));

            if let Some(axial_interval) = intersect_three_intervals(
                [box_min[axis], box_max[axis]],
                [cylinder_min, cylinder_max],
                [cell.min[axis], cell.max[axis]],
            ) {
                for branch in [-offset, offset] {
                    let other_coordinate = cylinder.center[other_radial] + branch;
                    if within(
                        other_coordinate,
                        box_min[other_radial].max(cell.min[other_radial]),
                        box_max[other_radial].min(cell.max[other_radial]),
                        tolerance,
                    ) {
                        let mut position = point;
                        position[face_axis] = face_coordinate;
                        position[other_radial] = other_coordinate;
                        position[axis] = clamp(point[axis], axial_interval[0], axial_interval[1]);
                        candidates.push(FeatureCandidate {
                            position,
                            descriptor,
                        });
                    }
                    descriptor = descriptor.saturating_add(1);
                }
            } else {
                descriptor = descriptor.saturating_add(2);
            }

            let other_interval = intersect_interval(
                box_min[other_radial],
                box_max[other_radial],
                cell.min[other_radial],
                cell.max[other_radial],
            );
            for cap in [cylinder_min, cylinder_max] {
                if within(cap, box_min[axis], box_max[axis], tolerance)
                    && within(cap, cell.min[axis], cell.max[axis], tolerance)
                    && let Some(interval) = other_interval
                {
                    let disk_interval = [
                        cylinder.center[other_radial] - offset,
                        cylinder.center[other_radial] + offset,
                    ];
                    if let Some(clipped) = intersect_interval(
                        interval[0],
                        interval[1],
                        disk_interval[0],
                        disk_interval[1],
                    ) {
                        let mut position = point;
                        position[face_axis] = face_coordinate;
                        position[other_radial] = clamp(point[other_radial], clipped[0], clipped[1]);
                        position[axis] = cap;
                        candidates.push(FeatureCandidate {
                            position,
                            descriptor,
                        });
                    }
                }
                descriptor = descriptor.saturating_add(1);
            }
        }
    }

    if coincident {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::Coincident);
    }
    candidates.retain(|candidate| {
        point_in_cell(candidate.position, cell, tolerance)
            && box_surface_residual(candidate.position, box_value, tolerance) <= tolerance
            && cylinder_surface_residual(candidate.position, cylinder) <= tolerance
    });
    candidates.sort_by(|a, b| {
        squared_distance(point, a.position)
            .total_cmp(&squared_distance(point, b.position))
            .then(a.descriptor.cmp(&b.descriptor))
    });
    candidates.dedup_by(|a, b| squared_distance(a.position, b.position) <= tolerance * tolerance);
    if candidates.len() > 1 {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::Ambiguous);
    }
    let Some(candidate) = candidates.first().copied() else {
        return if tangent {
            PairProjection::Fallback(SemiAnalyticProjectionOutcome::Tangent)
        } else {
            PairProjection::NoFeature
        };
    };
    let cell_diagonal = sqrt(squared_distance(cell.min, cell.max));
    if squared_distance(point, candidate.position) > cell_diagonal * cell_diagonal {
        return PairProjection::Fallback(SemiAnalyticProjectionOutcome::OverBudget);
    }
    PairProjection::Feature(SemiAnalyticProjection {
        position: candidate.position,
        primitive,
        feature: SemiAnalyticFeature::IntersectionCurve,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the clipped circle is defined by explicit axes and intervals"
)]
fn project_clipped_circle(
    point: [f32; 3],
    center: [f32; 3],
    radius: f32,
    axis: usize,
    axial_coordinate: f32,
    radial: [usize; 2],
    first: [f32; 2],
    second: [f32; 2],
    descriptor: u16,
    tolerance: f32,
) -> Vec<FeatureCandidate> {
    let delta = [
        point[radial[0]] - center[radial[0]],
        point[radial[1]] - center[radial[1]],
    ];
    let length = sqrt(delta[0] * delta[0] + delta[1] * delta[1]);
    let direction = if length > 0.0 {
        [delta[0] / length, delta[1] / length]
    } else {
        [1.0, 0.0]
    };
    let radial_projection = [
        center[radial[0]] + direction[0] * radius,
        center[radial[1]] + direction[1] * radius,
    ];
    let mut points = Vec::new();
    for boundary in [first[0], first[1]] {
        let delta = boundary - center[radial[0]];
        let discriminant = radius * radius - delta * delta;
        let discriminant_tolerance = tolerance * (radius + abs(delta)).max(tolerance);
        if discriminant >= -discriminant_tolerance {
            let offset = sqrt(discriminant.max(0.0));
            for other in [center[radial[1]] - offset, center[radial[1]] + offset] {
                push_circle_point(
                    &mut points,
                    center,
                    radius,
                    axis,
                    axial_coordinate,
                    radial,
                    [boundary, other],
                    first,
                    second,
                    tolerance,
                );
            }
        }
    }
    for boundary in [second[0], second[1]] {
        let delta = boundary - center[radial[1]];
        let discriminant = radius * radius - delta * delta;
        let discriminant_tolerance = tolerance * (radius + abs(delta)).max(tolerance);
        if discriminant >= -discriminant_tolerance {
            let offset = sqrt(discriminant.max(0.0));
            for other in [center[radial[0]] - offset, center[radial[0]] + offset] {
                push_circle_point(
                    &mut points,
                    center,
                    radius,
                    axis,
                    axial_coordinate,
                    radial,
                    [other, boundary],
                    first,
                    second,
                    tolerance,
                );
            }
        }
    }
    clipped_circle_components(
        point,
        center,
        radius,
        axis,
        axial_coordinate,
        radial,
        first,
        second,
        radial_projection,
        points,
        descriptor,
        tolerance,
    )
}

#[derive(Copy, Clone)]
struct CircleEvent {
    position: [f32; 3],
    direction: [f32; 2],
}

#[expect(
    clippy::too_many_arguments,
    reason = "component classification keeps the circle frame explicit"
)]
fn clipped_circle_components(
    point: [f32; 3],
    center: [f32; 3],
    radius: f32,
    axis: usize,
    axial_coordinate: f32,
    radial: [usize; 2],
    first: [f32; 2],
    second: [f32; 2],
    radial_projection: [f32; 2],
    boundary_points: Vec<[f32; 3]>,
    descriptor: u16,
    tolerance: f32,
) -> Vec<FeatureCandidate> {
    let mut events = boundary_points
        .into_iter()
        .map(|position| CircleEvent {
            position,
            direction: [
                (position[radial[0]] - center[radial[0]]) / radius,
                (position[radial[1]] - center[radial[1]]) / radius,
            ],
        })
        .collect::<Vec<_>>();
    events.sort_by(|a, b| angle_cmp(a.direction, b.direction));
    events.dedup_by(|a, b| squared_distance(a.position, b.position) <= tolerance * tolerance);
    if events.len() > 1
        && squared_distance(events[0].position, events[events.len() - 1].position)
            <= tolerance * tolerance
    {
        events.pop();
    }

    let radial_position =
        circle_position(center, axis, axial_coordinate, radial, radial_projection);
    if events.len() < 2 {
        let candidate = if point_in_rectangle(radial_projection, first, second, tolerance) {
            radial_position
        } else if let Some(event) = events.first() {
            event.position
        } else {
            return Vec::new();
        };
        return vec![FeatureCandidate {
            position: candidate,
            descriptor,
        }];
    }

    let inside_arcs = (0..events.len())
        .map(|index| {
            let next = (index + 1) % events.len();
            let direction = arc_midpoint(events[index].direction, events[next].direction);
            let coordinates = [
                center[radial[0]] + direction[0] * radius,
                center[radial[1]] + direction[1] * radius,
            ];
            point_in_rectangle(coordinates, first, second, tolerance)
        })
        .collect::<Vec<_>>();
    if inside_arcs.iter().all(|inside| !inside) {
        return Vec::new();
    }

    let radial_direction = [
        (radial_projection[0] - center[radial[0]]) / radius,
        (radial_projection[1] - center[radial[1]]) / radius,
    ];
    let radial_is_valid = point_in_rectangle(radial_projection, first, second, tolerance);
    let starts = (0..events.len())
        .filter(|&index| {
            inside_arcs[index] && !inside_arcs[(index + events.len() - 1) % events.len()]
        })
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return vec![FeatureCandidate {
            position: radial_position,
            descriptor,
        }];
    }

    let mut candidates = Vec::with_capacity(starts.len());
    for (component, start) in starts.into_iter().enumerate() {
        let mut last_arc = start;
        while inside_arcs[(last_arc + 1) % events.len()] {
            last_arc = (last_arc + 1) % events.len();
        }
        let end = (last_arc + 1) % events.len();
        let mut best = events[start].position;
        let mut best_distance = squared_distance(point, best);
        let end_distance = squared_distance(point, events[end].position);
        if end_distance.total_cmp(&best_distance).is_lt() {
            best = events[end].position;
            best_distance = end_distance;
        }
        if radial_is_valid {
            let mut arc = start;
            loop {
                let next = (arc + 1) % events.len();
                if direction_in_arc(
                    events[arc].direction,
                    radial_direction,
                    events[next].direction,
                ) {
                    let distance = squared_distance(point, radial_position);
                    if distance.total_cmp(&best_distance).is_lt() {
                        best = radial_position;
                    }
                    break;
                }
                if arc == last_arc {
                    break;
                }
                arc = next;
            }
        }
        candidates.push(FeatureCandidate {
            position: best,
            descriptor: descriptor.saturating_add(u16::try_from(component).unwrap_or(u16::MAX)),
        });
    }
    candidates
}

fn point_in_rectangle(point: [f32; 2], first: [f32; 2], second: [f32; 2], tolerance: f32) -> bool {
    within(point[0], first[0], first[1], tolerance)
        && within(point[1], second[0], second[1], tolerance)
}

fn circle_position(
    center: [f32; 3],
    axis: usize,
    axial_coordinate: f32,
    radial: [usize; 2],
    coordinates: [f32; 2],
) -> [f32; 3] {
    let mut position = center;
    position[axis] = axial_coordinate;
    position[radial[0]] = coordinates[0];
    position[radial[1]] = coordinates[1];
    position
}

fn angle_cmp(a: [f32; 2], b: [f32; 2]) -> core::cmp::Ordering {
    let a_half = a[1] < 0.0 || (a[1] == 0.0 && a[0] < 0.0);
    let b_half = b[1] < 0.0 || (b[1] == 0.0 && b[0] < 0.0);
    a_half.cmp(&b_half).then_with(|| {
        let cross = a[0] * b[1] - a[1] * b[0];
        if cross > 0.0 {
            core::cmp::Ordering::Less
        } else if cross < 0.0 {
            core::cmp::Ordering::Greater
        } else {
            a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
        }
    })
}

fn arc_midpoint(start: [f32; 2], end: [f32; 2]) -> [f32; 2] {
    let sum = [start[0] + end[0], start[1] + end[1]];
    let length = sqrt(sum[0] * sum[0] + sum[1] * sum[1]);
    let first = if length > 0.0 {
        [sum[0] / length, sum[1] / length]
    } else {
        [-start[1], start[0]]
    };
    if direction_in_arc(start, first, end) {
        first
    } else {
        [-first[0], -first[1]]
    }
}

fn direction_in_arc(start: [f32; 2], direction: [f32; 2], end: [f32; 2]) -> bool {
    if angle_cmp(start, end).is_lt() {
        !angle_cmp(direction, start).is_lt() && !angle_cmp(end, direction).is_lt()
    } else {
        !angle_cmp(direction, start).is_lt() || !angle_cmp(end, direction).is_lt()
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "candidate construction keeps the circle frame explicit"
)]
fn push_circle_point(
    points: &mut Vec<[f32; 3]>,
    center: [f32; 3],
    _radius: f32,
    axis: usize,
    axial_coordinate: f32,
    radial: [usize; 2],
    coordinates: [f32; 2],
    first: [f32; 2],
    second: [f32; 2],
    tolerance: f32,
) {
    if !within(coordinates[0], first[0], first[1], tolerance)
        || !within(coordinates[1], second[0], second[1], tolerance)
    {
        return;
    }
    let mut position = center;
    position[axis] = axial_coordinate;
    position[radial[0]] = coordinates[0];
    position[radial[1]] = coordinates[1];
    points.push(position);
}

fn coordinate_axis(axis: [f32; 3]) -> Option<usize> {
    let axis = canonical_axis(axis)?;
    (0..3).find(|&index| {
        axis[index] == 1.0 && (0..3).all(|other| other == index || axis[other] == 0.0)
    })
}

fn valid_cylinder(value: AnalyticCylinder) -> bool {
    canonical_axis(value.axis).is_some()
        && value.center.iter().all(|component| component.is_finite())
        && value.radius.is_finite()
        && value.half_height.is_finite()
        && value.radius > 0.0
        && value.half_height > 0.0
}

fn feature_tolerance(box_value: AnalyticBox, cylinder: AnalyticCylinder, cell: &Aabb) -> f32 {
    let scale = box_value
        .center
        .into_iter()
        .chain(box_value.half_extents)
        .chain(cylinder.center)
        .chain([cylinder.radius, cylinder.half_height])
        .chain(cell.min)
        .chain(cell.max)
        .fold(f32::MIN_POSITIVE, |scale, value| scale.max(abs(value)));
    64.0 * f32::EPSILON * scale
}

fn point_in_cell(point: [f32; 3], cell: &Aabb, tolerance: f32) -> bool {
    (0..3).all(|axis| within(point[axis], cell.min[axis], cell.max[axis], tolerance))
}

fn box_surface_residual(point: [f32; 3], value: AnalyticBox, tolerance: f32) -> f32 {
    let local = box_local_point(value, point);
    if (0..3).any(|axis| abs(local[axis]) > value.half_extents[axis] + tolerance) {
        return f32::INFINITY;
    }
    (0..3)
        .map(|axis| abs(abs(local[axis]) - value.half_extents[axis]))
        .fold(f32::INFINITY, f32::min)
}

fn cylinder_surface_residual(point: [f32; 3], value: AnalyticCylinder) -> f32 {
    let Some(axis) = canonical_axis(value.axis) else {
        return f32::INFINITY;
    };
    let delta = sub3(point, value.center);
    let axial = dot3(delta, axis);
    let radial = sub3(delta, mul3(axis, axial));
    let radius = sqrt(dot3(radial, radial));
    let side = abs(radius - value.radius).max((abs(axial) - value.half_height).max(0.0));
    let cap = abs(abs(axial) - value.half_height).max((radius - value.radius).max(0.0));
    side.min(cap)
}

fn disk_rectangle_overlaps(
    center: [f32; 3],
    radius: f32,
    radial: [usize; 2],
    box_min: [f32; 3],
    box_max: [f32; 3],
    cell: &Aabb,
    tolerance: f32,
) -> bool {
    let Some(first) = intersect_interval(
        box_min[radial[0]],
        box_max[radial[0]],
        cell.min[radial[0]],
        cell.max[radial[0]],
    ) else {
        return false;
    };
    let Some(second) = intersect_interval(
        box_min[radial[1]],
        box_max[radial[1]],
        cell.min[radial[1]],
        cell.max[radial[1]],
    ) else {
        return false;
    };
    let closest = [
        clamp(center[radial[0]], first[0], first[1]),
        clamp(center[radial[1]], second[0], second[1]),
    ];
    let first_delta = closest[0] - center[radial[0]];
    let second_delta = closest[1] - center[radial[1]];
    first_delta * first_delta + second_delta * second_delta
        <= (radius + tolerance) * (radius + tolerance)
}

fn intersect_three_intervals(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> Option<[f32; 2]> {
    intersect_interval(a[0], a[1], b[0], b[1])
        .and_then(|value| intersect_interval(value[0], value[1], c[0], c[1]))
}

fn intersect_interval(a_min: f32, a_max: f32, b_min: f32, b_max: f32) -> Option<[f32; 2]> {
    let minimum = a_min.max(b_min);
    let maximum = a_max.min(b_max);
    (minimum <= maximum).then_some([minimum, maximum])
}

fn within(value: f32, minimum: f32, maximum: f32, tolerance: f32) -> bool {
    value >= minimum - tolerance && value <= maximum + tolerance
}

fn project_box(value: AnalyticBox, point: [f32; 3]) -> Option<SemiAnalyticProjection> {
    if !valid_box(value) {
        return None;
    }
    let local = box_local_point(value, point);
    let mut best: Option<([f32; 3], f32, u8)> = None;
    for axis in 0..3 {
        for side in 0..2 {
            let mut candidate = local;
            for (other, coordinate) in candidate.iter_mut().enumerate() {
                *coordinate = clamp(
                    *coordinate,
                    -value.half_extents[other],
                    value.half_extents[other],
                );
            }
            candidate[axis] = if side == 0 {
                -value.half_extents[axis]
            } else {
                value.half_extents[axis]
            };
            let distance = squared_distance(local, candidate);
            let descriptor = u8::try_from(axis * 2 + side).expect("box face descriptor fits u8");
            if best.is_none_or(|(_, best_distance, best_descriptor)| {
                distance.total_cmp(&best_distance).is_lt()
                    || (distance == best_distance && descriptor < best_descriptor)
            }) {
                best = Some((candidate, distance, descriptor));
            }
        }
    }
    let (local_position, _, _) = best?;
    let boundary_axes = (0..3)
        .filter(|&axis| abs(local_position[axis]) == value.half_extents[axis])
        .count();
    let feature = match boundary_axes {
        3 => SemiAnalyticFeature::Corner,
        2 => SemiAnalyticFeature::Edge,
        _ => SemiAnalyticFeature::Surface,
    };
    Some(SemiAnalyticProjection {
        position: box_world_point(value, local_position),
        primitive: value.primitive,
        feature,
    })
}

fn project_cylinder(value: AnalyticCylinder, point: [f32; 3]) -> Option<SemiAnalyticProjection> {
    let axis = canonical_axis(value.axis)?;
    if !value.center.iter().all(|component| component.is_finite())
        || !value.radius.is_finite()
        || !value.half_height.is_finite()
        || value.radius <= 0.0
        || value.half_height <= 0.0
    {
        return None;
    }

    let delta = sub3(point, value.center);
    let axial = dot3(delta, axis);
    let radial = sub3(delta, mul3(axis, axial));
    let radial_length = sqrt(dot3(radial, radial));
    let radial_direction = if radial_length > 0.0 {
        mul3(radial, radial_length.recip())
    } else {
        canonical_perpendicular(axis)?
    };

    let side_axial = clamp(axial, -value.half_height, value.half_height);
    let side = add3(
        value.center,
        add3(mul3(axis, side_axial), mul3(radial_direction, value.radius)),
    );
    let side_feature = if abs(side_axial) == value.half_height {
        SemiAnalyticFeature::Edge
    } else {
        SemiAnalyticFeature::Surface
    };

    let cap_radial = if radial_length > value.radius {
        mul3(radial_direction, value.radius)
    } else {
        radial
    };
    let cap_min = add3(
        value.center,
        add3(mul3(axis, -value.half_height), cap_radial),
    );
    let cap_max = add3(
        value.center,
        add3(mul3(axis, value.half_height), cap_radial),
    );
    let cap_feature = if radial_length >= value.radius {
        SemiAnalyticFeature::Edge
    } else {
        SemiAnalyticFeature::Surface
    };

    let candidates = [
        (side, side_feature, 0_u8),
        (cap_min, cap_feature, 1_u8),
        (cap_max, cap_feature, 2_u8),
    ];
    let mut best = candidates[0];
    let mut best_distance = squared_distance(point, best.0);
    for candidate in candidates.into_iter().skip(1) {
        let distance = squared_distance(point, candidate.0);
        if distance.total_cmp(&best_distance).is_lt()
            || (distance == best_distance && candidate.2 < best.2)
        {
            best = candidate;
            best_distance = distance;
        }
    }
    Some(SemiAnalyticProjection {
        position: best.0,
        primitive: value.primitive,
        feature: best.1,
    })
}

fn valid_box(value: AnalyticBox) -> bool {
    value.center.iter().all(|component| component.is_finite())
        && value
            .axes
            .iter()
            .flatten()
            .all(|component| component.is_finite())
        && value
            .half_extents
            .iter()
            .all(|extent| extent.is_finite() && *extent > 0.0)
        && value
            .axes
            .iter()
            .all(|axis| abs(dot3(*axis, *axis) - 1.0) <= 1.0e-4)
        && abs(dot3(value.axes[0], value.axes[1])) <= 1.0e-4
        && abs(dot3(value.axes[0], value.axes[2])) <= 1.0e-4
        && abs(dot3(value.axes[1], value.axes[2])) <= 1.0e-4
}

fn canonical_axis(axis: [f32; 3]) -> Option<[f32; 3]> {
    let length_squared = dot3(axis, axis);
    if !axis.iter().all(|component| component.is_finite()) || length_squared <= 0.0 {
        return None;
    }
    let mut normalized = mul3(axis, sqrt(length_squared).recip());
    let first = normalized
        .iter()
        .copied()
        .find(|component| *component != 0.0)?;
    if first < 0.0 {
        normalized = mul3(normalized, -1.0);
    }
    Some(normalized)
}

fn canonical_perpendicular(axis: [f32; 3]) -> Option<[f32; 3]> {
    let basis_index = if abs(axis[0]) <= abs(axis[1]) && abs(axis[0]) <= abs(axis[2]) {
        0
    } else if abs(axis[1]) <= abs(axis[2]) {
        1
    } else {
        2
    };
    let mut basis = [0.0_f32; 3];
    basis[basis_index] = 1.0;
    let perpendicular = sub3(basis, mul3(axis, dot3(basis, axis)));
    let length = sqrt(dot3(perpendicular, perpendicular));
    if length > 0.0 {
        Some(mul3(perpendicular, length.recip()))
    } else {
        None
    }
}

fn box_local_point(value: AnalyticBox, point: [f32; 3]) -> [f32; 3] {
    let delta = sub3(point, value.center);
    [
        dot3(delta, value.axes[0]),
        dot3(delta, value.axes[1]),
        dot3(delta, value.axes[2]),
    ]
}

fn box_world_point(value: AnalyticBox, point: [f32; 3]) -> [f32; 3] {
    add3(
        value.center,
        add3(
            add3(mul3(value.axes[0], point[0]), mul3(value.axes[1], point[1])),
            mul3(value.axes[2], point[2]),
        ),
    )
}

fn translate_primitive(
    primitive: AnalyticPrimitive,
    offset: [f32; 3],
) -> Option<AnalyticPrimitive> {
    Some(match primitive {
        AnalyticPrimitive::Box(mut value) => {
            value.center = add3(value.center, offset);
            AnalyticPrimitive::Box(value)
        }
        AnalyticPrimitive::Cylinder(mut value) => {
            value.center = add3(value.center, offset);
            AnalyticPrimitive::Cylinder(value)
        }
    })
}

fn scale_primitive(primitive: AnalyticPrimitive, factor: f32) -> Option<AnalyticPrimitive> {
    if !factor.is_finite() || factor <= 0.0 {
        return None;
    }
    Some(match primitive {
        AnalyticPrimitive::Box(mut value) => {
            value.center = mul3(value.center, factor);
            value.half_extents = mul3(value.half_extents, factor);
            AnalyticPrimitive::Box(value)
        }
        AnalyticPrimitive::Cylinder(mut value) => {
            value.center = mul3(value.center, factor);
            value.radius *= factor;
            value.half_height *= factor;
            AnalyticPrimitive::Cylinder(value)
        }
    })
}

fn transform_primitive(
    primitive: AnalyticPrimitive,
    transform: RigidTransform3,
) -> Option<AnalyticPrimitive> {
    Some(match primitive {
        AnalyticPrimitive::Box(mut value) => {
            value.center = local_to_world_point(transform, value.center);
            for axis in &mut value.axes {
                *axis = transform.local_to_world_vector(*axis);
            }
            AnalyticPrimitive::Box(value)
        }
        AnalyticPrimitive::Cylinder(mut value) => {
            value.center = local_to_world_point(transform, value.center);
            value.axis = transform.local_to_world_vector(value.axis);
            AnalyticPrimitive::Cylinder(value)
        }
    })
}

fn transform_bounds(bounds: &Aabb, transform: RigidTransform3) -> Option<Aabb> {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for corner in aabb_corners(bounds) {
        let local = transform.world_to_local_point(corner);
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(local[axis]);
            maximum[axis] = maximum[axis].max(local[axis]);
        }
    }
    Aabb::new(minimum, maximum)
}

fn map_projection_position(
    projection: Option<SemiAnalyticProjection>,
    map: impl FnOnce([f32; 3]) -> [f32; 3],
) -> Option<SemiAnalyticProjection> {
    projection.map(|mut projection| {
        projection.position = map(projection.position);
        projection
    })
}

fn map_outcome_position(
    outcome: SemiAnalyticProjectionOutcome,
    map: impl FnOnce([f32; 3]) -> [f32; 3],
) -> SemiAnalyticProjectionOutcome {
    match outcome {
        SemiAnalyticProjectionOutcome::Projected(mut projection) => {
            projection.position = map(projection.position);
            SemiAnalyticProjectionOutcome::Projected(projection)
        }
        fallback => fallback,
    }
}

fn local_to_world_point(transform: RigidTransform3, point: [f32; 3]) -> [f32; 3] {
    add3(transform.origin(), transform.local_to_world_vector(point))
}

fn aabb_corners(bounds: &Aabb) -> [[f32; 3]; 8] {
    core::array::from_fn(|corner| {
        [
            if corner & 1 == 0 {
                bounds.min[0]
            } else {
                bounds.max[0]
            },
            if corner & 2 == 0 {
                bounds.min[1]
            } else {
                bounds.max[1]
            },
            if corner & 4 == 0 {
                bounds.min[2]
            } else {
                bounds.max[2]
            },
        ]
    })
}

const fn identity_axes() -> [[f32; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    dot3(sub3(a, b), sub3(a, b))
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul3(value: [f32; 3], scalar: f32) -> [f32; 3] {
    [value[0] * scalar, value[1] * scalar, value[2] * scalar]
}

fn clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    value.max(minimum).min(maximum)
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

fn sqrt(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.sqrt()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::sqrtf(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnalyticBox, AnalyticCylinder, AnalyticPrimitive, SemiAnalyticFeature, SemiAnalyticField,
        SemiAnalyticProjectionOutcome, sqrt,
    };
    use crate::analytic::{BoxField, CylinderField, Difference, Intersection, TaggedField, Union};
    use crate::transform::{RigidTransform3, Transform3, Translate, UniformScale};
    use exedra_spatial::Aabb;

    fn cell() -> Aabb {
        Aabb::new([-10.0; 3], [10.0; 3]).expect("valid cell")
    }

    #[test]
    fn box_projection_uses_stable_faces_edges_and_corners() {
        let primitive = AnalyticPrimitive::Box(AnalyticBox {
            primitive: 7,
            center: [1.0, 2.0, 3.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            half_extents: [2.0, 3.0, 4.0],
        });

        let interior = primitive.project([1.0, 2.0, 3.0]).expect("valid box");
        assert_eq!(interior.position, [-1.0, 2.0, 3.0]);
        assert_eq!(interior.feature, SemiAnalyticFeature::Surface);

        let edge = primitive.project([4.0, 6.0, 3.0]).expect("valid box");
        assert_eq!(edge.position, [3.0, 5.0, 3.0]);
        assert_eq!(edge.feature, SemiAnalyticFeature::Edge);

        let corner = primitive.project([4.0, 6.0, 8.0]).expect("valid box");
        assert_eq!(corner.position, [3.0, 5.0, 7.0]);
        assert_eq!(corner.feature, SemiAnalyticFeature::Corner);
        assert_eq!(corner.primitive, 7);
    }

    #[test]
    fn cylinder_projection_handles_arbitrary_axis_and_axis_queries() {
        let cylinder = AnalyticPrimitive::Cylinder(AnalyticCylinder {
            primitive: 11,
            center: [0.5, -0.25, 1.0],
            axis: [1.0, 2.0, 3.0],
            radius: 0.75,
            half_height: 1.5,
        });
        let negated = AnalyticPrimitive::Cylinder(AnalyticCylinder {
            axis: [-1.0, -2.0, -3.0],
            ..match cylinder {
                AnalyticPrimitive::Cylinder(value) => value,
                AnalyticPrimitive::Box(_) => unreachable!(),
            }
        });

        for point in [[2.0, 0.5, -0.25], [0.5, -0.25, 1.0], [4.0, 4.0, 4.0]] {
            let first = cylinder.project(point).expect("valid cylinder");
            let second = negated.project(point).expect("axis sign is immaterial");
            assert_eq!(first, second);
            assert_eq!(first.primitive, 11);
            assert!(cylinder_residual(first.position, cylinder) <= 2.0e-6);
        }
    }

    #[test]
    fn invalid_primitive_geometry_returns_none() {
        let invalid_box = AnalyticPrimitive::Box(AnalyticBox {
            primitive: 1,
            center: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            half_extents: [1.0; 3],
        });
        let invalid_cylinder = AnalyticPrimitive::Cylinder(AnalyticCylinder {
            primitive: 2,
            center: [0.0; 3],
            axis: [0.0; 3],
            radius: 1.0,
            half_height: 1.0,
        });

        assert_eq!(invalid_box.project([0.0; 3]), None);
        assert_eq!(invalid_cylinder.project([0.0; 3]), None);

        for (radius, half_height) in [
            (0.0, 1.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (1.0, -1.0),
            (f32::NAN, 1.0),
            (1.0, f32::INFINITY),
        ] {
            let invalid = AnalyticPrimitive::Cylinder(AnalyticCylinder {
                primitive: 2,
                center: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius,
                half_height,
            });
            assert_eq!(invalid.project([2.0, 0.0, 0.0]), None);
        }
    }

    #[test]
    fn cylinder_side_cap_tie_uses_stable_rim_candidate() {
        let cylinder = AnalyticPrimitive::Cylinder(AnalyticCylinder {
            primitive: 5,
            center: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            radius: 1.0,
            half_height: 1.0,
        });

        let projection = cylinder.project([2.0, 0.0, 2.0]).expect("valid cylinder");

        assert_eq!(projection.position, [1.0, 0.0, 1.0]);
        assert_eq!(projection.feature, SemiAnalyticFeature::Edge);
    }

    #[test]
    fn tagged_primitives_forward_identity_and_projection() {
        let tagged = TaggedField {
            field: BoxField {
                center: [0.0; 3],
                half_extents: [1.0; 3],
            },
            provenance: 23_u32,
        };
        let projected = tagged
            .project_cell_vertex([2.0, 0.25, -0.5], &cell())
            .expect("tagged box projects");

        assert_eq!(projected.position, [1.0, 0.25, -0.5]);
        assert_eq!(projected.primitive, 23);
        assert_eq!(tagged.primitive_at([100.0; 3]), 23);
    }

    #[test]
    fn primitive_projection_may_leave_the_source_cell() {
        let tagged = TaggedField {
            field: BoxField {
                center: [0.0; 3],
                half_extents: [1.0; 3],
            },
            provenance: 23_u32,
        };
        let source_cell = Aabb::new([0.8, -0.1, -0.1], [0.95, 0.1, 0.1]).expect("valid cell");

        let projected = tagged
            .project_cell_vertex([0.9, 0.0, 0.0], &source_cell)
            .expect("box projection exists");

        assert_eq!(projected.position, [1.0, 0.0, 0.0]);
        assert!(projected.position[0] > source_cell.max[0]);
    }

    #[test]
    fn wrappers_project_in_local_space_and_return_world_positions() {
        let tagged = TaggedField {
            field: CylinderField {
                center: [0.0; 3],
                axis: [0.0, 1.0, 0.0],
                radius: 1.0,
                half_height: 2.0,
            },
            provenance: 31_u32,
        };
        let translated = Translate::new(tagged, [3.0, 0.0, 0.0]);
        let scaled = UniformScale::new(translated, 2.0).expect("positive scale");
        let rotation = RigidTransform3::new(
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        )
        .expect("orthonormal transform");
        let transformed = Transform3::new(scaled, rotation);

        let first = transformed
            .project_cell_vertex([0.0, 10.0, 0.0], &cell())
            .expect("wrapped cylinder projects");
        let second = transformed
            .project_cell_vertex([0.0, 10.0, 0.0], &cell())
            .expect("projection repeats");

        assert_eq!(first, second);
        assert_eq!(first.primitive, 31);
        assert_eq!(first.position, [0.0, 9.0, 0.0]);
        assert_eq!(
            transformed.leaf_primitive().map(|p| p.primitive()),
            Some(31)
        );
    }

    #[test]
    fn projection_residual_scales_from_milli_to_kilo() {
        for scale in [1.0e-3_f32, 1.0e4_f32] {
            let cylinder = AnalyticPrimitive::Cylinder(AnalyticCylinder {
                primitive: 1,
                center: [0.25 * scale, -0.5 * scale, scale],
                axis: [0.0, 0.0, 1.0],
                radius: 0.75 * scale,
                half_height: 1.25 * scale,
            });
            let projected = cylinder
                .project([2.0 * scale, -0.2 * scale, 1.1 * scale])
                .expect("valid scaled cylinder");
            let tolerance = 32.0 * f32::EPSILON * scale.max(1.0e-3);
            assert!(cylinder_residual(projected.position, cylinder) <= tolerance);

            let box_primitive = AnalyticPrimitive::Box(AnalyticBox {
                primitive: 2,
                center: [0.25 * scale, -0.5 * scale, scale],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                half_extents: [0.75 * scale, 0.5 * scale, 1.25 * scale],
            });
            let box_projection = box_primitive
                .project([2.0 * scale, -0.2 * scale, 1.1 * scale])
                .expect("valid scaled box");
            assert!(box_residual(box_projection.position, box_primitive) <= tolerance);
        }
    }

    #[test]
    fn aligned_box_cylinder_csg_snaps_transverse_rim_curve() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let cell = Aabb::new([0.5, -0.1, 0.9], [0.7, 0.1, 1.1]).expect("feature cell");
        let point = [0.58, 0.02, 0.98];

        for (outcome, primitive) in [
            (
                Union::new(box_field, cylinder).project_cell_vertex_detailed(point, &cell),
                10,
            ),
            (
                Union::new(cylinder, box_field).project_cell_vertex_detailed(point, &cell),
                10,
            ),
            (
                Intersection::new(box_field, cylinder).project_cell_vertex_detailed(point, &cell),
                20,
            ),
            (
                Intersection::new(cylinder, box_field).project_cell_vertex_detailed(point, &cell),
                20,
            ),
            (
                Difference::new(box_field, cylinder).project_cell_vertex_detailed(point, &cell),
                20,
            ),
            (
                Difference::new(cylinder, box_field).project_cell_vertex_detailed(point, &cell),
                10,
            ),
        ] {
            let SemiAnalyticProjectionOutcome::Projected(projection) = outcome else {
                panic!("aligned transverse feature should project: {outcome:?}");
            };
            assert_eq!(projection.feature, SemiAnalyticFeature::IntersectionCurve);
            assert_eq!(projection.primitive, primitive);
            assert!((projection.position[2] - 1.0).abs() <= 1.0e-6);
            let radius = (projection.position[0] * projection.position[0]
                + projection.position[1] * projection.position[1])
                .sqrt();
            assert!((radius - 0.6).abs() <= 1.0e-6);
        }
    }

    #[test]
    fn aligned_pair_projects_side_and_cap_lines_and_dominant_surfaces() {
        let box_field = TaggedField {
            field: BoxField {
                center: [0.0; 3],
                half_extents: [1.0; 3],
            },
            provenance: 10,
        };
        let side_cylinder = TaggedField {
            field: CylinderField {
                center: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius: 1.2,
                half_height: 2.0,
            },
            provenance: 20,
        };
        let side_coordinate = sqrt(1.2_f32 * 1.2 - 1.0);
        let side_cell = Aabb::new(
            [0.9, side_coordinate - 0.1, -0.1],
            [1.1, side_coordinate + 0.1, 0.1],
        )
        .expect("side line cell");
        let SemiAnalyticProjectionOutcome::Projected(side_line) =
            Difference::new(box_field, side_cylinder)
                .project_cell_vertex_detailed([0.98, side_coordinate - 0.02, 0.03], &side_cell)
        else {
            panic!("face-side line should project");
        };
        assert_eq!(side_line.feature, SemiAnalyticFeature::IntersectionCurve);
        assert_eq!(side_line.position[0], 1.0);
        assert!((side_line.position[1] - side_coordinate).abs() <= 1.0e-6);
        assert_eq!(side_line.position[2], 0.03);
        assert_eq!(side_line.primitive, 20);

        let cap_cylinder = TaggedField {
            field: CylinderField {
                center: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius: 1.2,
                half_height: 0.75,
            },
            provenance: 20,
        };
        let line_cell = Aabb::new([0.9, -0.1, 0.7], [1.1, 0.1, 0.8]).expect("line cell");
        let SemiAnalyticProjectionOutcome::Projected(line) =
            Difference::new(box_field, cap_cylinder)
                .project_cell_vertex_detailed([0.98, 0.02, 0.74], &line_cell)
        else {
            panic!("face-cap line should project");
        };
        assert_eq!(line.feature, SemiAnalyticFeature::IntersectionCurve);
        assert_eq!(line.position[0], 1.0);
        assert_eq!(line.position[2], 0.75);
        assert_eq!(line.primitive, 20);

        let (box_field, cylinder) = tagged_box_cylinder();
        let box_cell = Aabb::new([0.9, 0.2, 0.2], [1.1, 0.3, 0.3]).expect("box surface");
        let SemiAnalyticProjectionOutcome::Projected(box_surface) =
            Difference::new(box_field, cylinder)
                .project_cell_vertex_detailed([0.98, 0.25, 0.25], &box_cell)
        else {
            panic!("dominant box surface should project");
        };
        assert_eq!(box_surface.feature, SemiAnalyticFeature::Surface);
        assert_eq!(box_surface.primitive, 10);
        assert!(
            box_residual(
                box_surface.position,
                box_field.leaf_primitive().expect("box primitive")
            ) <= 1.0e-6
        );

        let cylinder_cell =
            Aabb::new([0.5, -0.1, -0.1], [0.7, 0.1, 0.1]).expect("cylinder surface");
        let SemiAnalyticProjectionOutcome::Projected(cylinder_surface) =
            Difference::new(box_field, cylinder)
                .project_cell_vertex_detailed([0.58, 0.02, 0.0], &cylinder_cell)
        else {
            panic!("dominant cylinder surface should project");
        };
        assert_eq!(cylinder_surface.feature, SemiAnalyticFeature::Surface);
        assert_eq!(cylinder_surface.primitive, 20);
        assert!(
            cylinder_residual(
                cylinder_surface.position,
                cylinder.leaf_primitive().expect("cylinder primitive")
            ) <= 1.0e-6
        );
    }

    #[test]
    fn clipped_circle_counts_disconnected_arcs() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let thin_strip = Aabb::new([-0.1, -0.7, 0.9], [0.1, 0.7, 1.1]).expect("thin strip");

        assert_eq!(
            Difference::new(box_field, cylinder)
                .project_cell_vertex_detailed([0.0, 0.58, 0.98], &thin_strip),
            SemiAnalyticProjectionOutcome::Ambiguous
        );
    }

    #[test]
    fn invalid_pairs_are_distinct_from_unsupported_orientation() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let invalid = TaggedField {
            field: CylinderField {
                radius: -0.6,
                ..cylinder.field
            },
            ..cylinder
        };

        assert_eq!(
            Difference::new(box_field, invalid)
                .project_cell_vertex_detailed([0.58, 0.02, 0.98], &cell()),
            SemiAnalyticProjectionOutcome::Invalid
        );
    }

    #[test]
    fn tangent_contact_is_local_to_the_cell() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let tangent_cylinder = TaggedField {
            field: CylinderField {
                radius: 1.0,
                ..cylinder.field
            },
            ..cylinder
        };
        let excludes_contact =
            Aabb::new([0.9, 0.4, -0.1], [1.1, 0.5, 0.1]).expect("off-contact cell");

        let outcome = Difference::new(box_field, tangent_cylinder)
            .project_cell_vertex_detailed([1.0, 0.45, 0.0], &excludes_contact);
        let SemiAnalyticProjectionOutcome::Projected(projection) = outcome else {
            panic!("off-contact cell should use ordinary surface projection: {outcome:?}");
        };
        assert_eq!(projection.feature, SemiAnalyticFeature::Surface);
        assert_eq!(projection.primitive, 10);
        assert_eq!(projection.position, [1.0, 0.45, 0.0]);
    }

    #[test]
    fn rotated_tangent_and_coincident_pairs_are_typed_fallbacks() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let feature_cell = Aabb::new([0.5, -0.1, 0.9], [0.7, 0.1, 1.1]).expect("feature cell");
        let rotated = TaggedField {
            field: CylinderField {
                axis: [1.0, 1.0, 0.0],
                ..cylinder.field
            },
            ..cylinder
        };
        assert_eq!(
            Difference::new(box_field, rotated)
                .project_cell_vertex_detailed([0.58, 0.02, 0.98], &feature_cell),
            SemiAnalyticProjectionOutcome::Unsupported
        );

        let tangent_cylinder = TaggedField {
            field: CylinderField {
                radius: 1.0,
                ..cylinder.field
            },
            ..cylinder
        };
        let tangent_cell = Aabb::new([0.9, -0.1, -0.1], [1.1, 0.1, 0.1]).expect("tangent");
        assert_eq!(
            Difference::new(box_field, tangent_cylinder)
                .project_cell_vertex_detailed([1.0, 0.0, 0.0], &tangent_cell),
            SemiAnalyticProjectionOutcome::Tangent
        );

        let coincident_cylinder = TaggedField {
            field: CylinderField {
                radius: 0.6,
                half_height: 1.0,
                ..cylinder.field
            },
            ..cylinder
        };
        let coincident_cell = Aabb::new([-0.1, -0.1, 0.9], [0.1, 0.1, 1.1]).expect("coincident");
        assert_eq!(
            Difference::new(box_field, coincident_cylinder)
                .project_cell_vertex_detailed([0.0, 0.0, 1.0], &coincident_cell),
            SemiAnalyticProjectionOutcome::Coincident
        );
    }

    #[test]
    fn multiple_features_and_excessive_displacement_are_typed_fallbacks() {
        let (box_field, cylinder) = tagged_box_cylinder();
        let whole_box = Aabb::new([-1.1; 3], [1.1; 3]).expect("large cell");
        assert_eq!(
            Difference::new(box_field, cylinder)
                .project_cell_vertex_detailed([0.55, 0.0, 0.9], &whole_box),
            SemiAnalyticProjectionOutcome::Ambiguous
        );

        let feature_cell = Aabb::new([0.5, -0.1, 0.9], [0.7, 0.1, 1.1]).expect("feature cell");
        assert_eq!(
            Difference::new(box_field, cylinder)
                .project_cell_vertex_detailed([100.0, 0.0, 100.0], &feature_cell),
            SemiAnalyticProjectionOutcome::OverBudget
        );
    }

    fn tagged_box_cylinder() -> (TaggedField<BoxField, u32>, TaggedField<CylinderField, u32>) {
        (
            TaggedField {
                field: BoxField {
                    center: [0.0; 3],
                    half_extents: [1.0; 3],
                },
                provenance: 10,
            },
            TaggedField {
                field: CylinderField {
                    center: [0.0; 3],
                    axis: [0.0, 0.0, 1.0],
                    radius: 0.6,
                    half_height: 2.0,
                },
                provenance: 20,
            },
        )
    }

    fn box_residual(point: [f32; 3], primitive: AnalyticPrimitive) -> f32 {
        let AnalyticPrimitive::Box(value) = primitive else {
            unreachable!();
        };
        let delta = [
            (point[0] - value.center[0]).abs(),
            (point[1] - value.center[1]).abs(),
            (point[2] - value.center[2]).abs(),
        ];
        let within = (0..3).all(|axis| delta[axis] <= value.half_extents[axis]);
        if !within {
            return f32::INFINITY;
        }
        (0..3)
            .map(|axis| (delta[axis] - value.half_extents[axis]).abs())
            .fold(f32::INFINITY, f32::min)
    }

    fn cylinder_residual(point: [f32; 3], primitive: AnalyticPrimitive) -> f32 {
        let AnalyticPrimitive::Cylinder(value) = primitive else {
            unreachable!();
        };
        let axis_length = (value.axis[0] * value.axis[0]
            + value.axis[1] * value.axis[1]
            + value.axis[2] * value.axis[2])
            .sqrt();
        let axis = [
            value.axis[0] / axis_length,
            value.axis[1] / axis_length,
            value.axis[2] / axis_length,
        ];
        let delta = [
            point[0] - value.center[0],
            point[1] - value.center[1],
            point[2] - value.center[2],
        ];
        let axial = delta[0] * axis[0] + delta[1] * axis[1] + delta[2] * axis[2];
        let radial = [
            delta[0] - axial * axis[0],
            delta[1] - axial * axis[1],
            delta[2] - axial * axis[2],
        ];
        let radius = (radial[0] * radial[0] + radial[1] * radial[1] + radial[2] * radial[2]).sqrt();
        let side = (radius - value.radius)
            .abs()
            .max((axial.abs() - value.half_height).max(0.0));
        let cap = (axial.abs() - value.half_height)
            .abs()
            .max((radius - value.radius).max(0.0));
        side.min(cap)
    }
}
