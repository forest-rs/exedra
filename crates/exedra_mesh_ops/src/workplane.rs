// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Geometric planes and authored frames on explicit mesh-face selections.
//!
//! Source meaning and semantic selection belong to callers. Frames use mesh
//! coordinates, authored origins and X directions, and current face winding.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use exedra_math::{Placement3, add, cross, dot, norm, normalize, scale, sub};
use exedra_mesh::{FaceId, FaceTriangulation, Mesh, MeshRevision};

/// Accuracy and selected-patch work limits.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct WorkplanePolicy {
    /// Maximum vertex and authored-origin distance from the selected plane,
    /// in body units. Must be positive and finite.
    pub distance_tolerance: f64,
    /// Minimum sine of the angle between authored X and the face normal.
    /// Must lie in `(0, 1]`; directions closer to parallel are refused.
    pub min_axis_sine: f64,
    /// Minimum absolute cosine between an attachment projection and the face
    /// normal, in `(0, 1]`. Smaller values admit longer, less stable projections.
    pub min_projection_cos: f64,
    /// Maximum selected faces after canonicalization.
    pub max_faces: u32,
    /// Maximum corner visits across connectivity and geometry checks.
    /// A successful resolution visits each selected face corner twice.
    pub max_corners: u32,
}
impl Default for WorkplanePolicy {
    fn default() -> Self {
        Self {
            distance_tolerance: 1e-6,
            min_axis_sine: 1e-6,
            min_projection_cos: 1e-6,
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
    /// The retained workplane no longer matches the mesh revision.
    StaleSource,
    /// No face matches the selection.
    EmptySelection,
    /// A selected face is stale, outside, malformed, or cannot be triangulated.
    InvalidFace,
    /// Selected faces do not form one connected patch.
    AmbiguousSelection,
    /// Selected face corners do not lie on one plane within tolerance.
    NonPlanar,
    /// A face has zero area, incompatible winding, or unrepresentable geometry.
    InvalidGeometry,
    /// The authored origin is outside the plane's distance tolerance.
    OriginOffPlane,
    /// Authored X is too nearly parallel to the face normal.
    ParallelAxis,
    /// Attachment projection is too nearly parallel to the selected plane.
    ParallelProjection,
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
            Self::AmbiguousSelection => "workplane selection is ambiguous",
            Self::NonPlanar => "workplane selection is not planar within tolerance",
            Self::InvalidGeometry => "workplane geometry is degenerate or inconsistently oriented",
            Self::OriginOffPlane => "workplane origin is not on the selected plane",
            Self::ParallelProjection => "attachment projection is too parallel to the surface",
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
/// This value is bound to one logical mesh; revision equality
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

/// Geometric work resource exhausted while building a frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameResource {
    /// Selected faces.
    SelectedFaces,
    /// Face-corner visits, including repeated connectivity/geometry passes.
    Corners,
}

/// Measured geometric evidence explaining a frame failure.
#[derive(Clone, Debug, PartialEq)]
pub enum FrameEvidence {
    /// No additional numeric or topological evidence for this failure.
    None,
    /// Lowest face ID of each disconnected component.
    Disconnected {
        /// Component representatives, in ascending order.
        representatives: Vec<FaceId>,
    },
    /// Measured source corners lie farther from the plane than allowed.
    PlaneDeviation {
        /// Maximum observed distance.
        measured: f64,
        /// Caller-supplied tolerance.
        tolerance: f64,
    },
    /// The next work item exceeds an explicit limit.
    Budget {
        /// Exhausted resource.
        resource: FrameResource,
        /// Completed work before the refused next item.
        completed: u64,
        /// Authored limit.
        limit: u64,
    },
}

/// Typed geometric frame refusal, independent of semantic selectors.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameFailure {
    /// Stable coarse failure category.
    pub kind: WorkplaneError,
    /// Additional measured evidence.
    pub evidence: FrameEvidence,
}
impl FrameFailure {
    fn new(kind: WorkplaneError, evidence: FrameEvidence) -> Self {
        Self { kind, evidence }
    }
}
impl From<WorkplaneError> for FrameFailure {
    fn from(kind: WorkplaneError) -> Self {
        Self::new(kind, FrameEvidence::None)
    }
}
impl core::fmt::Display for FrameFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {:?}", self.kind, self.evidence)
    }
}
impl core::error::Error for FrameFailure {}
fn budget(resource: FrameResource, completed: u64, limit: u64) -> FrameFailure {
    FrameFailure::new(
        WorkplaneError::BudgetExceeded,
        FrameEvidence::Budget {
            resource,
            completed,
            limit,
        },
    )
}

