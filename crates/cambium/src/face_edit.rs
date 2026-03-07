// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face-edit operators (extrude, inset, cut, solidify).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use exedra::op::AddFaceError;
use exedra::{DeletePolicy, EdgeAttrPropagation, FaceId, HalfEdgeId, VertexId, op};

use crate::math::FloatExt;
use crate::op_common::op_error;
use crate::patch::region::{SelectedFace, selected_face_region};
use crate::plan::PlanHasher;
use crate::selection::{EdgeSet, FaceSet, canonicalize_face_set};
use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError, OpErrorKind,
    OpReport,
};

/// Parameters for [`ExtrudeFaces`].
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

/// Typed output from [`ExtrudeFaces`].
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

/// Parameters for [`InsetFaces`].
#[derive(Clone, Debug, PartialEq)]
pub struct InsetFacesParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Inset interpolation factor toward face centroid (`0 < factor < 1`).
    ///
    /// `0.0` and `1.0` are rejected to avoid degenerate frame topology.
    pub factor: f32,
}

/// Typed output from [`InsetFaces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InsetFacesOutput {
    /// Created inner face IDs.
    pub inner_faces: FaceSet,
    /// Created frame face IDs.
    pub frame_faces: FaceSet,
}

/// Parameters for [`SolidifyFaces`].
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

/// Typed output from [`SolidifyFaces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolidifyFacesOutput {
    /// Created offset cap face IDs.
    pub cap_faces: FaceSet,
    /// Created side-wall face IDs.
    pub wall_faces: FaceSet,
}

/// Parameters for [`CutRectFace`].
#[derive(Clone, Debug, PartialEq)]
pub struct CutRectFaceParams {
    /// Target source face to cut.
    pub face: FaceId,
    /// Rectangle frame origin in world space.
    pub frame_origin: [f32; 3],
    /// Rectangle frame U axis in world space.
    pub frame_u: [f32; 3],
    /// Rectangle frame V axis in world space.
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

/// Deterministic compiled plan payload for [`CutRectFace`].
#[derive(Clone, Debug)]
pub struct CutRectFacePlan {
    face: FaceId,
    outer_vertices: [VertexId; 4],
    inner_positions: [[f32; 3]; 4],
    source_edge_attrs: [SourceEdgeAttrs; 4],
    region: u32,
}

/// Typed output from [`CutRectFace`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CutRectFaceOutput {
    /// Created inner cut face ID.
    pub inner_faces: FaceSet,
    /// Created rectangular frame face IDs.
    pub frame_faces: FaceSet,
    /// Created inner-loop boundary edge IDs (frame-side half-edges).
    pub boundary_edges: EdgeSet,
}

/// Deterministic compiled plan payload for [`InsetFaces`].
#[derive(Clone, Debug)]
pub struct InsetFacesPlan {
    faces: FaceSet,
    face_plans: Vec<SelectedFace>,
    factor: f32,
    selections_canonicalized: bool,
}

impl Default for InsetFacesParams {
    fn default() -> Self {
        Self {
            faces: FaceSet::default(),
            factor: 0.2,
        }
    }
}

/// `edit.face.extrude` operator.
///
/// v0.1 propagation behavior:
/// - `face.region` is copied from source face to generated walls/cap,
/// - generated corner UVs are copied from source per-vertex UVs when authored,
/// - generated edge seam/sharpness follow [`OpContext::policy`](crate::OpContext::policy)
///   `propagate.edge_attr` for boundary-parallel edges; support edges default clear.
///
/// Mode behavior:
/// - [`ExtrudeMode::ShellOpen`] removes source faces before creating walls/caps.
/// - [`ExtrudeMode::KeepSource`] preserves source faces and requires all patch
///   boundary edges to be mesh-boundary edges.
#[derive(Copy, Clone, Debug, Default)]
pub struct ExtrudeFaces;

impl EditOperator for ExtrudeFaces {
    type Params = ExtrudeFacesParams;
    type Plan = ExtrudeFacesParams;
    type Output = ExtrudeFacesOutput;

