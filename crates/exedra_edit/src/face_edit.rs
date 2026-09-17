// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face-edit operators (extrude, inset, poke, cut, solidify).

use crate::op_common::op_error;
use crate::plan::PlanHasher;
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};
use alloc::format;

pub use exedra_mesh_ops::face_edit::{
    CutRectFaceOutput, CutRectFaceParams, CutRectFacePlan, ExtrudeFacesOutput, ExtrudeFacesParams,
    ExtrudeMode, InsetFacesOutput, InsetFacesParams, InsetFacesPlan, SolidifyFacesOutput,
    SolidifyFacesParams, SolidifyMode,
};
pub use exedra_mesh_ops::poke::{PokeFacesOutput, PokeFacesParams, PokeFacesPlan};

/// `edit.face.poke` operator.
///
/// Poke splits each selected face into a triangle fan around one new center
/// vertex. The source face is deleted and replaced by `N` triangles for a face
/// of degree `N`.
///
/// v0.1 propagation behavior:
/// - `face.region` is copied to all generated fan triangles,
/// - outer-edge corner UVs are copied from the source face when authored,
/// - center UV is the arithmetic average when all source corner UVs exist,
/// - preserved perimeter edge attrs follow source edges,
/// - new radial edges default clear.
#[derive(Copy, Clone, Debug, Default)]
pub struct PokeFaces;

impl EditOperator for PokeFaces {
    type Params = PokeFacesParams;
    type Plan = PokeFacesPlan;
    type Output = PokeFacesOutput;

    fn name(&self) -> &'static str {
        "edit.face.poke"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        PokeFacesPlan::prepare(mesh, params).map_err(|error| poke_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = plan
            .apply(txn, &ctx.policy.propagate)
            .map_err(|error| poke_error(ctx, error))?;
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        report.stats.counters.selections_canonicalized = u64::from(plan.selections_canonicalized());
        report.stats.counters.faces_processed = plan.faces().len() as u64;
        report.stats.elements_deleted.faces = plan.faces().len() as u64;
        report.stats.elements_created.vertices = output.center_vertices.len() as u64;
        report.stats.elements_created.faces = output.fan_faces.len() as u64;
        Ok((report, output))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_face_set(plan.faces());
        for center in plan.centers() {
            for component in center.position {
                hasher.write_f32_bits(component);
            }
        }
        for center in plan.centers() {
            match center.uv {
                Some(uv) => {
                    hasher.write_u8(1);
                    hasher.write_f32_bits(uv[0]);
                    hasher.write_f32_bits(uv[1]);
                }
                None => hasher.write_u8(0),
            }
        }
        hasher.finish()
    }
}

fn poke_error(ctx: &OpContext, error: exedra_mesh_ops::poke::PokeError) -> OpError {
    use exedra_mesh_ops::poke::PokeError;
    let (kind, code) = match &error {
        PokeError::Selection(exedra_mesh::SelectedFacePatchError::InvalidFaceLoop { .. }) => (
            OpErrorKind::InvalidMesh,
            DiagCode::InternalInvariantViolation,
        ),
        PokeError::Selection(_) | PokeError::StalePreparation => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
        ),
        PokeError::NumericLimit { .. } => {
            (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue)
        }
        _ => (
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
        ),
    };
    op_error(ctx, kind, code, format!("{error}"))
}

