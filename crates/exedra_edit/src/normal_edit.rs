// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face-scoped normal editing operators.

use alloc::format;

use exedra_mesh::{CornerId, FaceId, NormalParams};

use crate::op_common::op_error;
use crate::plan::PlanHasher;
use crate::selection::FaceSet;
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

use exedra_mesh_ops::normal_edit::{NormalEdit, NormalEditParams, NormalEditPlan};

/// Prepared removal of authored corner-normal overrides.
pub type ClearCornerNormalsPlan = NormalEditPlan;
/// Prepared per-face geometric normals.
pub type BakeFaceNormalsPlan = NormalEditPlan;
/// Prepared current derived normals.
pub type BakeDerivedNormalsPlan = NormalEditPlan;
/// Prepared normals with selected boundaries hardened during derivation.
pub type SmoothFaceNormalsPlan = NormalEditPlan;

/// Shared typed output from face-scoped normal editing operators.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NormalFacesOutput {
    /// Canonical face selection that was edited.
    pub faces: FaceSet,
}

/// Parameters for [`ClearCornerNormals`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClearCornerNormalsParams {
    /// Canonical face selection.
    pub faces: FaceSet,
}

/// `edit.normal.clear` operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct ClearCornerNormals;

impl EditOperator for ClearCornerNormals {
    type Params = ClearCornerNormalsParams;
    type Plan = ClearCornerNormalsPlan;
    type Output = NormalFacesOutput;

    fn name(&self) -> &'static str {
        "edit.normal.clear"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        NormalEditPlan::prepare(
            mesh,
            &NormalEditParams {
                faces: params.faces.clone(),
                mode: NormalEdit::Clear,
            },
        )
        .map_err(|error| normal_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = plan.apply(txn).map_err(|error| normal_error(ctx, error))?;
        Ok(normal_result(self.name(), output, ctx))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        fingerprint_plan(
            self.name(),
            plan.faces(),
            plan.writes(),
            plan.selections_canonicalized(),
        )
    }
}

/// Parameters for [`BakeFaceNormals`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BakeFaceNormalsParams {
    /// Canonical face selection.
    pub faces: FaceSet,
}

/// `edit.normal.face` operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct BakeFaceNormals;

impl EditOperator for BakeFaceNormals {
    type Params = BakeFaceNormalsParams;
    type Plan = BakeFaceNormalsPlan;
    type Output = NormalFacesOutput;

    fn name(&self) -> &'static str {
        "edit.normal.face"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        NormalEditPlan::prepare(
            mesh,
            &NormalEditParams {
                faces: params.faces.clone(),
                mode: NormalEdit::Face,
            },
        )
        .map_err(|error| normal_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = plan.apply(txn).map_err(|error| normal_error(ctx, error))?;
        Ok(normal_result(self.name(), output, ctx))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        fingerprint_plan(
            self.name(),
            plan.faces(),
            plan.writes(),
            plan.selections_canonicalized(),
        )
    }
}

/// Parameters for [`BakeDerivedNormals`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeDerivedNormalsParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Parameters used to derive normals before baking.
    pub normal_params: NormalParams,
}

/// `edit.normal.average` operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct BakeDerivedNormals;

impl EditOperator for BakeDerivedNormals {
    type Params = BakeDerivedNormalsParams;
    type Plan = BakeDerivedNormalsPlan;
    type Output = NormalFacesOutput;

    fn name(&self) -> &'static str {
        "edit.normal.average"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        NormalEditPlan::prepare(
            mesh,
            &NormalEditParams {
                faces: params.faces.clone(),
                mode: NormalEdit::Derived(params.normal_params),
            },
        )
        .map_err(|error| normal_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = plan.apply(txn).map_err(|error| normal_error(ctx, error))?;
        Ok(normal_result(self.name(), output, ctx))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        fingerprint_plan(
            self.name(),
            plan.faces(),
            plan.writes(),
            plan.selections_canonicalized(),
        )
    }
}

