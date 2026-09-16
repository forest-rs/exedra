// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Authored coordinate frames on evaluated planar face patches.
//!
//! This module owns planar selection and frame validation. It does not choose
//! an origin, infer an in-plane axis, or track a face across recipe reevaluation.
//! Frames are snapshots in body coordinates, pinned to one logical mesh revision.

use crate::edge_finish::OperandRegion;
use crate::ir::Placement3;
use crate::tessellate::{Feature, TessellatedBody};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use exedra_math::{add, cross, dot, norm, normalize, scale, sub};
use exedra_mesh::{FaceId, FaceTriangulation, Mesh, MeshRevision, attr};

/// The planar face patch whose winding determines the workplane's +Z.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WorkplaneSelection {
    /// One live face of the source mesh. IDs are scoped to that logical mesh.
    Face(FaceId),
    /// All faces carrying this region. They must be edge-connected and must
    /// not mix Boolean operands with reused region numbers.
    Region(u32),
    /// A region restricted to one producing Boolean operand.
    OperandRegion(OperandRegion),
}

/// Accuracy and selected-patch work limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WorkplanePolicy {
    /// Maximum vertex and authored-origin distance from the selected plane,
    /// in body units. Must be positive and finite.
    pub distance_tolerance: f64,
    /// Minimum sine of the angle between authored X and the face normal.
    /// Must lie in `(0, 1]`; directions closer to parallel are refused.
    pub min_axis_sine: f64,
    /// Maximum selected faces. Region resolution scans the body's face list.
    pub max_faces: u32,
    /// Maximum selected face corners, including shared vertices repeatedly.
    pub max_corners: u32,
}
impl Default for WorkplanePolicy {
    fn default() -> Self {
        Self {
            distance_tolerance: 1e-6,
            min_axis_sine: 1e-6,
            max_faces: 16384,
            max_corners: 65536,
        }
    }
}

/// A refused planar selection or authored frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkplaneError {
    /// Invalid policy, nonfinite origin, or unusable X direction.
    InvalidInput,
    /// The source provenance or retained workplane no longer matches the mesh.
    StaleSource,
    /// No face matches the selection.
    EmptySelection,
    /// A selected face is stale, outside, malformed, or cannot be triangulated.
    InvalidFace,
    /// Selected faces are disconnected or mix Boolean operand identities.
    AmbiguousSelection,
    /// Selected face corners do not lie on one plane within tolerance.
    NonPlanar,
    /// A face has zero area, incompatible winding, or unrepresentable geometry.
    InvalidGeometry,
    /// The authored origin is outside the plane's distance tolerance.
    OriginOffPlane,
    /// Authored X is too nearly parallel to the face normal.
    ParallelAxis,
    /// The selected patch exceeds a work limit.
    BudgetExceeded,
}
impl core::fmt::Display for WorkplaneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "invalid workplane policy or authored frame",
            Self::StaleSource => "workplane source revision is stale",
            Self::EmptySelection => "workplane selection is empty",
            Self::InvalidFace => "workplane selection contains an invalid face",
            Self::AmbiguousSelection => "workplane selection is disconnected or operand-ambiguous",
            Self::NonPlanar => "workplane selection is not planar within tolerance",
            Self::InvalidGeometry => "workplane geometry is degenerate or inconsistently oriented",
            Self::OriginOffPlane => "workplane origin is not on the selected plane",
            Self::ParallelAxis => "workplane X direction is too parallel to the normal",
            Self::BudgetExceeded => "workplane selection exceeds its work budget",
        })
    }
}
impl core::error::Error for WorkplaneError {}

