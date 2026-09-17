// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Authored coordinate frames on evaluated planar face patches.
//!
//! This module owns planar selection, authored frame validation, and semantic
//! attachment resolution. Origins and roll are authored explicitly; attachments
//! re-resolve surface intent instead of tracking transient faces across edits.
//! Frames are snapshots in body coordinates, pinned to one logical mesh revision.

use alloc::string::String;

use crate::edge_finish::OperandRegion;
use crate::ir::Placement3;
use crate::tessellate::{Feature, TessellatedBody};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use exedra_math::{add, cross, dot, norm, normalize, scale, sub};
use exedra_mesh::{FaceId, FaceTriangulation, Mesh, MeshRevision, attr};

mod inspection;
use inspection::{Rejection, analyze_patch, budget, components, selection_ambiguity};
pub use inspection::{
    SurfaceEntry, SurfaceInventory, SurfaceInventoryError, SurfaceInventoryPolicy,
    SurfaceInventoryStats, SurfacePatch, SurfacePlane, SurfaceResource, WorkplaneEvidence,
    WorkplaneFailure, inspect_surfaces,
};

/// The planar face patch whose winding determines the workplane's +Z.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkplaneSelection {
    /// One live face of the source mesh. IDs are scoped to that logical mesh.
    Face(FaceId),
    /// Faces retaining the authored start-cap feature. Missing provenance is
    /// an empty selection, never a guess based on position or region number.
    StartCap,
    /// Faces retaining the authored terminal-cap feature, including an
    /// extrusion terminated by a plane.
    EndCap,
    /// Surviving start cap of the uniquely named generating recipe node.
    SourceStartCap(String),
    /// Surviving terminal cap of the uniquely named generating recipe node.
    SourceEndCap(String),
    /// All faces carrying this region. They must be edge-connected and must
    /// not mix Boolean operands with reused region numbers.
    Region(u32),
    /// A region restricted to one producing Boolean operand.
    OperandRegion(OperandRegion),
}

/// A surface description that can be retained across recipe evaluations.
///
/// This value contains no mesh IDs. Resolve it afresh after edits. Caps follow
/// original feature provenance through Boolean splits, not extremal positions.
/// Source-qualified caps use opaque labels on generating nodes, not wrappers.
/// Labels must be unique in the recipe. Neither form stores transient mesh IDs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SurfaceSelector {
    /// The source operation's start cap.
    StartCap,
    /// The source operation's terminal cap.
    EndCap,
    /// Surviving start cap of the uniquely named generating recipe node.
    SourceStartCap(String),
    /// Surviving terminal cap of the uniquely named generating recipe node.
    SourceEndCap(String),
    /// An authored geometric region, which must resolve unambiguously.
    Region(u32),
    /// A region belonging to one operand of the producing Boolean operation.
    OperandRegion(OperandRegion),
}
impl SurfaceSelector {
    /// Authored generating-node label for a qualified cap selector.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        match self {
            Self::SourceStartCap(source) | Self::SourceEndCap(source) => Some(source),
            _ => None,
        }
    }

    /// Converts persistent surface intent to an evaluated workplane selection.
    #[must_use]
    pub fn selection(&self) -> WorkplaneSelection {
        match self {
            Self::StartCap => WorkplaneSelection::StartCap,
            Self::EndCap => WorkplaneSelection::EndCap,
            Self::Region(region) => WorkplaneSelection::Region(*region),
            Self::OperandRegion(region) => WorkplaneSelection::OperandRegion(*region),
            Self::SourceStartCap(source) => WorkplaneSelection::SourceStartCap(source.clone()),
            Self::SourceEndCap(source) => WorkplaneSelection::SourceEndCap(source.clone()),
        }
    }
}

/// Authored attachment intent, independently resolvable on each new body.
///
/// The origin is the intersection of the selected plane with the infinite line
/// `anchor + t * projection`. Both signs of `t` are allowed. All vectors use
/// body coordinates. The projected X direction controls roll; no axis is guessed.
/// The origin may lie outside the patch: use a planar clearance query to check
/// footprint containment. Resolution never matches a previous transient face ID.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkplaneAttachment {
    /// Surface to resolve uniquely on each evaluation.
    pub surface: SurfaceSelector,
    /// Point on the authored projection line.
    pub anchor: [f64; 3],
    /// Direction of that line, independent of vector magnitude.
    pub projection: [f64; 3],
    /// Preferred workplane X direction, projected into the selected plane.
    pub x_direction: [f64; 3],
}
impl WorkplaneAttachment {
    /// Resolves this intent against current topology and feature provenance.
    ///
    /// # Errors
    /// Rejects missing, ambiguous, nonplanar or stale surfaces, invalid authored
    /// vectors, near-parallel projection, and exhausted work budgets.
    pub fn resolve(
        &self,
        body: &TessellatedBody,
        policy: &WorkplanePolicy,
    ) -> Result<Workplane, WorkplaneFailure> {
        build_workplane(
            body,
            self.surface.selection(),
            self.anchor,
            Some(self.projection),
            self.x_direction,
            policy,
        )
    }