/// Parameters for [`SmoothFaceNormals`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmoothFaceNormalsParams {
    /// Canonical face selection.
    pub faces: FaceSet,
    /// Parameters used to derive smoothed normals before baking.
    pub normal_params: NormalParams,
}

/// `edit.normal.smooth` operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct SmoothFaceNormals;

impl EditOperator for SmoothFaceNormals {
    type Params = SmoothFaceNormalsParams;
    type Plan = SmoothFaceNormalsPlan;
    type Output = NormalFacesOutput;

    fn name(&self) -> &'static str {
        "edit.normal.smooth"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        NormalEditPlan::prepare(
            mesh,
            &NormalEditParams {
                faces: params.faces.clone(),
                mode: NormalEdit::Smooth(params.normal_params),
            },
        )
        .map_err(|error| normal_error(ctx, error))
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let output = plan.apply(txn).map_err(|error| normal_error(ctx, error))?;
        Ok(normal_result(self.name(), output, ctx))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        fingerprint_plan(
            self.name(),
            plan.faces(),
            plan.writes(),
            plan.selections_canonicalized(),
        )
    }
}

fn fingerprint_plan(
    op_name: &str,
    faces: &[FaceId],
    writes: &[(CornerId, Option<[f32; 3]>)],
    selections_canonicalized: bool,
) -> crate::PlanFingerprint {
    let mut hasher = PlanHasher::new();
    hasher.write_str(op_name);
    hasher.write_face_set(faces);
    hasher.write_u8(u8::from(selections_canonicalized));
    for &(corner, normal) in writes {
        hasher.write_u32(corner.index());
        match normal {
            Some(value) => {
                hasher.write_u8(1);
                hasher.write_u32(value[0].to_bits());
                hasher.write_u32(value[1].to_bits());
                hasher.write_u32(value[2].to_bits());
            }
            None => hasher.write_u8(0),
        }
    }
    hasher.finish()
}

fn normal_error(ctx: &OpContext, error: exedra_mesh_ops::normal_edit::NormalEditError) -> OpError {
    use exedra_mesh_ops::normal_edit::NormalEditError as E;
    let (kind, code) = match &error {
        E::Write(_) | E::BoundarySharpness(_) => (
            OpErrorKind::InternalInvariantViolation,
            DiagCode::InternalInvariantViolation,
        ),
        E::NumericLimit | E::InvalidParameters => {
            (OpErrorKind::NumericFailure, DiagCode::NumericToleranceIssue)
        }
        _ => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
        ),
    };
    op_error(ctx, kind, code, format!("{error}"))
}