/// `edit.face.extrude` operator.
///
/// v0.1 propagation behavior:
/// - `face.region` is copied from source face to generated walls/cap,
/// - generated corner UVs are copied from source per-vertex UVs when authored,
/// - generated edge seam/sharpness follow [`OpContext::policy`](crate::OpContext::policy)
///   `propagate.edge_attr` for boundary-parallel edges; support edges default clear,
/// - semantic feature boundaries are marked sharp by default: cap-to-wall
///   boundaries, wall columns, and keep-source source-to-wall boundaries are
///   hard.
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

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let (stats, output) =
            exedra_mesh_ops::face_edit::extrude_faces(txn, params, &ctx.policy.propagate)
                .map_err(|error| face_error(ctx, error))?;
        Ok((face_report(self.name(), stats, ctx), output))
    }

    fn compile(
        &self,
        _mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        _ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        Ok(params.clone())
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
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
///
/// v0.1 default shading semantics:
/// - the inset inner perimeter is marked sharp by default,
/// - inset columns/outer frame edges remain smooth by default.
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
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        InsetFacesPlan::prepare(mesh, params).map_err(|error| face_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let (stats, output) = plan
            .apply(txn, &ctx.policy.propagate)
            .map_err(|error| face_error(ctx, error))?;
        Ok((face_report(self.name(), stats, ctx), output))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_face_set(plan.faces());
        hasher.write_f32_bits(plan.factor());
        hasher.write_u8(u8::from(plan.selections_canonicalized()));
        hasher.finish()
    }
}

/// `edit.face.solidify` operator.
///
/// This is a dedicated shell-thickness operator built on the face-edit kernel
/// path used by [`ExtrudeFaces`], with a stable user-facing name and defaults.
///
/// v0.1 default shading semantics:
/// - shell feature boundaries and wall columns are marked sharp by default.
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
        mesh: &exedra_mesh::Mesh,
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

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
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
///   corresponding frame outer edges,
/// - marks the generated opening perimeter sharp by default.
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
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        CutRectFacePlan::prepare(mesh, params).map_err(|error| face_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let (stats, output) = plan
            .apply(txn, &ctx.policy.propagate)
            .map_err(|error| face_error(ctx, error))?;
        Ok((face_report(self.name(), stats, ctx), output))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_u32(plan.face().index());
        for vertex in plan.outer_vertices() {
            hasher.write_u32(vertex.index());
        }
        for position in plan.inner_positions() {
            hasher.write_f32_bits(position[0]);
            hasher.write_f32_bits(position[1]);
            hasher.write_f32_bits(position[2]);
        }
        for attrs in plan.source_edge_attrs() {
            hasher.write_u8(u8::from(attrs.seam.unwrap_or(false)));
            hasher.write_f32_bits(attrs.sharpness.unwrap_or(0.0));
        }
        hasher.write_u32(plan.region());
        hasher.finish()
    }
}