    fn name(&self) -> &'static str {
        "edit.face.extrude"
    }

    fn apply<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        if !params.distance.is_finite() {
            return Err(op_error(
                ctx,
                OpErrorKind::NumericFailure,
                DiagCode::NumericToleranceIssue,
                "extrude distance must be finite",
            ));
        }
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        let region = selected_face_region(txn.mesh(), &faces, true, ctx)?;

        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }
        let mut cap_faces = Vec::<FaceId>::new();
        let mut wall_faces = Vec::<FaceId>::new();
        if params.mode == ExtrudeMode::KeepSource && !region.boundary_lies_on_mesh_boundary {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "extrude KeepSource requires selected patch boundary to lie on mesh boundary",
            ));
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
        let mut cap_vertices = BTreeMap::<VertexId, VertexId>::new();
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
            cap_vertices.insert(vertex, op::add_vertex(txn, extruded));
        }

        if params.mode == ExtrudeMode::ShellOpen {
            let faces_to_delete = region
                .faces
                .iter()
                .map(|plan| plan.face)
                .collect::<Vec<_>>();
            op::delete_faces(txn, &faces_to_delete, DeletePolicy::KeepIsolated).map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("extrude delete failed unexpectedly: {err}"),
                )
            })?;
        }

        let mut wall_orientation = FrameOrientationState::default();
        for plan in &region.faces {
            for i in 0..plan.vertices.len() {
                let current = plan.vertices[i];
                let next = plan.vertices[(i + 1) % plan.vertices.len()];
                if !region
                    .boundary_edges
                    .iter()
                    .any(|edge| edge.face == plan.face && edge.from == current && edge.to == next)
                {
                    continue;
                }
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
                    ctx,
                    "extrude",
                )?;
                if op::set_face_region(txn, wall, plan.region).is_err() {
                    return Err(op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        "failed to set extrude wall region",
                    ));
                }
                propagate_frame_edge_attrs(
                    txn,
                    wall,
                    current,
                    next,
                    current_cap,
                    next_cap,
                    plan.edge_attrs[i],
                    &ctx.policy.propagate,
                );
                let uv_current = plan.vertex_uvs[i];
                let uv_next = plan.vertex_uvs[(i + 1) % plan.vertex_uvs.len()];
                let uv_map = [
                    (current, uv_current),
                    (next, uv_next),
                    (current_cap, uv_current),
                    (next_cap, uv_next),
                ];
                propagate_face_corner_uvs(txn, wall, &uv_map);
                wall_faces.push(wall);
            }
        }

        for plan in &region.faces {
            let mut cap_loop = plan
                .vertices
                .iter()
                .map(|vertex| {
                    *cap_vertices
                        .get(vertex)
                        .expect("cap vertex should exist for source vertex")
                })
                .collect::<Vec<_>>();
            if !wall_orientation.prefers_forward_outer_edge() {
                cap_loop.reverse();
            }
            let top = op::add_face(txn, &cap_loop).map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("extrude cap creation failed unexpectedly: {err}"),
                )
            })?;
            if op::set_face_region(txn, top, plan.region).is_err() {
                return Err(op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    "failed to set extrude cap region",
                ));
            }
            for i in 0..cap_loop.len() {
                let current_cap = cap_loop[i];
                let next_cap = cap_loop[(i + 1) % cap_loop.len()];
                propagate_edge_attrs_for_vertices(
                    txn,
                    top,
                    current_cap,
                    next_cap,
                    plan.edge_attrs[i],
                    &ctx.policy.propagate,
                );
            }
            let cap_uv_map = cap_loop
                .iter()
                .copied()
                .zip(plan.vertex_uvs.iter().copied())
                .collect::<Vec<_>>();
            propagate_face_corner_uvs(txn, top, &cap_uv_map);
            cap_faces.push(top);
        }

        report.stats.counters.faces_processed =
            u64::try_from(region.faces.len()).expect("face count should fit u64");
        report.stats.elements_created.vertices =
            u64::try_from(cap_vertices.len()).expect("vertex count should fit u64");
        report.stats.elements_created.faces =
            u64::try_from(wall_faces.len() + cap_faces.len()).expect("face count should fit u64");
        report.stats.elements_deleted.faces = match params.mode {
            ExtrudeMode::ShellOpen => {
                u64::try_from(region.faces.len()).expect("face count should fit u64")
            }
            ExtrudeMode::KeepSource => 0,
        };
        Ok((
            report,
            ExtrudeFacesOutput {
                cap_faces,
                wall_faces,
            },
        ))
    }

    fn compile(
        &self,
        _mesh: &exedra::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        self.apply(txn, plan, ctx)
    }
}

/// `edit.face.inset` operator.
///
/// v0.1 propagation behavior:
/// - `face.region` is copied from source face to generated frame/inner faces,
/// - generated corner UVs are copied from source per-vertex UVs when authored,
/// - generated edge seam/sharpness follow [`OpContext::policy`](crate::OpContext::policy)
///   `propagate.edge_attr` for boundary-parallel edges; support edges default clear.
#[derive(Copy, Clone, Debug, Default)]
pub struct InsetFaces;

impl EditOperator for InsetFaces {
    type Params = InsetFacesParams;
    type Plan = InsetFacesPlan;
    type Output = InsetFacesOutput;