    pub(crate) fn valid(&self) -> bool {
        self.anchor.iter().all(|value| value.is_finite())
            && normalize(self.projection).is_some()
            && normalize(self.x_direction).is_some()
    }
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
    /// Minimum absolute cosine between an attachment projection and the face
    /// normal, in `(0, 1]`. Smaller values admit longer, less stable projections.
    pub min_projection_cos: f64,
    /// Maximum selected faces. Region resolution scans the body's face list.
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
    /// The source provenance or retained workplane no longer matches the mesh.
    StaleSource,
    /// No face matches the selection.
    EmptySelection,
    /// A selected face is stale, outside, malformed, or cannot be triangulated.
    InvalidFace,
    /// Selected faces are disconnected, source labels are duplicated, or
    /// Boolean operand identities are mixed. See the failure evidence.
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
/// budgets. Failures retain the requested selection, coarse category and
/// structured evidence. This does not certify the source as a solid or infer
/// curved-face tangent frames.
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
) -> Result<Workplane, WorkplaneFailure> {
    build_workplane(body, selection, origin, None, x_direction, policy)
}

fn build_workplane(
    body: &TessellatedBody,
    selection: WorkplaneSelection,
    origin: [f64; 3],
    projection: Option<[f64; 3]>,
    x_direction: [f64; 3],
    policy: &WorkplanePolicy,
) -> Result<Workplane, WorkplaneFailure> {
    build_inner(body, &selection, origin, projection, x_direction, policy)
        .map_err(|rejection| rejection.for_selection(selection))
}

fn build_inner(
    body: &TessellatedBody,
    selection: &WorkplaneSelection,
    origin: [f64; 3],
    projection: Option<[f64; 3]>,
    x_direction: [f64; 3],
    policy: &WorkplanePolicy,
) -> Result<Workplane, Rejection> {
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
    body.source_map
        .check(&body.mesh)
        .map_err(|_| Error::StaleSource)?;
    let mesh = &body.mesh;
    let regions = mesh.attrs().dense(attr::FACE_REGION);
    if let Some(error) = selection_ambiguity(body, selection, &[]) {
        return Err(error);
    }
    let matches = |face: FaceId| match selection {
        WorkplaneSelection::Face(wanted) => face == *wanted,
        WorkplaneSelection::StartCap => body
            .source_map
            .surface_origin(face)
            .is_some_and(|origin| origin.feature == Feature::CapStart),
        WorkplaneSelection::EndCap => body
            .source_map
            .surface_origin(face)
            .is_some_and(|origin| origin.feature == Feature::CapEnd),
        WorkplaneSelection::SourceStartCap(source) | WorkplaneSelection::SourceEndCap(source) => {
            let feature = if matches!(selection, WorkplaneSelection::SourceStartCap(_)) {
                Feature::CapStart
            } else {
                Feature::CapEnd
            };
            body.source_map.surface_origin(face).is_some_and(|origin| {
                origin.feature == feature && origin.source.as_deref() == Some(source.as_str())
            })
        }
        WorkplaneSelection::Region(region) => {
            regions.and_then(|r| r.get(face.into())) == Some(region)
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
        && (*face == FaceId::OUTSIDE || mesh.face_edge(*face).is_none())
    {
        return Err(Error::InvalidFace.into());
    }
    let mut faces = Vec::new();
    if let WorkplaneSelection::Face(face) = selection {
        faces.push(*face);
    } else {
        for face in mesh.faces().filter(|&face| matches(face)) {
            if faces.len() >= policy.max_faces as usize {
                return Err(budget(
                    SurfaceResource::SelectedFaces,
                    faces.len() as u64,
                    u64::from(policy.max_faces),
                ));
            }
            faces.push(face);
        }
    }
    if faces.is_empty() {
        return Err(Error::EmptySelection.into());
    }
    if let Some(error) = selection_ambiguity(body, selection, &faces) {
        return Err(error);
    }
    let mut corners = 0;
    let patches = components(mesh, &faces, &mut corners, u64::from(policy.max_corners))?;
    if patches.len() != 1 {
        return Err(Rejection(
            Error::AmbiguousSelection,
            WorkplaneEvidence::Disconnected {
                representatives: patches.iter().map(|patch| patch[0]).collect(),
            },
        ));
    }
    let geometry = analyze_patch(
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
    let mut max_plane_deviation = 0.0_f64;
    for &p in &all_points {
        let distance = dot(z, sub(p, origin)).abs();
        if !distance.is_finite() || distance > policy.distance_tolerance {
            return Err(Error::InvalidGeometry.into());
        }
        max_plane_deviation = max_plane_deviation.max(distance);
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

#[cfg(test)]
mod tests;
