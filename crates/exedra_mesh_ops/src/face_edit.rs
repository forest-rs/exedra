// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Direct polygonal face extrusion, inset, cutting and solidification.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::{DeletePolicy, FaceId, VertexId, op};

use crate::math::FloatExt;
use crate::patch::attrs::{
    propagate_edge_attrs_for_vertices, propagate_face_corner_layers, propagate_face_corner_uvs,
    propagate_face_layers, propagate_frame_edge_attrs, set_face_edge_sharpness_for_vertices,
};
use crate::patch::connect::{FrameOrientationState, add_frame_face_with_orientation};
use crate::patch::duplicate::{create_vertex_copies, map_vertex_loop};
use crate::patch::geom::{centroid, normalized_face_normal};
use crate::patch::loops::{BoundaryLoopError, extract_boundary_loops};
use crate::patch::region::{SelectedFace, selected_face_region};
use crate::selection::{EdgeSet, FaceSet, canonicalize_face_set};
use exedra_math::{add, cross, distance_squared, dot, norm, normalize, scale, sub};

pub use crate::patch::attrs::SourceEdgeAttrs;
use crate::patch::source::{CaptureError, FaceInput, capture_inputs};

/// Refusal from face extrusion, inset, solidification or a rectangular cut.
#[derive(Clone, Debug, PartialEq)]
pub enum FaceEditError {
    /// Invalid or stale selected topology.
    Selection(exedra_mesh::SelectedFacePatchError),
    /// A selected face has missing or inconsistent geometry.
    InvalidFace {
        /// Offending face.
        face: FaceId,
    },
    /// No usable normal can be computed for a selected face.
    DegenerateFace {
        /// Offending face.
        face: FaceId,
    },
    /// Extrusion distance or shell thickness is not finite.
    InvalidDistance,
    /// Inset factor is not finite or not strictly between zero and one.
    InvalidInsetFactor,
    /// Keeping the source requires an open patch boundary.
    RequireOpenBoundary,
    /// Rectangle cutting currently requires one quad face.
    RequireQuad,
    /// Rectangle frame axes are not finite, nonzero and perpendicular.
    InvalidRectangleFrame,
    /// Rectangle cutting requires a planar source face.
    NonPlanarFace {
        /// Offending face.
        face: FaceId,
    },
    /// Rectangle origin or generated corners do not lie in the source plane.
    RectangleOffPlane,
    /// Rectangle axes do not lie in the source plane.
    RectangleAxesOffPlane,
    /// Rectangle extents are not finite and strictly increasing.
    InvalidRectangleExtents,
    /// The rectangle is not contained within the source face.
    RectangleOutsideFace,
    /// A position, normal or consumed attribute is not representable.
    NumericLimit,
    /// Source topology, positions, consumed attributes or revision changed.
    StalePreparation,
    /// The selected boundary cannot be traversed unambiguously.
    Boundary(exedra_mesh::SelectedFaceBoundaryError),
    /// Boundary traversal disagrees with selected patch topology.
    InvalidBoundary,
    /// Kernel refused source-face deletion. Earlier eager edits may remain.
    Delete(op::DeleteFacesError),
    /// Kernel refused a generated face. Earlier eager edits may remain.
    AddFace(op::AddFaceError),
    /// Kernel refused region attribution on a generated face.
    SetRegion(op::SetFaceRegionError),
    /// Generated topology is missing an expected twin.
    MissingBoundaryTwin,
}
impl core::fmt::Display for FaceEditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Selection(e) => write!(f, "face selection: {e}"),
            Self::InvalidFace { face } => write!(f, "invalid face {face:?}"),
            Self::DegenerateFace { face } => write!(f, "degenerate face {face:?}"),
            Self::InvalidDistance => f.write_str("face displacement must be finite"),
            Self::InvalidInsetFactor => {
                f.write_str("inset factor must be strictly between zero and one")
            }
            Self::RequireOpenBoundary => {
                f.write_str("keeping source faces requires an open patch boundary")
            }
            Self::RequireQuad => f.write_str("rectangle cutting requires a quad face"),
            Self::InvalidRectangleFrame => f.write_str("invalid rectangle frame axes"),
            Self::NonPlanarFace { face } => {
                write!(f, "rectangle cutting requires planar face {face:?}")
            }
            Self::RectangleOffPlane => f.write_str("rectangle is outside the face plane"),
            Self::RectangleAxesOffPlane => f.write_str("rectangle axes are outside the face plane"),
            Self::InvalidRectangleExtents => {
                f.write_str("rectangle extents must be finite and increasing")
            }
            Self::RectangleOutsideFace => f.write_str("rectangle is outside the selected face"),
            Self::NumericLimit => f.write_str("face edit exceeds numeric limits"),
            Self::StalePreparation => f.write_str("face preparation no longer matches its source"),
            Self::Boundary(e) => write!(f, "selected boundary: {e}"),
            Self::InvalidBoundary => f.write_str("selected boundary is inconsistent"),
            Self::Delete(e) => write!(f, "source face deletion: {e}"),
            Self::AddFace(e) => write!(f, "generated face insertion: {e}"),
            Self::SetRegion(e) => write!(f, "generated face region: {e}"),
            Self::MissingBoundaryTwin => f.write_str("generated boundary has no twin"),
        }
    }
}
impl core::error::Error for FaceEditError {}
impl From<CaptureError> for FaceEditError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Selection(error) => Self::Selection(error),
            CaptureError::NumericLimit { .. } => Self::NumericLimit,
        }
    }
}