    fn name(&self) -> &'static str {
        "edit.face.inset"
    }

    fn compile(
        &self,
        mesh: &exedra::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        if !params.factor.is_finite() || !(0.0..1.0).contains(&params.factor) {
            return Err(op_error(
                ctx,
                OpErrorKind::NumericFailure,
                DiagCode::NumericToleranceIssue,
                "inset factor must be finite and satisfy 0 < factor < 1",
            ));
        }
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        let region = selected_face_region(mesh, &faces, false, ctx)?;
        Ok(InsetFacesPlan {
            faces,
            face_plans: region.faces,
            factor: params.factor,
            selections_canonicalized: canonicalized,
        })
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let plans = plan.face_plans.clone();

        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if plan.selections_canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }
        let mut inner_faces = Vec::<FaceId>::new();
        let mut frame_faces = Vec::<FaceId>::new();
        let region = selected_face_region(txn.mesh(), &plan.faces, false, ctx)?;
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
        let mut inset_vertices = BTreeMap::<VertexId, VertexId>::new();
        for (&vertex, sum) in &inset_target_sum {
            let count = inset_target_count
                .get(&vertex)
                .copied()
                .expect("inset count should exist");
            let inv = 1.0 / (count as f32);
            let averaged = [sum[0] * inv, sum[1] * inv, sum[2] * inv];
            inset_vertices.insert(vertex, op::add_vertex(txn, averaged));
        }

        let faces_to_delete = plans.iter().map(|plan| plan.face).collect::<Vec<_>>();
        op::delete_faces(txn, &faces_to_delete, DeletePolicy::KeepIsolated).map_err(|err| {
            op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                format!("inset delete failed unexpectedly: {err}"),
            )
        })?;

        let mut frame_orientation = FrameOrientationState::default();
        for face_plan in &plans {
            let inset_loop = face_plan
                .vertices
                .iter()
                .map(|vertex| {
                    *inset_vertices
                        .get(vertex)
                        .expect("inset vertex should exist for source vertex")
                })
                .collect::<Vec<_>>();
            for i in 0..face_plan.vertices.len() {
                let current = face_plan.vertices[i];
                let next = face_plan.vertices[(i + 1) % face_plan.vertices.len()];
                if !region.boundary_edges.iter().any(|edge| {
                    edge.face == face_plan.face && edge.from == current && edge.to == next
                }) {
                    continue;
                }
                let current_inset = inset_loop[i];
                let next_inset = inset_loop[(i + 1) % inset_loop.len()];
                let frame = add_frame_face_with_orientation(
                    txn,
                    current,
                    next,
                    current_inset,
                    next_inset,
                    &mut frame_orientation,
                    ctx,
                    "inset",
                )?;
                if op::set_face_region(txn, frame, face_plan.region).is_err() {
                    return Err(op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        "failed to set inset frame face region",
                    ));
                }
                propagate_frame_edge_attrs(
                    txn,
                    frame,
                    current,
                    next,
                    current_inset,
                    next_inset,
                    face_plan.edge_attrs[i],
                    &ctx.policy.propagate,
                );
                let uv_current = face_plan.vertex_uvs[i];
                let uv_next = face_plan.vertex_uvs[(i + 1) % face_plan.vertex_uvs.len()];
                let uv_map = [
                    (current, uv_current),
                    (next, uv_next),
                    (current_inset, uv_current),
                    (next_inset, uv_next),
                ];
                propagate_face_corner_uvs(txn, frame, &uv_map);
                frame_faces.push(frame);
            }
        }

        for face_plan in &plans {
            let mut inner_loop = face_plan
                .vertices
                .iter()
                .map(|vertex| {
                    *inset_vertices
                        .get(vertex)
                        .expect("inset vertex should exist for source vertex")
                })
                .collect::<Vec<_>>();
            if !frame_orientation.prefers_forward_outer_edge() {
                inner_loop.reverse();
            }
            let inner = op::add_face(txn, &inner_loop).map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("inset inner face creation failed unexpectedly: {err}"),
                )
            })?;
            if op::set_face_region(txn, inner, face_plan.region).is_err() {
                return Err(op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    "failed to set inset inner face region",
                ));
            }
            for i in 0..inner_loop.len() {
                let current_inset = inner_loop[i];
                let next_inset = inner_loop[(i + 1) % inner_loop.len()];
                propagate_edge_attrs_for_vertices(
                    txn,
                    inner,
                    current_inset,
                    next_inset,
                    face_plan.edge_attrs[i],
                    &ctx.policy.propagate,
                );
            }
            let inset_uv_map = inner_loop
                .iter()
                .copied()
                .zip(face_plan.vertex_uvs.iter().copied())
                .collect::<Vec<_>>();
            propagate_face_corner_uvs(txn, inner, &inset_uv_map);
            inner_faces.push(inner);
        }

        report.stats.counters.faces_processed =
            u64::try_from(plans.len()).expect("face count should fit u64");
        report.stats.elements_created.vertices =
            u64::try_from(inset_vertices.len()).expect("vertex count should fit u64");
        report.stats.elements_created.faces = u64::try_from(frame_faces.len() + inner_faces.len())
            .expect("face count should fit u64");
        report.stats.elements_deleted.faces =
            u64::try_from(plans.len()).expect("face count should fit u64");
        Ok((
            report,
            InsetFacesOutput {
                inner_faces,
                frame_faces,
            },
        ))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_face_set(&plan.faces);
        hasher.write_f32_bits(plan.factor);
        hasher.write_u8(u8::from(plan.selections_canonicalized));
        hasher.finish()
    }
}

/// `edit.face.solidify` operator.
///
/// This is a dedicated shell-thickness operator built on the face-edit kernel
/// path used by [`ExtrudeFaces`], with a stable user-facing name and defaults.
#[derive(Copy, Clone, Debug, Default)]
pub struct SolidifyFaces;

impl EditOperator for SolidifyFaces {
    type Params = SolidifyFacesParams;
    type Plan = ExtrudeFacesParams;
    type Output = SolidifyFacesOutput;

    fn name(&self) -> &'static str {
        "edit.face.solidify"
    }

    fn compile(
        &self,
        mesh: &exedra::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        let extrude_mode = match params.mode {
            SolidifyMode::KeepSource => ExtrudeMode::KeepSource,
            SolidifyMode::ShellOpen => ExtrudeMode::ShellOpen,
        };
        let mapped = ExtrudeFacesParams {
            faces: params.faces.clone(),
            mode: extrude_mode,
            distance: params.thickness,
        };
        let op = ExtrudeFaces;
        op.compile(mesh, &mapped, ctx)
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let op = ExtrudeFaces;
        let (mut report, output) = op.apply_plan(txn, plan, ctx)?;
        report.name = self.name();
        Ok((
            report,
            SolidifyFacesOutput {
                cap_faces: output.cap_faces,
                wall_faces: output.wall_faces,
            },
        ))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_face_set(&plan.faces);
        hasher.write_u8(match plan.mode {
            ExtrudeMode::ShellOpen => 0,
            ExtrudeMode::KeepSource => 1,
        });
        hasher.write_f32_bits(plan.distance);
        hasher.finish()
    }
}

/// `edit.face.cut.rect` operator.
///
/// Cuts one rectangular inner face from a source quad face using an explicit
/// world-space frame and rectangle extents.
///
/// v0.1 scope:
/// - supports one live quad face,
/// - requires rectangle corners to lie inside the source face,
/// - propagates `face.region` and source outer-edge seam/sharpness to the
///   corresponding frame outer edges.
#[derive(Copy, Clone, Debug, Default)]
pub struct CutRectFace;

impl EditOperator for CutRectFace {
    type Params = CutRectFaceParams;
    type Plan = CutRectFacePlan;
    type Output = CutRectFaceOutput;