/// An orthonormal, right-handed frame on a selected planar patch.
///
/// +Z follows source winding (outward only when the source is oriented outward).
/// The origin is the authored point projected onto the plane within tolerance.
/// It need not be inside the face boundary: this is a coordinate frame, not a
/// containment query. Selected geometry is checked in f64 after exact promotion
/// from mesh storage. Holes do not affect frame validity.
///
/// Like a source map, this value is bound to one logical mesh; revision equality
/// cannot identify unrelated meshes. Call [`Self::check`] before reuse after edits.
#[derive(Clone, Debug)]
pub struct Workplane {
    frame: Placement3,
    faces: Vec<FaceId>,
    revision: MeshRevision,
    max_plane_deviation: f64,
}
impl Workplane {
    /// Returns the placement mapping workplane coordinates into body coordinates.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Returns the selected source faces in deterministic ID order.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        &self.faces
    }
    /// Maximum measured source-corner distance from the returned plane.
    #[must_use]
    pub fn max_plane_deviation(&self) -> f64 {
        self.max_plane_deviation
    }
    /// Refuses reuse after an edit of the same logical mesh.
    pub fn check(&self, mesh: &Mesh) -> Result<(), WorkplaneError> {
        if mesh.revision() == self.revision {
            Ok(())
        } else {
            Err(WorkplaneError::StaleSource)
        }
    }
    /// Maps a local point into body coordinates, including an offset along +Z.
    #[must_use]
    pub fn to_body(&self, point: [f64; 3]) -> [f64; 3] {
        self.frame
            .rows
            .map(|r| r[0] * point[0] + r[1] * point[1] + r[2] * point[2] + r[3])
    }
    /// Maps a body-space point into this orthonormal frame.
    #[must_use]
    pub fn to_local(&self, point: [f64; 3]) -> [f64; 3] {
        let relative = sub(point, self.frame.rows.map(|r| r[3]));
        core::array::from_fn(|i| dot(relative, self.frame.rows.map(|r| r[i])))
    }
    /// Returns an equally oriented placement translated by a local offset.
    /// Use local XY for feature positions and local Z for stand-off or depth.
    #[must_use]
    pub fn placement_at(&self, offset: [f64; 3]) -> Placement3 {
        let mut frame = self.frame;
        for (row, coordinate) in frame.rows.iter_mut().zip(self.to_body(offset)) {
            row[3] = coordinate;
        }
        frame
    }
}