/// Completed geometric work, independent of execution reports or clocks.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceEditStats {
    /// Selected source faces visited.
    pub faces_processed: u64,
    /// New vertices.
    pub vertices_created: u64,
    /// New faces.
    pub faces_created: u64,
    /// Removed source faces.
    pub faces_deleted: u64,
    /// Selection was sorted or deduplicated before use.
    pub selections_canonicalized: bool,
    /// Frame orientation reversed to fit the existing boundary direction.
    pub reversed_winding: bool,
}

/// Parameters for [`extrude_faces`].
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrudeFacesParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Extrude topology mode.
    pub mode: ExtrudeMode,
    /// Distance along each face normal.
    ///
    /// v0.1 semantics:
    /// - [`ExtrudeMode::ShellOpen`]: source faces are removed and replaced by
    ///   side walls + offset cap.
    /// - [`ExtrudeMode::KeepSource`]: source faces are kept; valid only when
    ///   selected patch boundary lies on mesh boundary.
    pub distance: f32,
}

/// Extrude topology mode.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum ExtrudeMode {
    /// Remove source faces and create open-shell extrusion.
    #[default]
    ShellOpen,
    /// Keep source faces and build a prism from an open-surface boundary.
    KeepSource,
}

/// Typed output from [`extrude_faces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtrudeFacesOutput {
    /// Created cap face IDs.
    pub cap_faces: FaceSet,
    /// Created side-wall face IDs.
    pub wall_faces: FaceSet,
}

impl Default for ExtrudeFacesParams {
    fn default() -> Self {
        Self {
            faces: FaceSet::default(),
            mode: ExtrudeMode::ShellOpen,
            distance: 1.0,
        }
    }
}

/// Parameters for [`inset_faces`].
#[derive(Clone, Debug, PartialEq)]
pub struct InsetFacesParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Inset interpolation factor toward face centroid (`0 < factor < 1`).
    ///
    /// `0.0` and `1.0` are rejected to avoid degenerate frame topology.
    pub factor: f32,
}

/// Typed output from [`inset_faces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InsetFacesOutput {
    /// Created inner face IDs.
    pub inner_faces: FaceSet,
    /// Created frame face IDs.
    pub frame_faces: FaceSet,
}

/// Parameters for [`solidify_faces`].
#[derive(Clone, Debug, PartialEq)]
pub struct SolidifyFacesParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Solidify mode controlling source-face retention.
    pub mode: SolidifyMode,
    /// Signed thickness along each selected face normal.
    pub thickness: f32,
}

/// Solidify source-face retention behavior.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum SolidifyMode {
    /// Keep source faces and create side walls + offset cap.
    ///
    /// This requires selected patch boundaries to lie on mesh boundary.
    #[default]
    KeepSource,
    /// Remove source faces and create open-shell topology.
    ShellOpen,
}

impl Default for SolidifyFacesParams {
    fn default() -> Self {
        Self {
            faces: FaceSet::default(),
            mode: SolidifyMode::KeepSource,
            thickness: 0.1,
        }
    }
}

/// Typed output from [`solidify_faces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolidifyFacesOutput {
    /// Created offset cap face IDs.
    pub cap_faces: FaceSet,
    /// Created side-wall face IDs.
    pub wall_faces: FaceSet,
}

/// Parameters for [`cut_rect_face`].
#[derive(Clone, Debug, PartialEq)]
pub struct CutRectFaceParams {
    /// Target source face to cut.
    pub face: FaceId,
    /// Rectangle frame origin in mesh coordinates, on the source plane.
    pub frame_origin: [f32; 3],
    /// Rectangle frame U axis in mesh coordinates; normalized during preparation.
    pub frame_u: [f32; 3],
    /// Rectangle frame V axis in mesh coordinates, perpendicular to U.
    /// Normalized during preparation, so extents use mesh distance units.
    pub frame_v: [f32; 3],
    /// Rectangle minimum extents in local UV coordinates.
    pub rect_min: [f32; 2],
    /// Rectangle maximum extents in local UV coordinates.
    pub rect_max: [f32; 2],
}

impl Default for CutRectFaceParams {
    fn default() -> Self {
        Self {
            face: FaceId::OUTSIDE,
            frame_origin: [0.0, 0.0, 0.0],
            frame_u: [1.0, 0.0, 0.0],
            frame_v: [0.0, 1.0, 0.0],
            rect_min: [0.25, 0.25],
            rect_max: [0.75, 0.75],
        }
    }
}

/// Deterministic compiled plan payload for [`cut_rect_face`].
#[derive(Clone, Debug)]
pub struct CutRectFacePlan {
    face: FaceId,
    outer_vertices: [VertexId; 4],
    inner_positions: [[f32; 3]; 4],
    source_edge_attrs: [SourceEdgeAttrs; 4],
    region: u32,
    inputs: Vec<FaceInput>,
    revision: exedra_mesh::MeshRevision,
}

/// Typed output from [`cut_rect_face`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CutRectFaceOutput {
    /// Created inner cut face ID.
    pub inner_faces: FaceSet,
    /// Created rectangular frame face IDs.
    pub frame_faces: FaceSet,
    /// Created inner-loop boundary edge IDs (frame-side half-edges).
    pub boundary_edges: EdgeSet,
}

/// Deterministic compiled plan payload for [`inset_faces`].
#[derive(Clone, Debug)]
pub struct InsetFacesPlan {
    faces: FaceSet,
    face_plans: Vec<SelectedFace>,
    factor: f32,
    selections_canonicalized: bool,
    inputs: Vec<FaceInput>,
    revision: exedra_mesh::MeshRevision,
}

impl Default for InsetFacesParams {
    fn default() -> Self {
        Self {
            faces: FaceSet::default(),
            factor: 0.2,
        }
    }
}