/// Measures planarity and winding without choosing a frame or requiring that
/// selected faces be connected. Use [`connected_patches`] first when needed.
///
/// `corners` accumulates visits across this and previous calls, under `limit`.
/// Coordinates and distances use mesh units. No tessellation is performed beyond
/// checking the stored polygon faces' robust triangulation.
pub fn analyze_patch(
    mesh: &Mesh,
    faces: &[FaceId],
    tolerance: f64,
    corners: &mut u64,
    limit: u64,
) -> Result<SurfacePlane, FrameFailure> {
    analyze_geometry(mesh, faces, tolerance, corners, limit).map(|geometry| geometry.plane)
}

/// Resolves an authored frame on one connected planar face selection.
///
/// `origin`, optional line-of-projection direction, and `x_direction` use mesh
/// coordinates. Without projection, the origin must already lie within the
/// plane tolerance. With projection it is intersected along the authored line.
/// Face IDs are sorted and deduplicated. The frame need not lie inside material;
/// use a boundary/clearance query for containment. No automatic axes are chosen.
/// Failure returns no frame and leaves the source unchanged.
pub fn face_workplane(
    mesh: &Mesh,
    faces: &[FaceId],
    origin: [f64; 3],
    projection: Option<[f64; 3]>,
    x_direction: [f64; 3],
    policy: &WorkplanePolicy,
) -> Result<Workplane, FrameFailure> {
    use WorkplaneError as Error;
    if !policy.distance_tolerance.is_finite()
        || policy.distance_tolerance <= 0.0
        || !policy.min_axis_sine.is_finite()
        || policy.min_axis_sine <= 0.0
        || policy.min_axis_sine > 1.0
        || !policy.min_projection_cos.is_finite()
        || policy.min_projection_cos <= 0.0
        || policy.min_projection_cos > 1.0
        || policy.max_faces == 0
        || policy.max_corners == 0
        || origin.iter().any(|v| !v.is_finite())
    {
        return Err(Error::InvalidInput.into());
    }
    let authored_x = normalize(x_direction).ok_or(Error::InvalidInput)?;
    let mut faces: Vec<_> = faces.to_vec();
    faces.sort_unstable();
    faces.dedup();
    if faces.len() > policy.max_faces as usize {
        return Err(budget(
            FrameResource::SelectedFaces,
            u64::from(policy.max_faces),
            u64::from(policy.max_faces),
        ));
    }
    if faces.is_empty() {
        return Err(Error::EmptySelection.into());
    }
    let mut corners = 0;
    let patches = connected_patches(mesh, &faces, &mut corners, u64::from(policy.max_corners))?;
    if patches.len() != 1 {
        return Err(FrameFailure::new(
            Error::AmbiguousSelection,
            FrameEvidence::Disconnected {
                representatives: patches.iter().map(|patch| patch[0]).collect(),
            },
        ));
    }
    let geometry = analyze_geometry(
        mesh,
        &faces,
        policy.distance_tolerance,
        &mut corners,
        u64::from(policy.max_corners),
    )?;
    let z = geometry.plane.normal;
    let reference = geometry.plane.point;
    let all_points = geometry.points;
    let origin = if let Some(direction) = projection {
        let direction = normalize(direction).ok_or(Error::InvalidInput)?;
        let denominator = dot(z, direction);
        if denominator.abs() < policy.min_projection_cos {
            return Err(Error::ParallelProjection.into());
        }
        let distance = dot(z, sub(reference, origin)) / denominator;
        let projected = add(origin, scale(direction, distance));
        if projected.iter().any(|value| !value.is_finite()) {
            return Err(Error::InvalidGeometry.into());
        }
        projected
    } else {
        origin
    };
    let offset = dot(z, sub(origin, reference));
    if !offset.is_finite() {
        return Err(Error::InvalidInput.into());
    }
    if offset.abs() > policy.distance_tolerance {
        return Err(Error::OriginOffPlane.into());
    }
    let origin = sub(origin, scale(z, offset));
    // Verify the plane that the rounded f64 frame actually represents, including
    // when an authored origin is far from the selected patch in the plane.
    let max_plane_deviation =
        crate::planar::max_plane_deviation(&all_points, z, origin).ok_or(Error::InvalidGeometry)?;
    if max_plane_deviation > policy.distance_tolerance {
        return Err(Error::InvalidGeometry.into());
    }
    // The transverse cross product measures sin(angle) directly. Subtracting
    // z * dot(x, z) can turn unit-length roundoff into an invented axis when
    // the authored direction is exactly parallel to z.
    let transverse = cross(z, authored_x);
    if norm(transverse) < policy.min_axis_sine {
        return Err(Error::ParallelAxis.into());
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

/// Geometry of a checked planar patch, without choosing an attachment frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePlane {
    /// A measured point on the plane in body coordinates, not an authored anchor.
    pub point: [f64; 3],
    /// Unit normal following the patch's current face winding.
    pub normal: [f64; 3],
    /// Maximum source-corner distance from this plane, in body units.
    pub max_plane_deviation: f64,
}

struct PatchGeometry {
    pub plane: SurfacePlane,
    pub points: Vec<[f64; 3]>,
}

/// Shared geometric check; callers have already established connectedness.
fn analyze_geometry(
    mesh: &Mesh,
    faces: &[FaceId],
    tolerance: f64,
    corners: &mut u64,
    limit: u64,
) -> Result<PatchGeometry, FrameFailure> {
    use WorkplaneError as Error;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(Error::InvalidInput.into());
    }
    if faces.is_empty() {
        return Err(Error::EmptySelection.into());
    }
    let mut all_points = Vec::new();
    let mut normals = Vec::new();
    let mut sum = [0.0; 3];
    for &face in faces {
        if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
            return Err(Error::InvalidFace.into());
        }
        let mut points = Vec::new();
        for edge in mesh.face_loop(face) {
            if *corners >= limit {
                return Err(budget(FrameResource::Corners, *corners, limit));
            }
            *corners += 1;
            let p = mesh
                .to_vertex(edge)
                .and_then(|vertex| mesh.vertex_position(vertex))
                .ok_or(Error::InvalidFace)?
                .map(f64::from);
            if p.iter().any(|v| !v.is_finite()) {
                return Err(Error::InvalidGeometry.into());
            }
            points.push(p);
        }
        if points.len() < 3 {
            return Err(Error::InvalidFace.into());
        }
        let mut triangles = Vec::new();
        if mesh.face_triangles_into(face, FaceTriangulation::Robust, &mut triangles)
            || triangles.is_empty()
        {
            return Err(Error::InvalidFace.into());
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
    let normal = normalize(sum).ok_or(Error::InvalidGeometry)?;
    if normals.iter().any(|&n| dot(n, normal) <= 0.0) {
        return Err(Error::InvalidGeometry.into());
    }
    let point = *all_points.first().ok_or(Error::EmptySelection)?;
    let measured = crate::planar::max_plane_deviation(&all_points, normal, point)
        .ok_or(Error::InvalidGeometry)?;
    if measured > tolerance {
        return Err(FrameFailure::new(
            Error::NonPlanar,
            FrameEvidence::PlaneDeviation {
                measured,
                tolerance,
            },
        ));
    }
    Ok(PatchGeometry {
        plane: SurfacePlane {
            point,
            normal,
            max_plane_deviation: measured,
        },
        points: all_points,
    })
}

/// Partitions selected faces into patches connected by shared edges.
///
/// Patches and their faces are sorted by stable face ID; duplicate input faces
/// are ignored. `corners` is an accumulated work counter shared across calls;
/// the next corner is refused once `limit` is reached. No budget is reset here.
/// Stale/outside faces and broken adjacency return a typed failure.
pub fn connected_patches(
    mesh: &Mesh,
    faces: &[FaceId],
    corners: &mut u64,
    limit: u64,
) -> Result<Vec<Vec<FaceId>>, FrameFailure> {
    let mut remaining: BTreeSet<_> = faces.iter().copied().collect();
    let mut patches = Vec::new();
    while let Some(&first) = remaining.first() {
        remaining.remove(&first);
        let mut pending = alloc::vec![first];
        let mut patch = Vec::new();
        while let Some(face) = pending.pop() {
            if face == FaceId::OUTSIDE || mesh.face_edge(face).is_none() {
                return Err(WorkplaneError::InvalidFace.into());
            }
            patch.push(face);
            for edge in mesh.face_loop(face) {
                if *corners >= limit {
                    return Err(budget(FrameResource::Corners, *corners, limit));
                }
                *corners += 1;
                let adjacent = mesh
                    .twin(edge)
                    .and_then(|edge| mesh.face(edge))
                    .ok_or(WorkplaneError::InvalidFace)?;
                if remaining.remove(&adjacent) {
                    pending.push(adjacent);
                }
            }
        }
        patch.sort_unstable();
        patches.push(patch);
    }
    Ok(patches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Mesh {
        Mesh::from_polygons(
            &[
                [0.0, 0.0, 2.0],
                [4.0, 0.0, 2.0],
                [4.0, 3.0, 2.0],
                [0.0, 3.0, 2.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap()
    }

    #[test]
    fn direct_frame_projects_authored_origin_and_shares_corner_budget() {
        let mesh = quad();
        let faces: Vec<_> = mesh.faces().collect();
        let frame = face_workplane(
            &mesh,
            &faces,
            [1.0, 1.0, -5.0],
            Some([0.0, 0.0, 1.0]),
            [0.0, 1.0, 0.0],
            &WorkplanePolicy::default(),
        )
        .unwrap();
        assert_eq!(frame.to_body([0.0; 3]), [1.0, 1.0, 2.0]);
        assert_eq!(frame.to_body([1.0, 0.0, 0.0]), [1.0, 2.0, 2.0]);
        assert_eq!(frame.to_local([1.0, 2.0, 2.0]), [1.0, 0.0, 0.0]);
        let mut corners = 0;
        assert_eq!(
            connected_patches(&mesh, &faces, &mut corners, 7).unwrap(),
            alloc::vec![faces.clone()]
        );
        assert_eq!(corners, 4);
        assert_eq!(
            analyze_patch(&mesh, &faces, 1e-6, &mut corners, 7).unwrap_err(),
            budget(FrameResource::Corners, 7, 7)
        );
        assert_eq!(corners, 7);
        assert_eq!(
            face_workplane(
                &mesh,
                &faces,
                [0.0, 0.0, 2.0],
                None,
                [1.0, 0.0, 0.0],
                &WorkplanePolicy {
                    max_corners: 7,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            budget(FrameResource::Corners, 7, 7)
        );
    }

    #[test]
    fn direct_analysis_rejects_outside_and_nonfinite_tolerance() {
        let mesh = quad();
        let faces: Vec<_> = mesh.faces().collect();
        assert_eq!(
            analyze_patch(&mesh, &[FaceId::OUTSIDE], 1e-6, &mut 0, 100)
                .unwrap_err()
                .kind,
            WorkplaneError::InvalidFace
        );
        assert_eq!(
            analyze_patch(&mesh, &faces, f64::NAN, &mut 0, 100)
                .unwrap_err()
                .kind,
            WorkplaneError::InvalidInput
        );
        assert_eq!(
            face_workplane(
                &mesh,
                &faces,
                [0.0; 3],
                Some([1.0, 0.0, 0.0]),
                [1.0, 0.0, 0.0],
                &WorkplanePolicy::default()
            )
            .unwrap_err()
            .kind,
            WorkplaneError::ParallelProjection
        );
    }
}