    fn name(&self) -> &'static str {
        "edit.face.cut.rect"
    }

    fn compile(
        &self,
        mesh: &exedra::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        if params.face == FaceId::OUTSIDE {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "cut_rect requires one interior source face",
            ));
        }
        let Some(face_edge) = mesh.face_edge(params.face) else {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!("cut_rect source face is stale: {}", params.face.index()),
            ));
        };
        let corners = mesh.face_loop(params.face).collect::<Vec<_>>();
        if corners.len() != 4 {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "cut_rect currently supports quad faces only",
            ));
        }
        let mut outer_vertices_vec = Vec::with_capacity(4);
        let mut outer_positions = [[0.0_f32; 3]; 4];
        let mut source_edge_attrs = [SourceEdgeAttrs::default(); 4];
        for (index, corner) in corners.iter().copied().enumerate() {
            let vertex = mesh.to_vertex(corner).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "cut_rect encountered corner with missing destination vertex",
                )
            })?;
            let position = mesh.vertex_position(vertex).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "cut_rect encountered vertex with missing position",
                )
            })?;
            outer_vertices_vec.push(vertex);
            outer_positions[index] = *position;
            source_edge_attrs[index] = SourceEdgeAttrs {
                seam: mesh.edge_seam(corner),
                sharpness: mesh.edge_sharpness(corner),
            };
        }
        let outer_vertices: [VertexId; 4] = outer_vertices_vec
            .try_into()
            .expect("quad preflight must collect exactly four vertices");
        let normal = normalized_face_normal(mesh, &outer_vertices).ok_or_else(|| {
            op_error(
                ctx,
                OpErrorKind::NumericFailure,
                DiagCode::NumericToleranceIssue,
                "cut_rect requires non-degenerate source face",
            )
        })?;
        let u_len = length3(params.frame_u);
        let v_len = length3(params.frame_v);
        if !(u_len.is_finite() && v_len.is_finite() && u_len > 0.0 && v_len > 0.0) {
            return Err(op_error(
                ctx,
                OpErrorKind::NumericFailure,
                DiagCode::NumericToleranceIssue,
                "cut_rect frame axes must be finite non-zero vectors",
            ));
        }
        let u = scale3(params.frame_u, 1.0 / u_len);
        let v = scale3(params.frame_v, 1.0 / v_len);
        if dot3(normal, u).abs() > 1e-4 || dot3(normal, v).abs() > 1e-4 {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "cut_rect frame axes must lie in source-face plane",
            ));
        }
        if !params.rect_min[0].is_finite()
            || !params.rect_min[1].is_finite()
            || !params.rect_max[0].is_finite()
            || !params.rect_max[1].is_finite()
            || params.rect_min[0] >= params.rect_max[0]
            || params.rect_min[1] >= params.rect_max[1]
        {
            return Err(op_error(
                ctx,
                OpErrorKind::NumericFailure,
                DiagCode::NumericToleranceIssue,
                "cut_rect rect_min/rect_max must be finite and strictly ordered",
            ));
        }
        let rect_local = [
            [params.rect_min[0], params.rect_min[1]],
            [params.rect_max[0], params.rect_min[1]],
            [params.rect_max[0], params.rect_max[1]],
            [params.rect_min[0], params.rect_max[1]],
        ];
        let mut inner_positions = [[0.0_f32; 3]; 4];
        for (i, local) in rect_local.iter().copied().enumerate() {
            let point = add3(
                params.frame_origin,
                add3(scale3(u, local[0]), scale3(v, local[1])),
            );
            if !point.iter().all(|value| value.is_finite()) {
                return Err(op_error(
                    ctx,
                    OpErrorKind::NumericFailure,
                    DiagCode::NumericToleranceIssue,
                    "cut_rect computed non-finite corner position",
                ));
            }
            inner_positions[i] = point;
        }

        let basis_u =
            normalize3(sub3(outer_positions[1], outer_positions[0])).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::NumericFailure,
                    DiagCode::NumericToleranceIssue,
                    "cut_rect source face has degenerate edge",
                )
            })?;
        let basis_v = cross3(normal, basis_u);
        let outer_2d = outer_positions
            .map(|p| project_to_basis(p, outer_positions[0], basis_u, basis_v))
            .to_vec();
        let inner_2d = inner_positions
            .map(|p| project_to_basis(p, outer_positions[0], basis_u, basis_v))
            .to_vec();
        for point in &inner_2d {
            if !point_in_convex_polygon_2d(*point, &outer_2d) {
                return Err(op_error(
                    ctx,
                    OpErrorKind::PreconditionFailed,
                    DiagCode::PreconditionFailed,
                    "cut_rect rectangle must lie fully inside source face",
                ));
            }
        }

        // Order rectangle corners to follow source face winding by nearest mapping.
        let mut best_perm = [0_usize, 1, 2, 3];
        let mut best_score = f32::INFINITY;
        for perm in permutations4() {
            let score = (0..4)
                .map(|i| distance_sq3(outer_positions[i], inner_positions[perm[i]]))
                .sum::<f32>();
            if score < best_score {
                best_score = score;
                best_perm = perm;
            }
        }
        let ordered_inner = [
            inner_positions[best_perm[0]],
            inner_positions[best_perm[1]],
            inner_positions[best_perm[2]],
            inner_positions[best_perm[3]],
        ];

        if mesh.face(face_edge) != Some(params.face) {
            return Err(op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                "cut_rect source face loop is unstable",
            ));
        }
        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .and_then(|layer| layer.get(params.face.as_id()).copied())
            .unwrap_or(0);

        Ok(CutRectFacePlan {
            face: params.face,
            outer_vertices,
            inner_positions: ordered_inner,
            source_edge_attrs,
            region,
        })
    }

    fn apply_plan<S: exedra::ChangeSink>(
        &self,
        txn: &mut exedra::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );

        let mut inner_vertices = Vec::with_capacity(4);
        for position in plan.inner_positions {
            inner_vertices.push(op::add_vertex(txn, position));
        }
        let inner_vertices: [VertexId; 4] = inner_vertices
            .try_into()
            .expect("cut_rect should create exactly four inner vertices");

        op::delete_faces(txn, &[plan.face], DeletePolicy::KeepIsolated).map_err(|err| {
            op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                format!("cut_rect delete failed unexpectedly: {err}"),
            )
        })?;

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
                ctx,
                "cut_rect",
            )?;
            if op::set_face_region(txn, frame_face, plan.region).is_err() {
                return Err(op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    "failed to set cut_rect frame region",
                ));
            }
            propagate_frame_edge_attrs(
                txn,
                frame_face,
                current,
                next,
                current_inner,
                next_inner,
                plan.source_edge_attrs[i],
                &ctx.policy.propagate,
            );
            frame_faces.push(frame_face);
        }

        let mut inner_loop = inner_vertices.to_vec();
        if !frame_orientation.prefers_forward_outer_edge() {
            inner_loop.reverse();
        }
        let inner_face = op::add_face(txn, &inner_loop).map_err(|err| {
            op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                format!("cut_rect inner face creation failed unexpectedly: {err}"),
            )
        })?;
        if op::set_face_region(txn, inner_face, plan.region).is_err() {
            return Err(op_error(
                ctx,
                OpErrorKind::InternalInvariantViolation,
                DiagCode::InternalInvariantViolation,
                "failed to set cut_rect inner face region",
            ));
        }

        let mut boundary_edges = EdgeSet::with_capacity(4);
        for inner_corner in txn.mesh().face_loop(inner_face) {
            let Some(frame_side) = txn.mesh().twin(inner_corner) else {
                return Err(op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "cut_rect produced inner edge without twin",
                ));
            };
            boundary_edges.push(frame_side);
        }
        let _ = crate::selection::canonicalize_edge_set(&mut boundary_edges);

        report.stats.counters.faces_processed = 1;
        report.stats.elements_created.vertices = 4;
        report.stats.elements_created.faces = 5;
        report.stats.elements_deleted.faces = 1;
        Ok((
            report,
            CutRectFaceOutput {
                inner_faces: vec![inner_face],
                frame_faces,
                boundary_edges,
            },
        ))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_u32(plan.face.index());
        for vertex in plan.outer_vertices {
            hasher.write_u32(vertex.index());
        }
        for position in plan.inner_positions {
            hasher.write_f32_bits(position[0]);
            hasher.write_f32_bits(position[1]);
            hasher.write_f32_bits(position[2]);
        }
        for attrs in plan.source_edge_attrs {
            hasher.write_u8(u8::from(attrs.seam.unwrap_or(false)));
            hasher.write_f32_bits(attrs.sharpness.unwrap_or(0.0));
        }
        hasher.write_u32(plan.region);
        hasher.finish()
    }
}