/// Extrudes selected faces in the caller's eager edit session.
///
/// Source regions and corner UVs transfer to caps/walls. Caller-defined layers
/// transfer under their own [`Propagation`](exedra_mesh::attributes::Propagation)
/// rules: face values from the source face, corner values from the source corner
/// at the same vertex, and vertex values onto the copied vertices. Edge tags follow
/// `propagate`; cap perimeters and wall columns are marked sharp. Normal overrides
/// do not transfer. Shared vertices move along the normalized
/// sum of incident selected face normals. The legacy zero-sum fallback is +Z.
/// Preflight failures precede mutation; kernel insertion/attribute failures can
/// leave partial edits. Finish the caller's change sink even after an error.
pub fn extrude_faces<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &ExtrudeFacesParams,
    propagate: &exedra_mesh::PropagatePolicy,
) -> Result<(FaceEditStats, ExtrudeFacesOutput), FaceEditError> {
    if !params.distance.is_finite() {
        return Err(FaceEditError::InvalidDistance);
    }
    let mut faces = params.faces.clone();
    let canonicalized = canonicalize_face_set(&mut faces);
    let region = selected_face_region(txn.mesh(), &faces, true)?;

    let mut stats = FaceEditStats::default();
    if canonicalized {
        stats.selections_canonicalized = true;
    }
    let mut cap_faces = Vec::<FaceId>::new();
    let mut wall_faces = Vec::<FaceId>::new();
    if params.mode == ExtrudeMode::KeepSource && !region.boundary_lies_on_mesh_boundary {
        return Err(FaceEditError::RequireOpenBoundary);
    }
    let mut summed_normals = BTreeMap::<VertexId, [f32; 3]>::new();
    for plan in &region.faces {
        for &vertex in &plan.vertices {
            let sum = summed_normals.entry(vertex).or_insert([0.0, 0.0, 0.0]);
            sum[0] += plan.normal[0];
            sum[1] += plan.normal[1];
            sum[2] += plan.normal[2];
        }
    }
    let mut cap_positions = BTreeMap::<VertexId, [f32; 3]>::new();
    for (&vertex, sum) in &summed_normals {
        let position = txn
            .mesh()
            .vertex_position(vertex)
            .expect("preflight-validated vertex must be live");
        let length_sq = sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2];
        let direction = if length_sq <= 1e-12 {
            [0.0, 0.0, 1.0]
        } else {
            let inv = 1.0 / length_sq.sqrt_ext();
            [sum[0] * inv, sum[1] * inv, sum[2] * inv]
        };
        let extruded = [
            position[0] + direction[0] * params.distance,
            position[1] + direction[1] * params.distance,
            position[2] + direction[2] * params.distance,
        ];
        cap_positions.insert(vertex, extruded);
    }
    if cap_positions.values().any(|p| !exedra_math::finite(*p)) {
        return Err(FaceEditError::NumericLimit);
    }
    let boundary_loops =
        extract_boundary_loops(txn.mesh(), &region).map_err(boundary_loop_error)?;

    let cap_vertices = create_vertex_copies(txn, &cap_positions);
    if params.mode == ExtrudeMode::ShellOpen {
        let faces_to_delete = region
            .faces
            .iter()
            .map(|plan| plan.face)
            .collect::<Vec<_>>();
        op::delete_faces(txn, &faces_to_delete, DeletePolicy::KeepIsolated)
            .map_err(FaceEditError::Delete)?;
    }
    let face_lookup = region
        .faces
        .iter()
        .map(|plan| (plan.face, plan))
        .collect::<BTreeMap<_, _>>();
    let mut wall_orientation = FrameOrientationState::default();
    for boundary_loop in &boundary_loops {
        for boundary in &boundary_loop.edges {
            let plan = *face_lookup
                .get(&boundary.face)
                .expect("boundary edge should belong to selected face");
            let current = boundary.from;
            let next = boundary.to;
            let current_cap = *cap_vertices
                .get(&current)
                .expect("cap vertex should exist for source vertex");
            let next_cap = *cap_vertices
                .get(&next)
                .expect("cap vertex should exist for source vertex");
            let wall = add_frame_face_with_orientation(
                txn,
                current,
                next,
                current_cap,
                next_cap,
                &mut wall_orientation,
            )
            .map_err(FaceEditError::AddFace)?;
            op::set_face_region(txn, wall, plan.region).map_err(FaceEditError::SetRegion)?;
            propagate_frame_edge_attrs(
                txn,
                wall,
                current,
                next,
                current_cap,
                next_cap,
                plan.edge_attrs[boundary.edge_index],
                propagate,
            );
            mark_frame_feature_edges_sharp(
                txn,
                wall,
                current,
                next,
                current_cap,
                next_cap,
                params.mode == ExtrudeMode::KeepSource
                    || txn
                        .mesh()
                        .twin(boundary.edge)
                        .and_then(|twin| txn.mesh().face(twin))
                        .is_some_and(|face| face != FaceId::OUTSIDE),
                true,
                true,
            );
            let uv_current = plan.vertex_uvs[boundary.edge_index];
            let uv_next = plan.vertex_uvs[(boundary.edge_index + 1) % plan.vertex_uvs.len()];
            let uv_map = [
                (current, uv_current),
                (next, uv_next),
                (current_cap, uv_current),
                (next_cap, uv_next),
            ];
            propagate_face_corner_uvs(txn, wall, &uv_map);
            let layers_current = &plan.corner_layers[boundary.edge_index];
            let layers_next =
                &plan.corner_layers[(boundary.edge_index + 1) % plan.corner_layers.len()];
            propagate_face_corner_layers(
                txn,
                wall,
                &[
                    (current, layers_current),
                    (next, layers_next),
                    (current_cap, layers_current),
                    (next_cap, layers_next),
                ],
            );
            propagate_face_layers(txn, wall, &plan.face_layers);
            wall_faces.push(wall);
        }
    }

    for plan in &region.faces {
        let mut cap_loop = map_vertex_loop(&plan.vertices, &cap_vertices);
        if !wall_orientation.prefers_forward_outer_edge() {
            cap_loop.reverse();
        }
        let top = op::add_face(txn, &cap_loop).map_err(FaceEditError::AddFace)?;
        op::set_face_region(txn, top, plan.region).map_err(FaceEditError::SetRegion)?;
        for i in 0..cap_loop.len() {
            let current_cap = cap_vertices[&plan.vertices[i]];
            let next_cap = cap_vertices[&plan.vertices[(i + 1) % plan.vertices.len()]];
            propagate_edge_attrs_for_vertices(
                txn,
                top,
                current_cap,
                next_cap,
                plan.edge_attrs[i],
                propagate,
            );
            set_face_edge_sharpness_for_vertices(txn, top, current_cap, next_cap, 1.0);
        }
        let cap_uv_map = plan
            .vertices
            .iter()
            .map(|vertex| cap_vertices[vertex])
            .zip(plan.vertex_uvs.iter().copied())
            .collect::<Vec<_>>();
        propagate_face_corner_uvs(txn, top, &cap_uv_map);
        let cap_layer_map = plan
            .vertices
            .iter()
            .map(|vertex| cap_vertices[vertex])
            .zip(plan.corner_layers.iter())
            .collect::<Vec<_>>();
        propagate_face_corner_layers(txn, top, &cap_layer_map);
        propagate_face_layers(txn, top, &plan.face_layers);
        cap_faces.push(top);
    }

    stats.faces_processed = u64::try_from(region.faces.len()).expect("face count should fit u64");
    stats.vertices_created =
        u64::try_from(cap_vertices.len()).expect("vertex count should fit u64");
    stats.faces_created =
        u64::try_from(wall_faces.len() + cap_faces.len()).expect("face count should fit u64");
    stats.faces_deleted = match params.mode {
        ExtrudeMode::ShellOpen => {
            u64::try_from(region.faces.len()).expect("face count should fit u64")
        }
        ExtrudeMode::KeepSource => 0,
    };
    stats.reversed_winding =
        !wall_orientation.prefers_forward_outer_edge() && !wall_faces.is_empty();
    Ok((
        stats,
        ExtrudeFacesOutput {
            cap_faces,
            wall_faces,
        },
    ))
}
impl InsetFacesPlan {
    /// Captures canonical selected topology and consumed attributes.
    pub fn prepare(
        mesh: &exedra_mesh::Mesh,
        params: &InsetFacesParams,
    ) -> Result<Self, FaceEditError> {
        if !params.factor.is_finite() || params.factor <= 0.0 || params.factor >= 1.0 {
            return Err(FaceEditError::InvalidInsetFactor);
        }
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        let region = selected_face_region(mesh, &faces, false)?;
        let inputs = capture_inputs(mesh, &faces)?;
        Ok(Self {
            faces,
            inputs,
            revision: mesh.revision(),
            face_plans: region.faces,
            factor: params.factor,
            selections_canonicalized: canonicalized,
        })
    }
    /// Canonical source faces, in output order.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        &self.faces
    }
    /// Authored interpolation factor.
    #[must_use]
    pub fn factor(&self) -> f32 {
        self.factor
    }
    /// Whether preparation sorted or deduplicated the selection.
    #[must_use]
    pub fn selections_canonicalized(&self) -> bool {
        self.selections_canonicalized
    }
    /// Applies the prepared inset after checking revision and exact consumed
    /// source state, including unfinished edits. Equivalent clones are accepted.
    /// Regions/UVs and edge tags transfer as for [`extrude_faces`]; only the inner
    /// perimeter is newly marked sharp. Kernel failures may leave partial edits;
    /// finish the caller's change sink after success or failure.
    pub fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        propagate: &exedra_mesh::PropagatePolicy,
    ) -> Result<(FaceEditStats, InsetFacesOutput), FaceEditError> {
        let plan = self;
        if txn.mesh().revision() != self.revision
            || capture_inputs(txn.mesh(), &self.faces).ok().as_ref() != Some(&self.inputs)
        {
            return Err(FaceEditError::StalePreparation);
        }

        let plans = plan.face_plans.clone();

        let mut stats = FaceEditStats::default();
        if plan.selections_canonicalized {
            stats.selections_canonicalized = true;
        }
        let mut inner_faces = Vec::<FaceId>::new();
        let mut frame_faces = Vec::<FaceId>::new();
        let region = selected_face_region(txn.mesh(), &plan.faces, false)?;
        let mut inset_target_sum = BTreeMap::<VertexId, [f32; 3]>::new();
        let mut inset_target_count = BTreeMap::<VertexId, u32>::new();
        for face_plan in &plans {
            let centroid = centroid(txn.mesh(), &face_plan.vertices).expect("preflight validated");
            for &vertex in &face_plan.vertices {
                let position = txn
                    .mesh()
                    .vertex_position(vertex)
                    .expect("preflight-validated vertex must be live");
                let inset = [
                    position[0] + (centroid[0] - position[0]) * plan.factor,
                    position[1] + (centroid[1] - position[1]) * plan.factor,
                    position[2] + (centroid[2] - position[2]) * plan.factor,
                ];
                let sum = inset_target_sum.entry(vertex).or_insert([0.0, 0.0, 0.0]);
                sum[0] += inset[0];
                sum[1] += inset[1];
                sum[2] += inset[2];
                *inset_target_count.entry(vertex).or_insert(0) += 1;
            }
        }
        let mut inset_positions = BTreeMap::<VertexId, [f32; 3]>::new();
        for (&vertex, sum) in &inset_target_sum {
            let count = inset_target_count
                .get(&vertex)
                .copied()
                .expect("inset count should exist");
            let inv = 1.0 / (count as f32);
            let averaged = [sum[0] * inv, sum[1] * inv, sum[2] * inv];
            inset_positions.insert(vertex, averaged);
        }
        if inset_positions.values().any(|p| !exedra_math::finite(*p)) {
            return Err(FaceEditError::NumericLimit);
        }
        let boundary_loops =
            extract_boundary_loops(txn.mesh(), &region).map_err(boundary_loop_error)?;

        let inset_vertices = create_vertex_copies(txn, &inset_positions);
        let faces_to_delete = plans.iter().map(|plan| plan.face).collect::<Vec<_>>();
        op::delete_faces(txn, &faces_to_delete, DeletePolicy::KeepIsolated)
            .map_err(FaceEditError::Delete)?;
        let plan_lookup = plans
            .iter()
            .map(|plan| (plan.face, plan))
            .collect::<BTreeMap<_, _>>();
        let mut frame_orientation = FrameOrientationState::default();
        for boundary_loop in &boundary_loops {
            for boundary in &boundary_loop.edges {
                let face_plan = *plan_lookup
                    .get(&boundary.face)
                    .expect("boundary edge should belong to selected face");
                let inset_loop = map_vertex_loop(&face_plan.vertices, &inset_vertices);
                let current = boundary.from;
                let next = boundary.to;
                let current_inset = inset_loop[boundary.edge_index];
                let next_inset = inset_loop[(boundary.edge_index + 1) % inset_loop.len()];
                let frame = add_frame_face_with_orientation(
                    txn,
                    current,
                    next,
                    current_inset,
                    next_inset,
                    &mut frame_orientation,
                )
                .map_err(FaceEditError::AddFace)?;
                op::set_face_region(txn, frame, face_plan.region)
                    .map_err(FaceEditError::SetRegion)?;
                propagate_frame_edge_attrs(
                    txn,
                    frame,
                    current,
                    next,
                    current_inset,
                    next_inset,
                    face_plan.edge_attrs[boundary.edge_index],
                    propagate,
                );
                mark_frame_feature_edges_sharp(
                    txn,
                    frame,
                    current,
                    next,
                    current_inset,
                    next_inset,
                    false,
                    true,
                    false,
                );
                let uv_current = face_plan.vertex_uvs[boundary.edge_index];
                let uv_next =
                    face_plan.vertex_uvs[(boundary.edge_index + 1) % face_plan.vertex_uvs.len()];
                let uv_map = [
                    (current, uv_current),
                    (next, uv_next),
                    (current_inset, uv_current),
                    (next_inset, uv_next),
                ];
                propagate_face_corner_uvs(txn, frame, &uv_map);
                let layers_current = &face_plan.corner_layers[boundary.edge_index];
                let layers_next = &face_plan.corner_layers
                    [(boundary.edge_index + 1) % face_plan.corner_layers.len()];
                propagate_face_corner_layers(
                    txn,
                    frame,
                    &[
                        (current, layers_current),
                        (next, layers_next),
                        (current_inset, layers_current),
                        (next_inset, layers_next),
                    ],
                );
                propagate_face_layers(txn, frame, &face_plan.face_layers);
                frame_faces.push(frame);
            }
        }

        for face_plan in &plans {
            let mut inner_loop = map_vertex_loop(&face_plan.vertices, &inset_vertices);
            if !frame_orientation.prefers_forward_outer_edge() {
                inner_loop.reverse();
            }
            let inner = op::add_face(txn, &inner_loop).map_err(FaceEditError::AddFace)?;
            op::set_face_region(txn, inner, face_plan.region).map_err(FaceEditError::SetRegion)?;
            for i in 0..inner_loop.len() {
                let current_inset = inset_vertices[&face_plan.vertices[i]];
                let next_inset =
                    inset_vertices[&face_plan.vertices[(i + 1) % face_plan.vertices.len()]];
                propagate_edge_attrs_for_vertices(
                    txn,
                    inner,
                    current_inset,
                    next_inset,
                    face_plan.edge_attrs[i],
                    propagate,
                );
                set_face_edge_sharpness_for_vertices(txn, inner, current_inset, next_inset, 1.0);
            }
            let inset_uv_map = face_plan
                .vertices
                .iter()
                .map(|vertex| inset_vertices[vertex])
                .zip(face_plan.vertex_uvs.iter().copied())
                .collect::<Vec<_>>();
            propagate_face_corner_uvs(txn, inner, &inset_uv_map);
            let inset_layer_map = face_plan
                .vertices
                .iter()
                .map(|vertex| inset_vertices[vertex])
                .zip(face_plan.corner_layers.iter())
                .collect::<Vec<_>>();
            propagate_face_corner_layers(txn, inner, &inset_layer_map);
            propagate_face_layers(txn, inner, &face_plan.face_layers);
            inner_faces.push(inner);
        }

        stats.faces_processed = u64::try_from(plans.len()).expect("face count should fit u64");
        stats.vertices_created =
            u64::try_from(inset_vertices.len()).expect("vertex count should fit u64");
        stats.faces_created = u64::try_from(frame_faces.len() + inner_faces.len())
            .expect("face count should fit u64");
        stats.faces_deleted = u64::try_from(plans.len()).expect("face count should fit u64");
        stats.reversed_winding =
            !frame_orientation.prefers_forward_outer_edge() && !frame_faces.is_empty();
        Ok((
            stats,
            InsetFacesOutput {
                inner_faces,
                frame_faces,
            },
        ))
    }
}
/// Prepares and applies a centroid-interpolated inset in an eager edit session.
/// This is not a constant-distance polygon offset. See [`InsetFacesPlan::apply`]
/// for propagation and failure semantics.
pub fn inset_faces<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &InsetFacesParams,
    propagate: &exedra_mesh::PropagatePolicy,
) -> Result<(FaceEditStats, InsetFacesOutput), FaceEditError> {
    InsetFacesPlan::prepare(txn.mesh(), params)?.apply(txn, propagate)
}
/// Thickens the selected patch using the same geometry as [`extrude_faces`].
/// Source retention is explicit; propagation and eager failure semantics are
/// identical to extrusion.
pub fn solidify_faces<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &SolidifyFacesParams,
    propagate: &exedra_mesh::PropagatePolicy,
) -> Result<(FaceEditStats, SolidifyFacesOutput), FaceEditError> {
    let (stats, output) = extrude_faces(
        txn,
        &ExtrudeFacesParams {
            faces: params.faces.clone(),
            mode: match params.mode {
                SolidifyMode::KeepSource => ExtrudeMode::KeepSource,
                SolidifyMode::ShellOpen => ExtrudeMode::ShellOpen,
            },
            distance: params.thickness,
        },
        propagate,
    )?;
    Ok((
        stats,
        SolidifyFacesOutput {
            cap_faces: output.cap_faces,
            wall_faces: output.wall_faces,
        },
    ))
}
impl CutRectFacePlan {
    /// Validates the source quad and prepares four inner corner positions.
    ///
    /// The source, frame origin and generated corners must lie within `1e-4`
    /// mesh units of the source plane. Normalized axes must be perpendicular
    /// to each other and the face normal within an absolute dot product of
    /// `1e-4`. Corners must lie strictly inside every directed source-edge
    /// half-plane, with edge/point cross-product magnitude greater than `1e-6`
    /// square mesh units. Boundary contact is refused, since it would create
    /// zero-width frame faces. For a concave quad, this limits the rectangle
    /// to the intersection of those inward half-planes.
    pub fn prepare(
        mesh: &exedra_mesh::Mesh,
        params: &CutRectFaceParams,
    ) -> Result<Self, FaceEditError> {
        if params.face == FaceId::OUTSIDE {
            return Err(FaceEditError::Selection(
                exedra_mesh::SelectedFacePatchError::OutsideFaceInSelection,
            ));
        }
        let Some(face_edge) = mesh.face_edge(params.face) else {
            return Err(FaceEditError::Selection(
                exedra_mesh::SelectedFacePatchError::StaleFace { face: params.face },
            ));
        };
        let corners = mesh.face_loop(params.face).collect::<Vec<_>>();
        if corners.len() != 4 {
            return Err(FaceEditError::RequireQuad);
        }
        let mut outer_vertices_vec = Vec::with_capacity(4);
        let mut outer_positions = [[0.0_f32; 3]; 4];
        let mut source_edge_attrs = [SourceEdgeAttrs::default(); 4];
        for (index, corner) in corners.iter().copied().enumerate() {
            let vertex = mesh
                .to_vertex(corner)
                .ok_or(FaceEditError::InvalidFace { face: params.face })?;
            let position = mesh
                .vertex_position(vertex)
                .ok_or(FaceEditError::InvalidFace { face: params.face })?;
            outer_vertices_vec.push(vertex);
            outer_positions[index] = *position;
            // A half-edge's corner stores its destination, so the segment
            // vertices[index] -> vertices[index+1] belongs to the next corner.
            let edge = corners[(index + 1) % corners.len()];
            source_edge_attrs[index] = SourceEdgeAttrs {
                seam: mesh.edge_seam(edge),
                sharpness: mesh.edge_sharpness(edge),
            };
        }
        let outer_vertices: [VertexId; 4] = outer_vertices_vec
            .try_into()
            .expect("quad preflight must collect exactly four vertices");
        let normal = normalized_face_normal(mesh, &outer_vertices)
            .ok_or(FaceEditError::DegenerateFace { face: params.face })?;
        let plane_distance = |point| dot(normal, sub(point, outer_positions[0]));
        if outer_positions.iter().any(|&point| {
            let distance = plane_distance(point);
            !distance.is_finite() || distance.abs() > 1e-4
        }) {
            return Err(FaceEditError::NonPlanarFace { face: params.face });
        }
        let u_len = norm(params.frame_u);
        let v_len = norm(params.frame_v);
        if !(u_len.is_finite() && v_len.is_finite() && u_len > 0.0 && v_len > 0.0) {
            return Err(FaceEditError::InvalidRectangleFrame);
        }
        let u = scale(params.frame_u, 1.0 / u_len);
        let v = scale(params.frame_v, 1.0 / v_len);
        if dot(u, v).abs() > 1e-4 {
            return Err(FaceEditError::InvalidRectangleFrame);
        }
        if dot(normal, u).abs() > 1e-4 || dot(normal, v).abs() > 1e-4 {
            return Err(FaceEditError::RectangleAxesOffPlane);
        }
        if !exedra_math::finite(params.frame_origin) {
            return Err(FaceEditError::NumericLimit);
        }
        let origin_distance = plane_distance(params.frame_origin);
        if !origin_distance.is_finite() || origin_distance.abs() > 1e-4 {
            return Err(FaceEditError::RectangleOffPlane);
        }
        if !params.rect_min[0].is_finite()
            || !params.rect_min[1].is_finite()
            || !params.rect_max[0].is_finite()
            || !params.rect_max[1].is_finite()
            || params.rect_min[0] >= params.rect_max[0]
            || params.rect_min[1] >= params.rect_max[1]
        {
            return Err(FaceEditError::InvalidRectangleExtents);
        }
        let rect_local = [
            [params.rect_min[0], params.rect_min[1]],
            [params.rect_max[0], params.rect_min[1]],
            [params.rect_max[0], params.rect_max[1]],
            [params.rect_min[0], params.rect_max[1]],
        ];
        let mut inner_positions = [[0.0_f32; 3]; 4];
        for (i, local) in rect_local.iter().copied().enumerate() {
            let point = add(
                params.frame_origin,
                add(scale(u, local[0]), scale(v, local[1])),
            );
            if !point.iter().all(|value| value.is_finite()) {
                return Err(FaceEditError::NumericLimit);
            }
            let distance = plane_distance(point);
            if !distance.is_finite() || distance.abs() > 1e-4 {
                return Err(FaceEditError::RectangleOffPlane);
            }
            inner_positions[i] = point;
        }

        let basis_u = normalize(sub(outer_positions[1], outer_positions[0]))
            .ok_or(FaceEditError::DegenerateFace { face: params.face })?;
        let basis_v = cross(normal, basis_u);
        let outer_2d = outer_positions
            .map(|p| project_to_basis(p, outer_positions[0], basis_u, basis_v))
            .to_vec();
        let inner_2d = inner_positions
            .map(|p| project_to_basis(p, outer_positions[0], basis_u, basis_v))
            .to_vec();
        for point in &inner_2d {
            if !point_in_convex_polygon_2d(*point, &outer_2d) {
                return Err(FaceEditError::RectangleOutsideFace);
            }
        }

        // Order rectangle corners to follow source face winding by nearest mapping.
        let mut best_perm = [0_usize, 1, 2, 3];
        let mut best_score = f32::INFINITY;
        for perm in permutations4() {
            let score = (0..4)
                .map(|i| distance_squared(outer_positions[i], inner_positions[perm[i]]))
                .sum::<f32>();
            if score < best_score {
                best_score = score;
                best_perm = perm;
            }
        }
        if !best_score.is_finite() {
            return Err(FaceEditError::NumericLimit);
        }
        let ordered_inner = [
            inner_positions[best_perm[0]],
            inner_positions[best_perm[1]],
            inner_positions[best_perm[2]],
            inner_positions[best_perm[3]],
        ];

        if mesh.face(face_edge) != Some(params.face) {
            return Err(FaceEditError::InvalidFace { face: params.face });
        }
        let region = mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .and_then(|layer| layer.get(params.face.as_id()).copied())
            .unwrap_or(0);

        Ok(Self {
            inputs: capture_inputs(mesh, &[params.face])?,
            revision: mesh.revision(),
            face: params.face,
            outer_vertices,
            inner_positions: ordered_inner,
            source_edge_attrs,
            region,
        })
    }
    /// Source quad to replace.
    #[must_use]
    pub fn face(&self) -> FaceId {
        self.face
    }
    /// Source vertices in captured corner order.
    #[must_use]
    pub fn outer_vertices(&self) -> &[VertexId; 4] {
        &self.outer_vertices
    }
    /// Prepared inner positions in source correspondence order.
    #[must_use]
    pub fn inner_positions(&self) -> &[[f32; 3]; 4] {
        &self.inner_positions
    }
    /// Captured tags for each segment `outer_vertices[i]` to
    /// `outer_vertices[(i + 1) % 4]`, independent of output winding.
    #[must_use]
    pub fn source_edge_attrs(&self) -> &[SourceEdgeAttrs; 4] {
        &self.source_edge_attrs
    }
    /// Source region assigned to generated faces.
    #[must_use]
    pub fn region(&self) -> u32 {
        self.region
    }
    /// Applies a prepared cut after checking revision and exact consumed state,
    /// including unfinished edits. Equivalent clones are accepted. Regions and
    /// boundary edge tags transfer. Caller-defined layers transfer under their
    /// own rules: the source face's values onto every generated face, and the
    /// source corners' values onto the frame corners at the outer vertices; the
    /// inner corners and vertices have no source and start empty. Normal
    /// overrides and UVs do not transfer. The opening perimeter is sharp. Kernel failures may leave partial
    /// edits; finish the caller's change sink after success or failure.
    pub fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        propagate: &exedra_mesh::PropagatePolicy,
    ) -> Result<(FaceEditStats, CutRectFaceOutput), FaceEditError> {
        let plan = self;
        if txn.mesh().revision() != self.revision
            || capture_inputs(txn.mesh(), &[self.face]).ok().as_ref() != Some(&self.inputs)
        {
            return Err(FaceEditError::StalePreparation);
        }

        let mut stats = FaceEditStats::default();

        let face_layers = txn.mesh().capture_attributes(&[(plan.face, 1.0)]);
        let outer_corner_layers = plan.outer_vertices.map(|vertex| {
            let corner = txn
                .mesh()
                .face_loop(plan.face)
                .find(|&corner| txn.mesh().to_vertex(corner) == Some(vertex));
            match corner {
                Some(corner) => txn.mesh().capture_attributes(&[(corner, 1.0)]),
                None => txn
                    .mesh()
                    .capture_attributes::<exedra_mesh::HalfEdgeId>(&[]),
            }
        });
        let mut inner_vertices = Vec::with_capacity(4);
        for position in plan.inner_positions {
            inner_vertices.push(op::add_vertex(txn, position));
        }
        let inner_vertices: [VertexId; 4] = inner_vertices
            .try_into()
            .expect("cut_rect should create exactly four inner vertices");

        op::delete_faces(txn, &[plan.face], DeletePolicy::KeepIsolated)
            .map_err(FaceEditError::Delete)?;

        let mut frame_faces = Vec::with_capacity(4);
        let mut frame_orientation = FrameOrientationState::default();
        for i in 0..4 {
            let current = plan.outer_vertices[i];
            let next = plan.outer_vertices[(i + 1) % 4];
            let current_inner = inner_vertices[i];
            let next_inner = inner_vertices[(i + 1) % 4];
            let frame_face = add_frame_face_with_orientation(
                txn,
                current,
                next,
                current_inner,
                next_inner,
                &mut frame_orientation,
            )
            .map_err(FaceEditError::AddFace)?;
            op::set_face_region(txn, frame_face, plan.region).map_err(FaceEditError::SetRegion)?;
            propagate_frame_edge_attrs(
                txn,
                frame_face,
                current,
                next,
                current_inner,
                next_inner,
                plan.source_edge_attrs[i],
                propagate,
            );
            mark_frame_feature_edges_sharp(
                txn,
                frame_face,
                current,
                next,
                current_inner,
                next_inner,
                false,
                true,
                false,
            );
            propagate_face_layers(txn, frame_face, &face_layers);
            propagate_face_corner_layers(
                txn,
                frame_face,
                &[
                    (current, &outer_corner_layers[i]),
                    (next, &outer_corner_layers[(i + 1) % 4]),
                ],
            );
            frame_faces.push(frame_face);
        }

        let mut inner_loop = inner_vertices.to_vec();
        if !frame_orientation.prefers_forward_outer_edge() {
            inner_loop.reverse();
        }
        let inner_face = op::add_face(txn, &inner_loop).map_err(FaceEditError::AddFace)?;
        op::set_face_region(txn, inner_face, plan.region).map_err(FaceEditError::SetRegion)?;
        propagate_face_layers(txn, inner_face, &face_layers);

        let mut boundary_edges = EdgeSet::with_capacity(4);
        for inner_corner in txn.mesh().face_loop(inner_face) {
            let Some(frame_side) = txn.mesh().twin(inner_corner) else {
                return Err(FaceEditError::MissingBoundaryTwin);
            };
            boundary_edges.push(frame_side);
        }
        let _ = crate::selection::canonicalize_edge_set(&mut boundary_edges);

        stats.faces_processed = 1;
        stats.vertices_created = 4;
        stats.faces_created = 5;
        stats.faces_deleted = 1;
        stats.reversed_winding = !frame_orientation.prefers_forward_outer_edge();
        Ok((
            stats,
            CutRectFaceOutput {
                inner_faces: vec![inner_face],
                frame_faces,
                boundary_edges,
            },
        ))
    }
}
/// Prepares and applies a rectangular inner face and four surrounding frame faces.
/// See [`CutRectFacePlan::apply`] for attribute and eager failure semantics.
pub fn cut_rect_face<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &CutRectFaceParams,
    propagate: &exedra_mesh::PropagatePolicy,
) -> Result<(FaceEditStats, CutRectFaceOutput), FaceEditError> {
    CutRectFacePlan::prepare(txn.mesh(), params)?.apply(txn, propagate)
}
fn project_to_basis(
    point: [f32; 3],
    origin: [f32; 3],
    basis_u: [f32; 3],
    basis_v: [f32; 3],
) -> [f32; 2] {
    let delta = sub(point, origin);
    [dot(delta, basis_u), dot(delta, basis_v)]
}

