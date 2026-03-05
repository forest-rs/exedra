// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face extrude/inset edit operators.

use alloc::format;
use alloc::vec::Vec;

use exedra::{AddFaceError, DeletePolicy, FaceId, VertexId};

use crate::math::FloatExt;
use crate::op_common::op_error;
use crate::plan::PlanHasher;
use crate::selection::{FaceSet, canonicalize_face_set};
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

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

        for plan in plans {
            let mut cap = Vec::with_capacity(plan.vertices.len());
            for &vertex in &plan.vertices {
                let position = txn
                    .mesh()
                    .vertex_position(vertex)
                    .expect("preflight-validated vertex must be live");
                let extruded = [
                    position[0] + plan.normal[0] * params.distance,
                    position[1] + plan.normal[1] * params.distance,
                    position[2] + plan.normal[2] * params.distance,
                ];
                cap.push(txn.add_vertex(extruded));
            }

            txn.delete_faces(&[plan.face], DeletePolicy::KeepIsolated)
                .map_err(|err| {
                    op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        format!("extrude delete failed unexpectedly: {err}"),
                    )
                })?;

            let mut wall_winding = None;
            for i in 0..plan.vertices.len() {
                let current = plan.vertices[i];
                let next = plan.vertices[(i + 1) % plan.vertices.len()];
                let current_cap = cap[i];
                let next_cap = cap[(i + 1) % cap.len()];
                let (wall, used_winding) =
                    add_frame_face(txn, current, next, current_cap, next_cap, wall_winding)
                        .map_err(|err| {
                            op_error(
                                ctx,
                                OpErrorKind::InternalInvariantViolation,
                                DiagCode::InternalInvariantViolation,
                                format!("extrude wall creation failed unexpectedly: {err}"),
                            )
                        })?;
                wall_winding = Some(used_winding);
                if !txn.set_face_region(wall, plan.region) {
                    return Err(op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        "failed to set extrude wall region",
                    ));
                }
                wall_faces.push(wall);
            }

            let mut cap_loop = cap.clone();
            if wall_winding != Some(FrameWinding::UseForwardOuterEdge) {
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
            cap_faces.push(top);

            report.stats.counters.faces_processed =
                report.stats.counters.faces_processed.saturating_add(1);
            report.stats.elements_created.vertices = report
                .stats
                .elements_created
                .vertices
                .saturating_add(u64::try_from(cap.len()).expect("vertex count should fit u64"));
            report.stats.elements_created.faces =
                report.stats.elements_created.faces.saturating_add(
                    u64::try_from(cap.len() + 1).expect("face count should fit u64"),
                );
            report.stats.elements_deleted.faces =
                report.stats.elements_deleted.faces.saturating_add(1);
        }
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

        for face_plan in plans {
            let centroid = centroid(txn.mesh(), &face_plan.vertices).expect("preflight validated");
            let mut inset_loop = Vec::with_capacity(face_plan.vertices.len());
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
                inset_loop.push(txn.add_vertex(inset));
            }

            txn.delete_faces(&[face_plan.face], DeletePolicy::KeepIsolated)
                .map_err(|err| {
                    op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        format!("inset delete failed unexpectedly: {err}"),
                    )
                })?;

            let mut frame_winding = None;
            for i in 0..face_plan.vertices.len() {
                let current = face_plan.vertices[i];
                let next = face_plan.vertices[(i + 1) % face_plan.vertices.len()];
                let current_inset = inset_loop[i];
                let next_inset = inset_loop[(i + 1) % inset_loop.len()];
                let (frame, used_winding) =
                    add_frame_face(txn, current, next, current_inset, next_inset, frame_winding)
                        .map_err(|err| {
                            op_error(
                                ctx,
                                OpErrorKind::InternalInvariantViolation,
                                DiagCode::InternalInvariantViolation,
                                format!("inset frame face creation failed unexpectedly: {err}"),
                            )
                        })?;
                frame_winding = Some(used_winding);
                if !txn.set_face_region(frame, face_plan.region) {
                    return Err(op_error(
                        ctx,
                        OpErrorKind::InternalInvariantViolation,
                        DiagCode::InternalInvariantViolation,
                        "failed to set inset frame face region",
                    ));
                }
                frame_faces.push(frame);
            }

            // Fill the inset hole after frame faces are created so the inner
            // loop orientation can consistently reuse the resulting OUTSIDE
            // boundary ring.
            let mut inner_loop = inset_loop.clone();
            if frame_winding != Some(FrameWinding::UseForwardOuterEdge) {
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
            inner_faces.push(inner);

            report.stats.counters.faces_processed =
                report.stats.counters.faces_processed.saturating_add(1);
            report.stats.elements_created.vertices =
                report.stats.elements_created.vertices.saturating_add(
                    u64::try_from(inset_loop.len()).expect("vertex count should fit u64"),
                );
            report.stats.elements_created.faces =
                report.stats.elements_created.faces.saturating_add(
                    u64::try_from(inset_loop.len() + 1).expect("face count should fit u64"),
                );
            report.stats.elements_deleted.faces =
                report.stats.elements_deleted.faces.saturating_add(1);
        }
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
    normal: [f32; 3],
    region: u32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum FrameWinding {
    UseReverseOuterEdge,
    UseForwardOuterEdge,
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
        plans.push(FacePlan {
            face,
            vertices,
            normal,
            region,
        });
    }

    let mut edge_to_face = Vec::<(u32, u32, FaceId)>::new();
    for plan in &plans {
        for i in 0..plan.vertices.len() {
            let a = plan.vertices[i].index();
            let b = plan.vertices[(i + 1) % plan.vertices.len()].index();
            edge_to_face.push((u32::min(a, b), u32::max(a, b), plan.face));
        }
    }
    edge_to_face.sort_unstable_by_key(|(a, b, face)| (*a, *b, *face));
    for pair in edge_to_face.windows(2) {
        let (a0, b0, f0) = pair[0];
        let (a1, b1, f1) = pair[1];
        if a0 == a1 && b0 == b1 && f0 != f1 {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!(
                    "selected faces share an edge ({a0}, {b0}); adjacent selections are not supported"
                ),
            ));
        }
    }

    Ok(plans)
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