#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct SourceEdgeAttrs {
    pub(crate) seam: Option<bool>,
    pub(crate) sharpness: Option<f32>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum FrameWinding {
    UseReverseOuterEdge,
    UseForwardOuterEdge,
}

#[derive(Copy, Clone, Debug, Default)]
struct FrameOrientationState {
    winding: Option<FrameWinding>,
}

impl FrameOrientationState {
    const fn prefers_forward_outer_edge(self) -> bool {
        matches!(self.winding, Some(FrameWinding::UseForwardOuterEdge))
    }
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length3(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt_ext()
}

fn normalize3(a: [f32; 3]) -> Option<[f32; 3]> {
    let len = length3(a);
    (len > 0.0).then(|| scale3(a, 1.0 / len))
}

fn distance_sq3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub3(a, b);
    dot3(d, d)
}

fn project_to_basis(
    point: [f32; 3],
    origin: [f32; 3],
    basis_u: [f32; 3],
    basis_v: [f32; 3],
) -> [f32; 2] {
    let delta = sub3(point, origin);
    [dot3(delta, basis_u), dot3(delta, basis_v)]
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

fn centroid(mesh: &exedra::Mesh, vertices: &[VertexId]) -> Option<[f32; 3]> {
    if vertices.is_empty() {
        return None;
    }
    let mut sum = [0.0_f32, 0.0, 0.0];
    for &vertex in vertices {
        let position = mesh.vertex_position(vertex)?;
        sum[0] += position[0];
        sum[1] += position[1];
        sum[2] += position[2];
    }
    let inv = 1.0 / (vertices.len() as f32);
    Some([sum[0] * inv, sum[1] * inv, sum[2] * inv])
}

pub(crate) fn normalized_face_normal(
    mesh: &exedra::Mesh,
    vertices: &[VertexId],
) -> Option<[f32; 3]> {
    let mut nx = 0.0_f32;
    let mut ny = 0.0_f32;
    let mut nz = 0.0_f32;
    for i in 0..vertices.len() {
        let current = mesh.vertex_position(vertices[i])?;
        let next = mesh.vertex_position(vertices[(i + 1) % vertices.len()])?;
        nx += (current[1] - next[1]) * (current[2] + next[2]);
        ny += (current[2] - next[2]) * (current[0] + next[0]);
        nz += (current[0] - next[0]) * (current[1] + next[1]);
    }
    let length_sq = nx * nx + ny * ny + nz * nz;
    if length_sq <= 1e-12 {
        return None;
    }
    let inv_len = 1.0 / length_sq.sqrt_ext();
    Some([nx * inv_len, ny * inv_len, nz * inv_len])
}

fn add_frame_face_with_orientation<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    current: VertexId,
    next: VertexId,
    current_inset: VertexId,
    next_inset: VertexId,
    orientation: &mut FrameOrientationState,
    ctx: &mut OpContext,
    op_name: &'static str,
) -> Result<FaceId, OpError> {
    let reverse_outer = [next, current, current_inset, next_inset];
    let forward_outer = [current, next, next_inset, current_inset];
    match orientation.winding {
        Some(FrameWinding::UseReverseOuterEdge) => {
            op::add_face(txn, &reverse_outer).map_err(|err| frame_face_error(ctx, op_name, err))
        }
        Some(FrameWinding::UseForwardOuterEdge) => {
            op::add_face(txn, &forward_outer).map_err(|err| frame_face_error(ctx, op_name, err))
        }
        None => match op::add_face(txn, &reverse_outer) {
            Ok(face) => {
                orientation.winding = Some(FrameWinding::UseReverseOuterEdge);
                Ok(face)
            }
            // add_face performs full preflight before mutation,
            // so this fallback remains deterministic and side-effect free.
            Err(AddFaceError::NonManifoldEdge { .. }) => {
                ctx.diagnostics.push(Diagnostic::new(
                    DiagLevel::Warn,
                    DiagCode::PreconditionFailed,
                    alloc::format!(
                        "{op_name}: frame winding fallback to forward orientation due to boundary reuse direction"
                    ),
                ));
                let face = op::add_face(txn, &forward_outer)
                    .map_err(|err| frame_face_error(ctx, op_name, err))?;
                orientation.winding = Some(FrameWinding::UseForwardOuterEdge);
                Ok(face)
            }
            Err(err) => Err(frame_face_error(ctx, op_name, err)),
        },
    }
}