fn point_in_convex_polygon_2d(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut has_pos = false;
    let mut has_neg = false;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let edge = [b[0] - a[0], b[1] - a[1]];
        let rel = [point[0] - a[0], point[1] - a[1]];
        let cross = edge[0] * rel[1] - edge[1] * rel[0];
        if !cross.is_finite() || cross.abs() <= 1e-6 {
            return false;
        }
        if cross > 1e-6 {
            has_pos = true;
        } else if cross < -1e-6 {
            has_neg = true;
        }
        if has_pos && has_neg {
            return false;
        }
    }
    true
}

fn permutations4() -> [[usize; 4]; 24] {
    [
        [0, 1, 2, 3],
        [0, 1, 3, 2],
        [0, 2, 1, 3],
        [0, 2, 3, 1],
        [0, 3, 1, 2],
        [0, 3, 2, 1],
        [1, 0, 2, 3],
        [1, 0, 3, 2],
        [1, 2, 0, 3],
        [1, 2, 3, 0],
        [1, 3, 0, 2],
        [1, 3, 2, 0],
        [2, 0, 1, 3],
        [2, 0, 3, 1],
        [2, 1, 0, 3],
        [2, 1, 3, 0],
        [2, 3, 0, 1],
        [2, 3, 1, 0],
        [3, 0, 1, 2],
        [3, 0, 2, 1],
        [3, 1, 0, 2],
        [3, 1, 2, 0],
        [3, 2, 0, 1],
        [3, 2, 1, 0],
    ]
}

