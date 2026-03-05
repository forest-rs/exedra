// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face extrude/inset edit operators.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;

use exedra::{AddFaceError, DeletePolicy, EdgeAttrPropagation, FaceId, HalfEdgeId, VertexId};

use crate::math::FloatExt;
use crate::op_common::op_error;
use crate::plan::PlanHasher;
use crate::selection::{FaceSet, canonicalize_face_set};
use crate::{
    Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, OpContext, OpError, OpErrorKind,
    OpReport,
};

/// Parameters for [`ExtrudeFaces`].
#[derive(Clone, Debug, PartialEq)]
pub struct ExtrudeFacesParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Distance along each face normal.
    ///
    /// v0.1 semantics are shell-style: the source face is removed and replaced
    /// by side walls + offset cap.
    pub distance: f32,
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

/// Deterministic compiled plan payload for [`InsetFaces`].
#[derive(Clone, Debug)]
pub struct InsetFacesPlan {
    faces: FaceSet,
    face_plans: Vec<FacePlan>,
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
#[derive(Copy, Clone, Debug, Default)]
pub struct ExtrudeFaces;

impl EditOperator for ExtrudeFaces {
    type Params = ExtrudeFacesParams;
    type Plan = ExtrudeFacesParams;
    type Output = ExtrudeFacesOutput;

    fn name(&self) -> &'static str {
        "edit.face.extrude"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
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
        let plans = preflight_face_plans(txn.mesh(), &faces, true, ctx)?;

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
        let edge_counts = selected_edge_counts(&plans);
        let mut summed_normals = BTreeMap::<VertexId, [f32; 3]>::new();
        for plan in &plans {
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
            cap_vertices.insert(vertex, txn.add_vertex(extruded));
        }

        let faces_to_delete = plans.iter().map(|plan| plan.face).collect::<Vec<_>>();
        txn.delete_faces(&faces_to_delete, DeletePolicy::KeepIsolated)
            .map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("extrude delete failed unexpectedly: {err}"),
                )
            })?;

        let mut wall_orientation = FrameOrientationState::default();
        for plan in &plans {
            for i in 0..plan.vertices.len() {
                let current = plan.vertices[i];
                let next = plan.vertices[(i + 1) % plan.vertices.len()];
                if !is_boundary_edge(current, next, &edge_counts) {
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
                if !txn.set_face_region(wall, plan.region) {
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

        for plan in &plans {
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
            let top = txn.add_face(&cap_loop).map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("extrude cap creation failed unexpectedly: {err}"),
                )
            })?;
            if !txn.set_face_region(top, plan.region) {
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
            u64::try_from(plans.len()).expect("face count should fit u64");
        report.stats.elements_created.vertices =
            u64::try_from(cap_vertices.len()).expect("vertex count should fit u64");
        report.stats.elements_created.faces =
            u64::try_from(wall_faces.len() + cap_faces.len()).expect("face count should fit u64");
        report.stats.elements_deleted.faces =
            u64::try_from(plans.len()).expect("face count should fit u64");
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

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
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
        let face_plans = preflight_face_plans(mesh, &faces, false, ctx)?;
        Ok(InsetFacesPlan {
            faces,
            face_plans,
            factor: params.factor,
            selections_canonicalized: canonicalized,
        })
    }

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
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
        let edge_counts = selected_edge_counts(&plans);
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
            inset_vertices.insert(vertex, txn.add_vertex(averaged));
        }

        let faces_to_delete = plans.iter().map(|plan| plan.face).collect::<Vec<_>>();
        txn.delete_faces(&faces_to_delete, DeletePolicy::KeepIsolated)
            .map_err(|err| {
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
                if !is_boundary_edge(current, next, &edge_counts) {
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
                if !txn.set_face_region(frame, face_plan.region) {
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
            let inner = txn.add_face(&inner_loop).map_err(|err| {
                op_error(
                    ctx,
                    OpErrorKind::InternalInvariantViolation,
                    DiagCode::InternalInvariantViolation,
                    format!("inset inner face creation failed unexpectedly: {err}"),
                )
            })?;
            if !txn.set_face_region(inner, face_plan.region) {
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

#[derive(Clone, Debug)]
struct FacePlan {
    face: FaceId,
    vertices: Vec<VertexId>,
    vertex_uvs: Vec<Option<[f32; 2]>>,
    edge_attrs: Vec<SourceEdgeAttrs>,
    normal: [f32; 3],
    region: u32,
}

#[derive(Copy, Clone, Debug, Default)]
struct SourceEdgeAttrs {
    seam: Option<bool>,
    sharpness: Option<f32>,
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

fn preflight_face_plans(
    mesh: &exedra::Mesh,
    faces: &[FaceId],
    require_normal: bool,
    ctx: &OpContext,
) -> Result<Vec<FacePlan>, OpError> {
    let mut plans = Vec::<FacePlan>::with_capacity(faces.len());
    for &face in faces {
        if face == FaceId::OUTSIDE {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "selection contains FaceId::OUTSIDE",
            ));
        }
        if mesh.face_edge(face).is_none() {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!("selection contains stale face id: {}", face.index()),
            ));
        }
        let mut vertices = Vec::<VertexId>::new();
        let mut corners = Vec::<HalfEdgeId>::new();
        for corner in mesh.face_loop(face) {
            let vertex = mesh.to_vertex(corner).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "face loop contains corner with missing destination vertex",
                )
            })?;
            vertices.push(vertex);
            corners.push(corner);
        }
        if vertices.len() < 3 {
            return Err(op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                "face loop degree is less than 3",
            ));
        }
        let normal = if require_normal {
            normalized_face_normal(mesh, &vertices).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::NumericFailure,
                    DiagCode::NumericToleranceIssue,
                    format!("cannot extrude degenerate face {}", face.index()),
                )
            })?
        } else {
            [0.0, 0.0, 0.0]
        };
        let region = mesh
            .attrs()
            .dense(exedra::attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let mut vertex_uvs = Vec::with_capacity(corners.len());
        for &corner in &corners {
            let uv = mesh
                .attrs()
                .sparse(exedra::attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id()).copied());
            vertex_uvs.push(uv);
        }
        let mut edge_attrs = Vec::with_capacity(vertices.len());
        for i in 0..vertices.len() {
            let edge_corner = corners[(i + 1) % corners.len()];
            edge_attrs.push(SourceEdgeAttrs {
                seam: mesh.edge_seam(edge_corner),
                sharpness: mesh.edge_sharpness(edge_corner),
            });
        }
        plans.push(FacePlan {
            face,
            vertices,
            vertex_uvs,
            edge_attrs,
            normal,
            region,
        });
    }

    Ok(plans)
}