fn frame_face_error(ctx: &OpContext, op_name: &'static str, err: AddFaceError) -> OpError {
    op_error(
        ctx,
        OpErrorKind::InternalInvariantViolation,
        DiagCode::InternalInvariantViolation,
        format!("{op_name} frame face creation failed unexpectedly: {err}"),
    )
}

fn propagate_face_corner_uvs<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    uv_map: &[(VertexId, Option<[f32; 2]>)],
) {
    let corners = txn.mesh().face_loop(face).collect::<Vec<_>>();
    for corner in corners {
        let Some(to_vertex) = txn.mesh().to_vertex(corner) else {
            continue;
        };
        let uv = uv_map
            .iter()
            .find_map(|(vertex, uv)| (*vertex == to_vertex).then_some(*uv))
            .flatten();
        if let Some(uv) = uv {
            let _ = op::set_corner_uv(txn, corner, uv);
        }
    }
}

fn propagate_edge_attrs_for_vertices<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    a: VertexId,
    b: VertexId,
    source: SourceEdgeAttrs,
    policy: &exedra::PropagatePolicy,
) {
    let Some(corner) = find_face_edge_for_vertices(txn.mesh(), face, a, b) else {
        return;
    };
    match policy.edge_attr {
        EdgeAttrPropagation::Clear => {
            let _ = op::set_edge_seam(txn, corner, false);
            let _ = op::set_edge_sharpness(txn, corner, 0.0);
        }
        EdgeAttrPropagation::Inherit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.unwrap_or(0.0);
            let _ = op::set_edge_seam(txn, corner, seam);
            let _ = op::set_edge_sharpness(txn, corner, sharpness);
        }
        EdgeAttrPropagation::DecayOnSplit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.map_or(0.0, |value| (value - 1.0).max(0.0));
            let _ = op::set_edge_seam(txn, corner, seam);
            let _ = op::set_edge_sharpness(txn, corner, sharpness);
        }
    }
}

fn propagate_frame_edge_attrs<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    face: FaceId,
    current: VertexId,
    next: VertexId,
    current_inner: VertexId,
    next_inner: VertexId,
    source: SourceEdgeAttrs,
    policy: &exedra::PropagatePolicy,
) {
    propagate_edge_attrs_for_vertices(txn, face, current, next, source, policy);
    propagate_edge_attrs_for_vertices(txn, face, current_inner, next_inner, source, policy);
    propagate_edge_attrs_for_vertices(
        txn,
        face,
        current,
        current_inner,
        SourceEdgeAttrs::default(),
        policy,
    );
    propagate_edge_attrs_for_vertices(
        txn,
        face,
        next,
        next_inner,
        SourceEdgeAttrs::default(),
        policy,
    );
}