fn add_frame_face(
    txn: &mut exedra::EditSession<'_>,
    current: VertexId,
    next: VertexId,
    current_inset: VertexId,
    next_inset: VertexId,
    preferred: Option<FrameWinding>,
) -> Result<(FaceId, FrameWinding), AddFaceError> {
    let reverse_outer = [next, current, current_inset, next_inset];
    let forward_outer = [current, next, next_inset, current_inset];
    match preferred {
        Some(FrameWinding::UseReverseOuterEdge) => txn
            .add_face(&reverse_outer)
            .map(|face| (face, FrameWinding::UseReverseOuterEdge)),
        Some(FrameWinding::UseForwardOuterEdge) => txn
            .add_face(&forward_outer)
            .map(|face| (face, FrameWinding::UseForwardOuterEdge)),
        None => match txn.add_face(&reverse_outer) {
            Ok(face) => Ok((face, FrameWinding::UseReverseOuterEdge)),
            // Relies on EditSession::add_face preflight returning NonManifoldEdge
            // before mutation, so trying the alternate winding is safe.
            Err(AddFaceError::NonManifoldEdge { .. }) => txn
                .add_face(&forward_outer)
                .map(|face| (face, FrameWinding::UseForwardOuterEdge)),
            Err(err) => Err(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use exedra::{BuildParams, Mesh};

    use super::{ExtrudeFaces, ExtrudeFacesParams, InsetFaces, InsetFacesParams};
    use crate::{
        OpErrorKind, OperatorRunner, TagFaceRegion, TagFaceRegionParams, mesh_signature,
        test_support::commit,
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
    fn extrude_rejects_adjacent_face_selection() {
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
        let err = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces,
                distance: 1.0,
            },
        )
        .expect_err("adjacent selection should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
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
}