fn mark_frame_feature_edges_sharp<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    face: FaceId,
    current: VertexId,
    next: VertexId,
    current_inner: VertexId,
    next_inner: VertexId,
    mark_outer: bool,
    mark_inner: bool,
    mark_columns: bool,
) {
    if mark_outer {
        set_face_edge_sharpness_for_vertices(txn, face, current, next, 1.0);
    }
    if mark_inner {
        set_face_edge_sharpness_for_vertices(txn, face, current_inner, next_inner, 1.0);
    }
    if mark_columns {
        set_face_edge_sharpness_for_vertices(txn, face, current, current_inner, 1.0);
        set_face_edge_sharpness_for_vertices(txn, face, next, next_inner, 1.0);
    }
}

fn boundary_loop_error(error: BoundaryLoopError) -> FaceEditError {
    use exedra_mesh::SelectedFaceBoundaryError as E;
    match error {
        BoundaryLoopError::InvalidSelectedPatch => FaceEditError::InvalidBoundary,
        BoundaryLoopError::AmbiguousBoundaryVertex { vertex, candidates } => {
            FaceEditError::Boundary(E::AmbiguousBoundaryVertex { vertex, candidates })
        }
        BoundaryLoopError::OpenBoundaryChain { start, end } => {
            FaceEditError::Boundary(E::OpenBoundaryChain { start, end })
        }
    }
}

#[cfg(test)]
mod tests;