/// Builds a frame on a planar face or connected region using authored origin/X.
///
/// `origin` and `x_direction` are in body coordinates. X is normalized after
/// projection into the selected plane; Y is Z cross X. There is no automatic
/// axis or origin fallback. The signed area sum determines Z independently of
/// cap triangulation in exact arithmetic; floating-point roundoff can produce
/// small differences after remeshing. Open planar patches are supported.
///
/// # Errors
/// Rejects stale provenance, empty/ambiguous selections, invalid or nonplanar
/// geometry, an off-plane origin, a near-normal X direction, and exhausted
/// budgets. This does not certify the source as a solid or infer curved-face
/// tangent frames. The additive API requires no existing-caller migration.
///
/// ```
/// use exedra_constructive::{builders::rect, ir::{CapMode, Placement3},
///     tessellate::{EvalPolicy, REGION_CAP_END, tessellate_extrude},
///     workplane::{WorkplaneSelection, WorkplanePolicy, face_workplane}};
/// let body = tessellate_extrude(&rect(4.0, 3.0)?, &Placement3::IDENTITY,
///     2.0, CapMode::Both, &EvalPolicy::default())?;
/// let plane = face_workplane(&body, WorkplaneSelection::Region(REGION_CAP_END),
///     [0.0, 0.0, 2.0], [1.0, 0.0, 0.0], &WorkplanePolicy::default())?;
/// assert_eq!(plane.to_body([1.0, 1.0, 0.0]), [1.0, 1.0, 2.0]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn face_workplane(
    body: &TessellatedBody,
    selection: WorkplaneSelection,
    origin: [f64; 3],
    x_direction: [f64; 3],
    policy: &WorkplanePolicy,
) -> Result<Workplane, WorkplaneError> {
    use WorkplaneError as Error;
    if !policy.distance_tolerance.is_finite()
        || policy.distance_tolerance <= 0.0
        || !policy.min_axis_sine.is_finite()
        || policy.min_axis_sine <= 0.0
        || policy.min_axis_sine > 1.0
        || policy.max_faces == 0
        || policy.max_corners == 0
        || origin.iter().any(|v| !v.is_finite())
    {
        return Err(Error::InvalidInput);
    }
    let authored_x = normalize(x_direction).ok_or(Error::InvalidInput)?;
    body.source_map
        .check(&body.mesh)
        .map_err(|_| Error::StaleSource)?;
    let mesh = &body.mesh;
    let regions = mesh.attrs().dense(attr::FACE_REGION);
    let matches = |face: FaceId| match selection {
        WorkplaneSelection::Face(wanted) => face == wanted,
        WorkplaneSelection::Region(region) => {
            regions.and_then(|r| r.get(face.into())) == Some(&region)
        }
        WorkplaneSelection::OperandRegion(wanted) => {
            regions.and_then(|r| r.get(face.into())) == Some(&wanted.region)
                && body.source_map.face_feature(face)
                    == Some(Feature::BooleanFace {
                        operand: wanted.operand,
                    })
        }
    };
    if let WorkplaneSelection::Face(face) = selection
        && (face == FaceId::OUTSIDE || mesh.face_edge(face).is_none())
    {
        return Err(Error::InvalidFace);
    }
    let mut faces = Vec::new();
    if let WorkplaneSelection::Face(face) = selection {
        faces.push(face);
    } else {
        for face in mesh.faces().filter(|&face| matches(face)) {
            if faces.len() >= policy.max_faces as usize {
                return Err(Error::BudgetExceeded);
            }
            faces.push(face);
        }
    }
    if faces.is_empty() {
        return Err(Error::EmptySelection);
    }
    if matches!(selection, WorkplaneSelection::Region(_)) {
        let operands: BTreeSet<_> = faces
            .iter()
            .map(|&face| match body.source_map.face_feature(face) {
                Some(Feature::BooleanFace { operand }) => Some(operand),
                _ => None,
            })
            .collect();
        if operands.len() > 1 {
            return Err(Error::AmbiguousSelection);
        }
    }
    let selected: BTreeSet<_> = faces.iter().copied().collect();
    let mut pending = alloc::vec![faces[0]];
    let mut visited = BTreeSet::new();
    let mut corners = 0_u64;
    let mut all_points = Vec::new();
    let mut normals = Vec::new();
    let mut sum = [0.0; 3];
    while let Some(face) = pending.pop() {
        if !visited.insert(face) {
            continue;
        }
        let mut points = Vec::new();
        for edge in mesh.face_loop(face) {
            corners += 1;
            if corners > u64::from(policy.max_corners) {
                return Err(Error::BudgetExceeded);
            }
            let adjacent = mesh
                .twin(edge)
                .and_then(|edge| mesh.face(edge))
                .ok_or(Error::InvalidFace)?;
            if selected.contains(&adjacent) && !visited.contains(&adjacent) {
                pending.push(adjacent);
            }
            let p = mesh
                .to_vertex(edge)
                .and_then(|vertex| mesh.vertex_position(vertex))
                .ok_or(Error::InvalidFace)?
                .map(f64::from);
            if p.iter().any(|v| !v.is_finite()) {
                return Err(Error::InvalidGeometry);
            }
            points.push(p);
        }
        if points.len() < 3 {
            return Err(Error::InvalidFace);
        }
        let mut triangles = Vec::new();
        if mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut triangles)
            || triangles.is_empty()
        {
            return Err(Error::InvalidFace);
        }
        let mut normal = [0.0; 3];
        for pair in points[1..].windows(2) {
            normal = add(
                normal,
                cross(sub(pair[0], points[0]), sub(pair[1], points[0])),
            );
        }
        normals.push(normalize(normal).ok_or(Error::InvalidGeometry)?);
        sum = add(sum, normal);
        all_points.extend(points);
    }
    if visited.len() != faces.len() {
        return Err(Error::AmbiguousSelection);
    }
    let z = normalize(sum).ok_or(Error::InvalidGeometry)?;
    if normals.iter().any(|&normal| dot(normal, z) <= 0.0) {
        return Err(Error::InvalidGeometry);
    }
    let reference = all_points[0];
    let mut max_plane_deviation = 0.0_f64;
    for &p in &all_points {
        let distance = dot(z, sub(p, reference)).abs();
        if !distance.is_finite() {
            return Err(Error::InvalidGeometry);
        }
        max_plane_deviation = max_plane_deviation.max(distance);
    }
    if max_plane_deviation > policy.distance_tolerance {
        return Err(Error::NonPlanar);
    }
    let offset = dot(z, sub(origin, reference));
    if !offset.is_finite() {
        return Err(Error::InvalidInput);
    }
    if offset.abs() > policy.distance_tolerance {
        return Err(Error::OriginOffPlane);
    }
    let origin = sub(origin, scale(z, offset));
    // Verify the plane that the rounded f64 frame actually represents, including
    // when an authored origin is far from the selected patch in the plane.
    max_plane_deviation = 0.0;
    for &p in &all_points {
        let distance = dot(z, sub(p, origin)).abs();
        if !distance.is_finite() || distance > policy.distance_tolerance {
            return Err(Error::InvalidGeometry);
        }
        max_plane_deviation = max_plane_deviation.max(distance);
    }
    // The transverse cross product measures sin(angle) directly. Subtracting
    // z * dot(x, z) can turn unit-length roundoff into an invented axis when
    // the authored direction is exactly parallel to z.
    let transverse = cross(z, authored_x);
    if norm(transverse) < policy.min_axis_sine {
        return Err(Error::ParallelAxis);
    }
    let y = normalize(transverse).ok_or(Error::ParallelAxis)?;
    let x = cross(y, z);
    Ok(Workplane {
        frame: Placement3::from_axes(x, y, z, origin),
        faces,
        revision: mesh.revision(),
        max_plane_deviation,
    })
}

#[cfg(test)]
mod tests;