fn normal_result(
    name: &'static str,
    output: exedra_mesh_ops::normal_edit::NormalEditOutput,
    ctx: &OpContext,
) -> (OpReport, NormalFacesOutput) {
    let mut report = OpReport::new(
        name,
        Artifacts::new(
            ctx.policy.limits.max_artifact_items,
            ctx.policy.limits.max_artifact_bytes,
        ),
    );
    report.stats.counters.selections_canonicalized = u64::from(output.selections_canonicalized);
    report.stats.counters.faces_processed = output.faces.len() as u64;
    report.stats.counters.corners_written = output.corners_written;
    report.stats.elements_touched.faces = report.stats.counters.faces_processed;
    (
        report,
        NormalFacesOutput {
            faces: output.faces,
        },
    )
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::{
        BakeDerivedNormals, BakeDerivedNormalsParams, BakeFaceNormals, BakeFaceNormalsParams,
        ClearCornerNormals, ClearCornerNormalsParams, ExtractParams, NormalsSource, OpErrorKind,
        OperatorRunner, SmoothFaceNormals, SmoothFaceNormalsParams,
    };
    use exedra_mesh::{BuildParams, FaceId, Mesh, NormalParams, attr, op};

    fn usize_to_u32(value: usize) -> u32 {
        u32::try_from(value).expect("test mesh index should fit u32")
    }

    fn shared_strip_mesh() -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.5],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("strip mesh should build")
    }

    fn capped_cylinder_mesh(segments: usize) -> (Mesh, Vec<FaceId>) {
        let mut positions = Vec::with_capacity(segments * 2);
        for ring in [0.0_f32, 1.0_f32] {
            for i in 0..segments {
                let theta = (i as f32) * core::f32::consts::TAU / (segments as f32);
                positions.push([theta.cos(), ring, theta.sin()]);
            }
        }

        let mut polys = Vec::<Vec<u32>>::with_capacity(segments + 2);
        for i in 0..segments {
            let next = (i + 1) % segments;
            polys.push(vec![
                usize_to_u32(i),
                usize_to_u32(segments + i),
                usize_to_u32(segments + next),
                usize_to_u32(next),
            ]);
        }
        polys.push(
            (0..segments)
                .rev()
                .map(|i| usize_to_u32(segments + i))
                .collect(),
        );
        polys.push((0..segments).map(usize_to_u32).collect());
        let poly_refs = polys.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let mesh = Mesh::from_polygons(&positions, &poly_refs).expect("cylinder mesh should build");
        let side_faces = mesh.faces().take(segments).collect::<Vec<_>>();
        (mesh, side_faces)
    }

    #[test]
    fn clear_corner_normals_clears_selected_face_overrides() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad mesh should build");
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");
        let mut edit = mesh.edit();
        op::set_corner_normal_override(&mut edit, corner, Some([1.0, 0.0, 0.0]))
            .expect("write should succeed");
        let _: () = edit.finish();

        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(
                &mesh,
                &ClearCornerNormals,
                &ClearCornerNormalsParams { faces: vec![face] },
            )
            .expect("compile should succeed");
        let result = runner
            .apply_in_place(&mut mesh, &ClearCornerNormals, &plan)
            .expect("clear should succeed");
        assert_eq!(result.output.faces, vec![face]);
        assert_eq!(
            mesh.attrs()
                .sparse(attr::CORNER_NORMAL_OVERRIDE)
                .and_then(|layer| layer.get(corner.as_id())),
            None
        );
        let (tri, _) = mesh.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOrDerived,
            ..ExtractParams::default()
        });
        assert_eq!(tri.normals[0], [0.0, 0.0, 1.0]);
    }

    #[test]
    fn clear_corner_normals_compile_rejects_stale_face() {
        let mesh = Mesh::new();
        let stale = FaceId::from(exedra_mesh::Id::new(
            42,
            core::num::NonZeroU32::new(1).expect("1 is non-zero"),
        ));
        let mut runner = OperatorRunner::new();
        let err = runner
            .compile(
                &mesh,
                &ClearCornerNormals,
                &ClearCornerNormalsParams { faces: vec![stale] },
            )
            .expect_err("stale face should be rejected");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn bake_face_normals_writes_flat_normals_per_face() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad mesh should build");
        let face = mesh.faces().next().expect("face should exist");

        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(
                &mesh,
                &BakeFaceNormals,
                &BakeFaceNormalsParams { faces: vec![face] },
            )
            .expect("compile should succeed");
        let result = runner
            .apply_in_place(&mut mesh, &BakeFaceNormals, &plan)
            .expect("bake face normals should succeed");
        assert_eq!(result.output.faces, vec![face]);
        for corner in mesh.face_loop(face) {
            assert_eq!(
                mesh.attrs()
                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                    .and_then(|layer| layer.get(corner.as_id())),
                Some(&[0.0, 0.0, 1.0])
            );
        }
    }

    #[test]
    fn bake_face_normals_rejects_degenerate_face() {
        let mesh = Mesh::from_polygons(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            &[&[0, 1, 2]],
        )
        .expect("degenerate triangle topology should build");
        let face = mesh.faces().next().expect("face should exist");

        let mut runner = OperatorRunner::new();
        let err = runner
            .compile(
                &mesh,
                &BakeFaceNormals,
                &BakeFaceNormalsParams { faces: vec![face] },
            )
            .expect_err("degenerate face should be rejected");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn bake_derived_normals_writes_current_derived_values() {
        let mut mesh = shared_strip_mesh();
        let faces = mesh.faces().collect::<Vec<_>>();
        let expected = mesh.derive_corner_normals(&NormalParams::default());

        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(
                &mesh,
                &BakeDerivedNormals,
                &BakeDerivedNormalsParams {
                    faces: faces.clone(),
                    normal_params: NormalParams::default(),
                },
            )
            .expect("compile should succeed");
        let result = runner
            .apply_in_place(&mut mesh, &BakeDerivedNormals, &plan)
            .expect("bake derived normals should succeed");
        assert_eq!(result.output.faces, faces);
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                assert_eq!(
                    mesh.attrs()
                        .sparse(attr::CORNER_NORMAL_OVERRIDE)
                        .and_then(|layer| layer.get(corner.as_id()))
                        .copied(),
                    expected.get(corner)
                );
            }
        }
    }

    #[test]
    fn smooth_face_normals_matches_current_derived_values() {
        let mut mesh = shared_strip_mesh();
        let faces = mesh.faces().collect::<Vec<_>>();
        let params = NormalParams {
            auto_sharp_angle_degrees: Some(35.0),
            ..NormalParams::default()
        };

        let face_plan = OperatorRunner::new()
            .compile(
                &mesh,
                &BakeFaceNormals,
                &BakeFaceNormalsParams {
                    faces: faces.clone(),
                },
            )
            .expect("face-normal plan should compile");
        let mut runner = OperatorRunner::new();
        runner
            .apply_in_place(&mut mesh, &BakeFaceNormals, &face_plan)
            .expect("face-normal bake should succeed");

        let expected = mesh.derive_corner_normals(&params);
        let smooth_plan = runner
            .compile(
                &mesh,
                &SmoothFaceNormals,
                &SmoothFaceNormalsParams {
                    faces: faces.clone(),
                    normal_params: params,
                },
            )
            .expect("smooth plan should compile");
        let result = runner
            .apply_in_place(&mut mesh, &SmoothFaceNormals, &smooth_plan)
            .expect("smooth apply should succeed");
        assert_eq!(result.output.faces, faces);
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                assert_eq!(
                    mesh.attrs()
                        .sparse(attr::CORNER_NORMAL_OVERRIDE)
                        .and_then(|layer| layer.get(corner.as_id()))
                        .copied(),
                    expected.get(corner)
                );
            }
        }
    }

    #[test]
    fn smooth_face_normals_on_cylinder_sides_stay_radial() {
        let (mut mesh, side_faces) = capped_cylinder_mesh(12);
        let params = NormalParams::default();
        let mut runner = OperatorRunner::new();
        let plan = runner
            .compile(
                &mesh,
                &SmoothFaceNormals,
                &SmoothFaceNormalsParams {
                    faces: side_faces.clone(),
                    normal_params: params,
                },
            )
            .expect("smooth compile should succeed");
        runner
            .apply_in_place(&mut mesh, &SmoothFaceNormals, &plan)
            .expect("smooth apply should succeed");

        for face in side_faces {
            for corner in mesh.face_loop(face) {
                let vertex = mesh.to_vertex(corner).expect("corner vertex should exist");
                let position = mesh
                    .vertex_position(vertex)
                    .copied()
                    .expect("vertex position should exist");
                let expected = {
                    let xz_len = (position[0] * position[0] + position[2] * position[2]).sqrt();
                    [position[0] / xz_len, 0.0, position[2] / xz_len]
                };
                let normal = mesh
                    .attrs()
                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                    .and_then(|layer| layer.get(corner.as_id()))
                    .copied()
                    .expect("smoothed side face should have authored normal");
                assert!(
                    normal[1].abs() < 1.0e-4,
                    "side normal should not tilt into the cap: {:?}",
                    normal
                );
                assert!(
                    (normal[0] - expected[0]).abs() < 1.0e-4
                        && (normal[2] - expected[2]).abs() < 1.0e-4,
                    "side normal should stay radial: actual {:?}, expected {:?}",
                    normal,
                    expected
                );
            }
        }
    }
}