fn find_face_edge_for_vertices(
    mesh: &exedra::Mesh,
    face: FaceId,
    a: VertexId,
    b: VertexId,
) -> Option<HalfEdgeId> {
    mesh.face_loop(face).find(|&corner| {
        let Some(from) = mesh.from_vertex(corner) else {
            return false;
        };
        let Some(to) = mesh.to_vertex(corner) else {
            return false;
        };
        (from == a && to == b) || (from == b && to == a)
    })
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::{BuildParams, EdgeAttrPropagation, Mesh, PropagatePolicy};

    use super::{
        CutRectFace, CutRectFaceParams, ExtrudeFaces, ExtrudeFacesParams, ExtrudeMode, InsetFaces,
        InsetFacesParams, SolidifyFaces, SolidifyFacesParams, SolidifyMode,
    };
    use crate::{
        DeleteFaces, DeleteFacesParams, OpErrorKind, OperatorRunner, TagFaceRegion,
        TagFaceRegionParams, mesh_signature, test_support::commit,
    };

    fn quad_mesh() -> (Mesh, exedra::FaceId) {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        (mesh, face)
    }

    #[test]
    fn extrude_creates_cap_and_side_walls() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = ExtrudeFaces;
        let params = ExtrudeFacesParams {
            faces: vec![face],
            mode: ExtrudeMode::ShellOpen,
            distance: 1.0,
        };

        let result = commit(&mut runner, &mut mesh, &op, &params).expect("extrude should succeed");
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(mesh.faces().count(), 5);
        assert_eq!(mesh.vertices().count(), 8);
        let nonzero_z = mesh
            .vertices()
            .filter_map(|vertex| mesh.vertex_position(vertex))
            .filter(|position| position[2].abs() > 1e-5)
            .count();
        assert_eq!(nonzero_z, 4);
        assert_eq!(result.output.cap_faces.len(), 1);
        assert_eq!(result.output.wall_faces.len(), 4);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_creates_inner_face_and_frame() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let op = InsetFaces;
        let params = InsetFacesParams {
            faces: vec![face],
            factor: 0.25,
        };

        let result = commit(&mut runner, &mut mesh, &op, &params).expect("inset should succeed");
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(mesh.faces().count(), 5);
        assert_eq!(mesh.vertices().count(), 8);
        let nonzero_z = mesh
            .vertices()
            .filter_map(|vertex| mesh.vertex_position(vertex))
            .filter(|position| position[2].abs() > 1e-5)
            .count();
        assert_eq!(nonzero_z, 0);
        assert_eq!(result.output.inner_faces.len(), 1);
        assert_eq!(result.output.frame_faces.len(), 4);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn extrude_and_inset_preserve_face_region() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 42,
                faces: vec![face],
            },
        )
        .expect("region tagging should succeed");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::ShellOpen,
                distance: 0.5,
            },
        )
        .expect("extrude should succeed");

        let layer = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face.region must exist");
        for face in mesh.faces() {
            let region = layer
                .get(face.as_id())
                .copied()
                .expect("region value should exist");
            assert_eq!(region, 42);
        }

        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.5, 0.5, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &TagFaceRegion,
            &TagFaceRegionParams {
                region_id: 9,
                faces: vec![face],
            },
        )
        .expect("region tagging should succeed");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.3,
            },
        )
        .expect("inset should succeed");
        let layer = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .expect("face.region must exist");
        for face in mesh.faces() {
            let region = layer
                .get(face.as_id())
                .copied()
                .expect("region value should exist");
            assert_eq!(region, 9);
        }
    }

    #[test]
    fn extrude_supports_adjacent_face_selection_without_duplicate_internal_walls() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces,
                mode: ExtrudeMode::ShellOpen,
                distance: 1.0,
            },
        )
        .expect("adjacent selection should succeed");
        assert_eq!(result.output.wall_faces.len(), 4);
        assert_eq!(result.output.cap_faces.len(), 2);
        assert_eq!(mesh.faces().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_supports_adjacent_face_selection_without_duplicate_internal_frames() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("mesh build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams { faces, factor: 0.3 },
        )
        .expect("adjacent inset should succeed");
        assert_eq!(result.output.frame_faces.len(), 4);
        assert_eq!(result.output.inner_faces.len(), 2);
        assert_eq!(mesh.faces().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_handles_ngon_face() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.5, 0.8, 0.0],
                [0.5, 1.5, 0.0],
                [-0.3, 0.8, 0.0],
            ],
            &[&[0, 1, 2, 3, 4]],
        )
        .expect("pentagon build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.35,
            },
        )
        .expect("ngon inset should succeed");
        assert_eq!(mesh.faces().count(), 6);
        assert_eq!(mesh.vertices().count(), 10);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    fn face_avg_z(mesh: &Mesh, face: exedra::FaceId) -> f32 {
        let mut sum = 0.0_f32;
        let mut count = 0_u32;
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).expect("corner vertex should exist");
            let position = mesh
                .vertex_position(vertex)
                .expect("vertex position should exist");
            sum += position[2];
            count = count.saturating_add(1);
        }
        if count == 0 { 0.0 } else { sum / count as f32 }
    }

    #[test]
    fn inset_succeeds_on_extruded_top_face() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::ShellOpen,
                distance: 0.6,
            },
        )
        .expect("extrude should succeed");
        let top = mesh
            .faces()
            .max_by(|&a, &b| face_avg_z(&mesh, a).total_cmp(&face_avg_z(&mesh, b)))
            .expect("top face should exist");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![top],
                factor: 0.3,
            },
        )
        .expect("inset on extruded top should succeed");
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_compile_is_deterministic_for_identical_mesh_state() {
        let (mesh, face) = quad_mesh();
        let params = InsetFacesParams {
            faces: vec![face],
            factor: 0.3,
        };
        let signature = mesh_signature(&mesh);
        let mut runner = OperatorRunner::new();

        let plan_a = runner
            .compile(&mesh, &InsetFaces, &params)
            .expect("inset compile should succeed");
        let plan_b = runner
            .compile(&mesh, &InsetFaces, &params)
            .expect("inset compile should succeed");
        assert_eq!(signature, mesh_signature(&mesh));
        assert_eq!(plan_a.fingerprint, plan_b.fingerprint);
        assert_eq!(plan_a.payload.faces, vec![face]);
        assert_eq!(plan_a.payload.factor, 0.3);
    }

    fn cube_mesh() -> Mesh {
        Mesh::from_polygons(
            &[
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            &[
                &[0, 1, 2, 3],
                &[4, 7, 6, 5],
                &[0, 4, 5, 1],
                &[1, 5, 6, 2],
                &[2, 6, 7, 3],
                &[3, 7, 4, 0],
            ],
        )
        .expect("cube build should succeed")
    }

    #[test]
    fn extrude_succeeds_on_closed_box_face() {
        let mut mesh = cube_mesh();
        let face = mesh
            .faces()
            .max_by(|&a, &b| face_avg_z(&mesh, a).total_cmp(&face_avg_z(&mesh, b)))
            .expect("target face should exist");
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::ShellOpen,
                distance: 0.4,
            },
        )
        .expect("extrude on closed box should succeed");
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn extrude_keep_source_succeeds_on_open_surface() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::KeepSource,
                distance: 0.4,
            },
        )
        .expect("keep-source extrude on open surface should succeed");
        assert_eq!(mesh.faces().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn extrude_keep_source_rejects_closed_box_face() {
        let mut mesh = cube_mesh();
        let face = mesh
            .faces()
            .max_by(|&a, &b| face_avg_z(&mesh, a).total_cmp(&face_avg_z(&mesh, b)))
            .expect("target face should exist");
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::KeepSource,
                distance: 0.25,
            },
        )
        .expect_err("keep-source extrude on closed volume should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn solidify_keep_source_succeeds_on_open_surface() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &SolidifyFaces,
            &SolidifyFacesParams {
                faces: vec![face],
                mode: SolidifyMode::KeepSource,
                thickness: 0.3,
            },
        )
        .expect("solidify should succeed on open surface");
        assert_eq!(result.report.name, "edit.face.solidify");
        assert_eq!(result.output.cap_faces.len(), 1);
        assert_eq!(result.output.wall_faces.len(), 4);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn solidify_keep_source_rejects_closed_box_face() {
        let mut mesh = cube_mesh();
        let face = mesh
            .faces()
            .max_by(|&a, &b| face_avg_z(&mesh, a).total_cmp(&face_avg_z(&mesh, b)))
            .expect("target face should exist");
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &SolidifyFaces,
            &SolidifyFacesParams {
                faces: vec![face],
                mode: SolidifyMode::KeepSource,
                thickness: 0.2,
            },
        )
        .expect_err("solidify keep-source on closed volume should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn extrude_propagates_corner_uv_and_edge_attrs() {
        let (mut mesh, face) = quad_mesh();
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit();
            for (index, &corner) in corners.iter().enumerate() {
                assert!(exedra::op::set_corner_uv(&mut txn, corner, [index as f32, 0.0]).is_ok());
                assert!(exedra::op::set_edge_seam(&mut txn, corner, true).is_ok());
                assert!(exedra::op::set_edge_sharpness(&mut txn, corner, 2.5).is_ok());
            }
            let _: () = txn.finish();
        }

        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::ShellOpen,
                distance: 0.5,
            },
        )
        .expect("extrude should succeed");
        let cap = result.output.cap_faces[0];

        let uv_layer = mesh
            .attrs()
            .sparse(exedra::attr::CORNER_UV)
            .expect("corner uv layer should exist");
        for corner in mesh.face_loop(cap) {
            assert!(uv_layer.get(corner.as_id()).is_some());
            assert_eq!(mesh.edge_seam(corner), Some(true));
            assert_eq!(mesh.edge_sharpness(corner), Some(2.5));
        }
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_clear_policy_resets_generated_edge_tags() {
        let (mut mesh, face) = quad_mesh();
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit();
            for &corner in &corners {
                assert!(exedra::op::set_edge_seam(&mut txn, corner, true).is_ok());
                assert!(exedra::op::set_edge_sharpness(&mut txn, corner, 3.0).is_ok());
            }
            let _: () = txn.finish();
        }

        let mut runner = OperatorRunner::new();
        runner.ctx.policy.propagate = PropagatePolicy {
            edge_attr: EdgeAttrPropagation::Clear,
            ..PropagatePolicy::default()
        };

        let result = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.3,
            },
        )
        .expect("inset should succeed");

        for generated_face in result
            .output
            .inner_faces
            .iter()
            .copied()
            .chain(result.output.frame_faces.iter().copied())
        {
            for corner in mesh.face_loop(generated_face) {
                assert_eq!(mesh.edge_seam(corner), Some(false));
                assert_eq!(mesh.edge_sharpness(corner), Some(0.0));
            }
        }
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn cut_rect_creates_inner_face_and_frame_faces() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [1.0, 0.0, 0.0],
                frame_v: [0.0, 1.0, 0.0],
                rect_min: [0.25, 0.2],
                rect_max: [0.75, 0.8],
            },
        )
        .expect("cut_rect should succeed");

        assert_eq!(result.output.inner_faces.len(), 1);
        assert_eq!(result.output.frame_faces.len(), 4);
        assert_eq!(result.output.boundary_edges.len(), 4);
        assert_eq!(mesh.faces().count(), 5);
        assert_eq!(mesh.vertices().count(), 8);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn cut_rect_inner_face_can_be_deleted_to_make_opening() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let cut = commit(
            &mut runner,
            &mut mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [1.0, 0.0, 0.0],
                frame_v: [0.0, 1.0, 0.0],
                rect_min: [0.2, 0.2],
                rect_max: [0.8, 0.8],
            },
        )
        .expect("cut_rect should succeed");
        let inner = cut.output.inner_faces[0];
        let _ = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: vec![inner],
                policy: exedra::DeletePolicy::KeepIsolated,
            },
        )
        .expect("delete inner cut face should succeed");

        assert_eq!(mesh.faces().count(), 4);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn cut_rect_rejects_non_quad_face() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.5, 0.8, 0.0],
                [0.5, 1.5, 0.0],
                [-0.3, 0.8, 0.0],
            ],
            &[&[0, 1, 2, 3, 4]],
        )
        .expect("pentagon build should succeed");
        let face = mesh.faces().next().expect("face should exist");
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [1.0, 0.0, 0.0],
                frame_v: [0.0, 1.0, 0.0],
                rect_min: [0.2, 0.2],
                rect_max: [0.8, 0.8],
            },
        )
        .expect_err("cut_rect should reject non-quad source face");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }
}
