// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Authored coordinate frames on evaluated planar face patches.
//!
//! This module owns semantic surface selection and attachment resolution.
//! Mesh geometry and authored frame validation delegate to `exedra_mesh_ops`.
//! Origins and roll are authored explicitly; attachments re-resolve surface
//! intent instead of tracking transient faces across edits.
//! Frames are snapshots in body coordinates, pinned to one logical mesh revision.

use alloc::string::String;

use crate::edge_finish::OperandRegion;
use crate::tessellate::{Feature, TessellatedBody};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use exedra_math::normalize;
use exedra_mesh::{FaceId, Mesh, MeshRevision, attr};

mod inspection;
use inspection::{Rejection, budget, selection_ambiguity};
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

pub use exedra_mesh_ops::workplane::{Workplane, WorkplaneError, WorkplanePolicy};

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
    let _ = normalize(x_direction).ok_or(Error::InvalidInput)?;
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
    exedra_mesh_ops::workplane::face_workplane(
        mesh,
        &faces,
        origin,
        projection,
        x_direction,
        policy,
    )
    .map_err(Rejection::from)
}

#[cfg(test)]
mod tests;