fn canonical_edge_key(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn selected_edge_counts(plans: &[FacePlan]) -> BTreeMap<(VertexId, VertexId), u32> {
    let mut counts = BTreeMap::<(VertexId, VertexId), u32>::new();
    for plan in plans {
        for i in 0..plan.vertices.len() {
            let current = plan.vertices[i];
            let next = plan.vertices[(i + 1) % plan.vertices.len()];
            let key = canonical_edge_key(current, next);
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts
}

fn is_boundary_edge(
    current: VertexId,
    next: VertexId,
    edge_counts: &BTreeMap<(VertexId, VertexId), u32>,
) -> bool {
    edge_counts
        .get(&canonical_edge_key(current, next))
        .is_some_and(|count| *count == 1)
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

fn normalized_face_normal(mesh: &exedra::Mesh, vertices: &[VertexId]) -> Option<[f32; 3]> {
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

fn add_frame_face_with_orientation(
    txn: &mut exedra::EditSession<'_>,
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
        Some(FrameWinding::UseReverseOuterEdge) => txn
            .add_face(&reverse_outer)
            .map_err(|err| frame_face_error(ctx, op_name, err)),
        Some(FrameWinding::UseForwardOuterEdge) => txn
            .add_face(&forward_outer)
            .map_err(|err| frame_face_error(ctx, op_name, err)),
        None => match txn.add_face(&reverse_outer) {
            Ok(face) => {
                orientation.winding = Some(FrameWinding::UseReverseOuterEdge);
                Ok(face)
            }
            // EditSession::add_face performs full preflight before mutation,
            // so this fallback remains deterministic and side-effect free.
            Err(AddFaceError::NonManifoldEdge { .. }) => {
                ctx.diagnostics.push(Diagnostic::new(
                    DiagLevel::Warn,
                    DiagCode::PreconditionFailed,
                    alloc::format!(
                        "{op_name}: frame winding fallback to forward orientation due to boundary reuse direction"
                    ),
                ));
                let face = txn
                    .add_face(&forward_outer)
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

fn propagate_face_corner_uvs(
    txn: &mut exedra::EditSession<'_>,
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
            let _ = txn.set_corner_uv(corner, uv);
        }
    }
}

fn propagate_edge_attrs_for_vertices(
    txn: &mut exedra::EditSession<'_>,
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
            let _ = txn.set_edge_seam(corner, false);
            let _ = txn.set_edge_sharpness(corner, 0.0);
        }
        EdgeAttrPropagation::Inherit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.unwrap_or(0.0);
            let _ = txn.set_edge_seam(corner, seam);
            let _ = txn.set_edge_sharpness(corner, sharpness);
        }
        EdgeAttrPropagation::DecayOnSplit => {
            let seam = source.seam.unwrap_or(false);
            let sharpness = source.sharpness.map_or(0.0, |value| (value - 1.0).max(0.0));
            let _ = txn.set_edge_seam(corner, seam);
            let _ = txn.set_edge_sharpness(corner, sharpness);
        }
    }
}

fn propagate_frame_edge_attrs(
    txn: &mut exedra::EditSession<'_>,
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

    use super::{ExtrudeFaces, ExtrudeFacesParams, InsetFaces, InsetFacesParams};
    use crate::{
        OperatorRunner, TagFaceRegion, TagFaceRegionParams, mesh_signature, test_support::commit,
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
                distance: 0.4,
            },
        )
        .expect("extrude on closed box should succeed");
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn extrude_propagates_corner_uv_and_edge_attrs() {
        let (mut mesh, face) = quad_mesh();
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.begin();
            for (index, &corner) in corners.iter().enumerate() {
                assert!(txn.set_corner_uv(corner, [index as f32, 0.0]));
                assert!(txn.set_edge_seam(corner, true));
                assert!(txn.set_edge_sharpness(corner, 2.5));
            }
            let _ = txn.commit();
        }

        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
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
            let mut txn = mesh.begin();
            for &corner in &corners {
                assert!(txn.set_edge_seam(corner, true));
                assert!(txn.set_edge_sharpness(corner, 3.0));
            }
            let _ = txn.commit();
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
}