fn face_report(
    name: &'static str,
    stats: exedra_mesh_ops::face_edit::FaceEditStats,
    ctx: &mut OpContext,
) -> OpReport {
    let mut report = OpReport::new(
        name,
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    );
    report.stats.counters.faces_processed = stats.faces_processed;
    report.stats.counters.selections_canonicalized = u64::from(stats.selections_canonicalized);
    report.stats.elements_created.vertices = stats.vertices_created;
    report.stats.elements_created.faces = stats.faces_created;
    report.stats.elements_deleted.faces = stats.faces_deleted;
    if stats.reversed_winding {
        ctx.diagnostics.push(crate::Diagnostic::new(crate::DiagLevel::Warn, DiagCode::PreconditionFailed,
            format!("{name}: frame winding fallback to reverse orientation due to boundary reuse direction")));
    }
    report
}
fn face_error(ctx: &OpContext, error: exedra_mesh_ops::face_edit::FaceEditError) -> OpError {
    use exedra_mesh_ops::face_edit::FaceEditError as E;
    let (kind, code) = match &error {
        E::InvalidDistance
        | E::InvalidInsetFactor
        | E::NumericLimit
        | E::DegenerateFace { .. }
        | E::InvalidRectangleFrame
        | E::InvalidRectangleExtents => {
            (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue)
        }
        E::InvalidFace { .. }
        | E::InvalidBoundary
        | E::MissingBoundaryTwin
        | E::Selection(exedra_mesh::SelectedFacePatchError::InvalidFaceLoop { .. }) => (
            OpErrorKind::InvalidMesh,
            DiagCode::InternalInvariantViolation,
        ),
        E::Delete(_) | E::AddFace(_) | E::SetRegion(_) => (
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
        ),
        _ => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
        ),
    };
    op_error(ctx, kind, code, format!("{error}"))
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use core::num::NonZeroU32;

    use exedra_mesh::{BuildParams, EdgeAttrPropagation, Id, Mesh, PropagatePolicy};

    use super::{
        CutRectFace, CutRectFaceParams, ExtrudeFaces, ExtrudeFacesParams, ExtrudeMode, InsetFaces,
        InsetFacesParams, PokeFaces, PokeFacesParams, SolidifyFaces, SolidifyFacesParams,
        SolidifyMode,
    };
    use crate::{
        DeleteFaces, DeleteFacesParams, OpErrorKind, OperatorRunner, TagFaceRegion,
        TagFaceRegionParams, mesh_signature, test_support::commit,
    };

    fn quad_mesh() -> (Mesh, exedra_mesh::FaceId) {
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

    fn face_normal(mesh: &Mesh, face: exedra_mesh::FaceId) -> [f32; 3] {
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        let mut sum = [0.0_f32, 0.0_f32, 0.0_f32];
        for i in 0..corners.len() {
            let a = mesh
                .to_vertex(corners[i])
                .and_then(|vertex| mesh.vertex_position(vertex))
                .expect("corner position should exist");
            let b = mesh
                .to_vertex(corners[(i + 1) % corners.len()])
                .and_then(|vertex| mesh.vertex_position(vertex))
                .expect("corner position should exist");
            sum[0] += (a[1] - b[1]) * (a[2] + b[2]);
            sum[1] += (a[2] - b[2]) * (a[0] + b[0]);
            sum[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        let len = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        assert!(len > 0.0, "face normal should be non-degenerate");
        [sum[0] / len, sum[1] / len, sum[2] / len]
    }

    fn wall_edge_sharpness_classes(
        mesh: &Mesh,
        face: exedra_mesh::FaceId,
    ) -> Vec<([f32; 3], [f32; 3], f32)> {
        mesh.face_loop(face)
            .map(|corner| {
                let from = mesh
                    .from_vertex(corner)
                    .and_then(|vertex| mesh.vertex_position(vertex).copied())
                    .expect("from position should exist");
                let to = mesh
                    .to_vertex(corner)
                    .and_then(|vertex| mesh.vertex_position(vertex).copied())
                    .expect("to position should exist");
                let sharpness = mesh
                    .edge_sharpness(corner)
                    .expect("sharpness layer should exist on generated edge");
                (from, to, sharpness)
            })
            .collect::<Vec<_>>()
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
    fn extrude_preserves_source_face_normal_direction_on_cap() {
        let (mut mesh, face) = quad_mesh();
        let source_normal = face_normal(&mesh, face);
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: vec![face],
                mode: ExtrudeMode::ShellOpen,
                distance: 1.0,
            },
        )
        .expect("extrude should succeed");
        let cap = result.output.cap_faces[0];
        let cap_normal = face_normal(&mesh, cap);
        let dot = source_normal[0] * cap_normal[0]
            + source_normal[1] * cap_normal[1]
            + source_normal[2] * cap_normal[2];
        assert!(
            dot > 0.99,
            "cap face normal should match source orientation"
        );
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
            .dense(exedra_mesh::attr::FACE_REGION)
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
            .dense(exedra_mesh::attr::FACE_REGION)
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

    #[test]
    fn inset_preserves_source_face_normal_direction_on_quad() {
        let (mut mesh, face) = quad_mesh();
        let source_normal = face_normal(&mesh, face);
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.25,
            },
        )
        .expect("inset should succeed");
        let inner = result.output.inner_faces[0];
        let inner_normal = face_normal(&mesh, inner);
        let dot = source_normal[0] * inner_normal[0]
            + source_normal[1] * inner_normal[1]
            + source_normal[2] * inner_normal[2];
        assert!(
            dot > 0.99,
            "inner face normal should match source orientation"
        );
    }

    #[test]
    fn poke_creates_triangle_fan_from_quad() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &PokeFaces,
            &PokeFacesParams { faces: vec![face] },
        )
        .expect("poke should succeed");
        assert_eq!(result.output.source_faces, vec![face]);
        assert_eq!(result.output.center_vertices.len(), 1);
        assert_eq!(result.output.fan_faces.len(), 4);
        assert_eq!(mesh.faces().count(), 4);
        assert_eq!(mesh.vertices().count(), 5);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn poke_handles_ngon_face() {
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
        let result = commit(
            &mut runner,
            &mut mesh,
            &PokeFaces,
            &PokeFacesParams { faces: vec![face] },
        )
        .expect("poke should succeed");
        assert_eq!(result.output.fan_faces.len(), 5);
        assert_eq!(mesh.faces().count(), 5);
        assert_eq!(mesh.vertices().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn poke_compile_canonicalizes_and_rejects_stale_face() {
        let (mesh, face) = quad_mesh();
        let stale = exedra_mesh::FaceId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();

        let plan = runner
            .compile(
                &mesh,
                &PokeFaces,
                &PokeFacesParams {
                    faces: vec![face, face],
                },
            )
            .expect("compile should canonicalize duplicate face selection");
        assert_eq!(plan.payload.faces(), &[face]);

        let err = runner
            .compile(&mesh, &PokeFaces, &PokeFacesParams { faces: vec![stale] })
            .expect_err("stale face should fail at compile time");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    fn face_avg_z(mesh: &Mesh, face: exedra_mesh::FaceId) -> f32 {
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
        assert_eq!(plan_a.payload.faces(), vec![face]);
        assert_eq!(plan_a.payload.factor(), 0.3);
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
    fn solidify_marks_shell_feature_boundaries_sharp_by_default() {
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
        .expect("solidify should succeed");

        for wall in &result.output.wall_faces {
            let edges = wall_edge_sharpness_classes(&mesh, *wall);
            let horizontal = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() < 1e-5)
                .map(|(_, _, sharpness)| *sharpness)
                .collect::<Vec<_>>();
            let vertical = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() >= 1e-5)
                .map(|(_, _, sharpness)| *sharpness)
                .collect::<Vec<_>>();
            assert_eq!(horizontal.len(), 2);
            assert_eq!(vertical.len(), 2);
            assert_eq!(horizontal, vec![1.0, 1.0]);
            assert_eq!(vertical, vec![1.0, 1.0]);
        }
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
                assert!(
                    exedra_mesh::op::set_corner_uv(&mut txn, corner, [index as f32, 0.0]).is_ok()
                );
                assert!(exedra_mesh::op::set_edge_seam(&mut txn, corner, true).is_ok());
                assert!(exedra_mesh::op::set_edge_sharpness(&mut txn, corner, 2.5).is_ok());
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
            .sparse(exedra_mesh::attr::CORNER_UV)
            .expect("corner uv layer should exist");
        for corner in mesh.face_loop(cap) {
            assert!(uv_layer.get(corner.as_id()).is_some());
            assert_eq!(mesh.edge_seam(corner), Some(true));
            assert_eq!(mesh.edge_sharpness(corner), Some(1.0));
        }
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn extrude_marks_cap_boundary_and_wall_columns_sharp_by_default() {
        let (mut mesh, face) = quad_mesh();
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

        for wall in &result.output.wall_faces {
            let edges = wall_edge_sharpness_classes(&mesh, *wall);
            let horizontal = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() < 1e-5)
                .collect::<Vec<_>>();
            let vertical = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() >= 1e-5)
                .collect::<Vec<_>>();
            assert_eq!(horizontal.len(), 2);
            assert_eq!(vertical.len(), 2);
            let top = horizontal
                .iter()
                .find(|(from, to, _)| from[2] > 1e-5 && to[2] > 1e-5)
                .expect("top wall edge should exist");
            let bottom = horizontal
                .iter()
                .find(|(from, to, _)| from[2].abs() < 1e-5 && to[2].abs() < 1e-5)
                .expect("bottom wall edge should exist");
            assert_eq!(top.2, 1.0);
            assert_eq!(bottom.2, 0.0);
            assert_eq!(vertical[0].2, 1.0);
            assert_eq!(vertical[1].2, 1.0);
        }
    }

    #[test]
    fn inset_then_shell_extrude_marks_step_boundary_sharp() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
        let inset = commit(
            &mut runner,
            &mut mesh,
            &InsetFaces,
            &InsetFacesParams {
                faces: vec![face],
                factor: 0.25,
            },
        )
        .expect("inset should succeed");
        let extrude = commit(
            &mut runner,
            &mut mesh,
            &ExtrudeFaces,
            &ExtrudeFacesParams {
                faces: inset.output.inner_faces.clone(),
                mode: ExtrudeMode::ShellOpen,
                distance: 0.5,
            },
        )
        .expect("extrude should succeed");

        for wall in &extrude.output.wall_faces {
            let edges = wall_edge_sharpness_classes(&mesh, *wall);
            let horizontal = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() < 1e-5)
                .collect::<Vec<_>>();
            let vertical = edges
                .iter()
                .filter(|(from, to, _)| (from[2] - to[2]).abs() >= 1e-5)
                .collect::<Vec<_>>();
            assert_eq!(horizontal.len(), 2);
            assert_eq!(vertical.len(), 2);
            assert_eq!(horizontal[0].2, 1.0);
            assert_eq!(horizontal[1].2, 1.0);
            assert_eq!(vertical[0].2, 1.0);
            assert_eq!(vertical[1].2, 1.0);
        }
    }

    #[test]
    fn inset_clear_policy_resets_generated_edge_tags() {
        let (mut mesh, face) = quad_mesh();
        let corners = mesh.face_loop(face).collect::<Vec<_>>();
        {
            let mut txn = mesh.edit();
            for &corner in &corners {
                assert!(exedra_mesh::op::set_edge_seam(&mut txn, corner, true).is_ok());
                assert!(exedra_mesh::op::set_edge_sharpness(&mut txn, corner, 3.0).is_ok());
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

        let inner = result.output.inner_faces[0];
        for corner in mesh.face_loop(inner) {
            assert_eq!(mesh.edge_seam(corner), Some(false));
            assert_eq!(mesh.edge_sharpness(corner), Some(1.0));
        }
        for frame in &result.output.frame_faces {
            let edges = wall_edge_sharpness_classes(&mesh, *frame);
            let sharp_count = edges
                .iter()
                .filter(|(_, _, sharpness)| (*sharpness - 1.0).abs() < 1e-5)
                .count();
            let smooth_count = edges
                .iter()
                .filter(|(_, _, sharpness)| sharpness.abs() < 1e-5)
                .count();
            assert_eq!(sharp_count, 1);
            assert_eq!(smooth_count, 3);
            for corner in mesh.face_loop(*frame) {
                assert_eq!(mesh.edge_seam(corner), Some(false));
            }
        }
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn inset_marks_inner_perimeter_sharp_by_default() {
        let (mut mesh, face) = quad_mesh();
        let mut runner = OperatorRunner::new();
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
        let inner = result.output.inner_faces[0];
        for corner in mesh.face_loop(inner) {
            assert_eq!(mesh.edge_sharpness(corner), Some(1.0));
        }
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
    fn cut_rect_marks_opening_perimeter_sharp_by_default() {
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

        let inner = result.output.inner_faces[0];
        for corner in mesh.face_loop(inner) {
            assert_eq!(mesh.edge_sharpness(corner), Some(1.0));
        }
    }

    #[test]
    fn wall_opening_then_solidify_keeps_opening_side_boundaries_hard() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 3.0, 0.0],
                [0.0, 3.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("wall build should succeed");
        let face = mesh.faces().next().expect("wall face should exist");
        let mut runner = OperatorRunner::new();
        let cut = commit(
            &mut runner,
            &mut mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [2.0, 0.0, 0.0],
                frame_v: [0.0, 3.0, 0.0],
                rect_min: [0.25, 0.2],
                rect_max: [0.75, 0.8],
            },
        )
        .expect("cut_rect should succeed");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: cut.output.inner_faces.clone(),
                policy: exedra_mesh::DeletePolicy::KeepIsolated,
            },
        )
        .expect("delete inner face should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let solidify = commit(
            &mut runner,
            &mut mesh,
            &SolidifyFaces,
            &SolidifyFacesParams {
                faces,
                mode: SolidifyMode::KeepSource,
                thickness: 0.15,
            },
        )
        .expect("solidify should succeed");

        assert!(
            solidify.output.wall_faces.iter().any(|wall| {
                wall_edge_sharpness_classes(&mesh, *wall)
                    .iter()
                    .all(|(_, _, sharpness)| (*sharpness - 1.0).abs() < 1e-5)
            }),
            "opening side walls should have hard feature edges"
        );
    }

    fn normal_variants_at_position(mesh: &Mesh, position: [f32; 3]) -> Vec<[u32; 3]> {
        let (tri, _) = mesh.to_trimesh(&exedra_mesh::ExtractParams::default());
        let mut variants = tri
            .positions
            .iter()
            .zip(tri.normals.iter())
            .filter_map(|(candidate, normal)| {
                ((candidate[0] - position[0]).abs() < 1.0e-5
                    && (candidate[1] - position[1]).abs() < 1.0e-5
                    && (candidate[2] - position[2]).abs() < 1.0e-5)
                    .then_some([
                        normal[0].to_bits(),
                        normal[1].to_bits(),
                        normal[2].to_bits(),
                    ])
            })
            .collect::<Vec<_>>();
        variants.sort_unstable();
        variants.dedup();
        variants
    }

    #[test]
    fn wall_opening_corner_extracts_three_normal_variants() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 3.0, 0.0],
                [0.0, 3.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("wall build should succeed");
        let face = mesh.faces().next().expect("wall face should exist");
        let mut runner = OperatorRunner::new();
        let cut = commit(
            &mut runner,
            &mut mesh,
            &CutRectFace,
            &CutRectFaceParams {
                face,
                frame_origin: [0.0, 0.0, 0.0],
                frame_u: [2.0, 0.0, 0.0],
                frame_v: [0.0, 3.0, 0.0],
                rect_min: [0.25, 0.2],
                rect_max: [0.75, 0.8],
            },
        )
        .expect("cut_rect should succeed");
        let top_left_opening_corner = cut
            .output
            .boundary_edges
            .iter()
            .filter_map(|&edge| {
                let vertex = mesh.to_vertex(edge)?;
                let position = mesh.vertex_position(vertex).copied()?;
                Some(position)
            })
            .min_by(|a, b| {
                b[1].total_cmp(&a[1])
                    .then_with(|| a[0].total_cmp(&b[0]))
                    .then_with(|| a[2].total_cmp(&b[2]))
            })
            .expect("cut output should expose opening boundary vertices");
        let _ = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: cut.output.inner_faces.clone(),
                policy: exedra_mesh::DeletePolicy::KeepIsolated,
            },
        )
        .expect("delete inner face should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let _ = commit(
            &mut runner,
            &mut mesh,
            &SolidifyFaces,
            &SolidifyFacesParams {
                faces,
                mode: SolidifyMode::KeepSource,
                thickness: 0.15,
            },
        )
        .expect("solidify should succeed");
        let variants = normal_variants_at_position(&mesh, top_left_opening_corner);
        assert_eq!(
            variants.len(),
            3,
            "variants at {top_left_opening_corner:?}: {variants:?}"
        );
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
                policy: exedra_mesh::DeletePolicy::KeepIsolated,
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
