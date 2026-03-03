// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic planar UV projection operator.

use alloc::vec::Vec;

use exedra::{CornerId, FaceId};

use crate::{
    Artifact, Artifacts, DiagCode, DiagLevel, Diagnostic, EditOperator, FaceSet, OpContext,
    OpError, OpReport,
    uv_common::{face_normal, select_faces, stale_face_error},
};

/// Face selection scope for UV projection.
#[derive(Clone, Debug, PartialEq)]
pub enum UvScope {
    /// Process all faces in arena order.
    WholeMesh,
    /// Process an explicit face set (canonicalized before execution).
    FaceSet(FaceSet),
}

/// Projection plane mode.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UvPlane {
    /// Project from world XY.
    WorldXY,
    /// Project from world XZ.
    WorldXZ,
    /// Project from world YZ.
    WorldYZ,
    /// Choose per-face dominant axis from geometry.
    PerFaceFromGeometry,
}

/// Parameters for [`UvPlanar`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvPlanarParams {
    /// Face scope.
    pub scope: UvScope,
    /// Projection plane mode.
    pub plane: UvPlane,
    /// Uniform UV scale multiplier.
    pub scale: f32,
    /// UV offset after scale.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
    /// Epsilon used by per-face dominant-axis tie-breaking.
    pub normal_epsilon: f32,
}

impl Default for UvPlanarParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            plane: UvPlane::WorldXY,
            scale: 1.0,
            offset: [0.0, 0.0],
            write_missing_only: false,
            normal_epsilon: 1.0e-6,
        }
    }
}

/// Deterministic planar UV projection operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct UvPlanar;

impl EditOperator for UvPlanar {
    type Params = UvPlanarParams;

    fn name(&self) -> &'static str {
        "uv.planar"
    }

    fn apply(
        &self,
        txn: &mut exedra::Txn<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<OpReport, OpError> {
        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        let faces = {
            let _bucket = ctx.clock.bucket("select");
            select_faces(txn.mesh(), &params.scope)
        };
        if matches!(params.scope, UvScope::FaceSet(_)) && faces.changed {
            report.stats.counters.selections_canonicalized = 1;
        }
        if faces.faces.is_empty() {
            return Ok(report);
        }

        if report.artifacts.push(Artifact::FaceSet {
            name: "uv.planar.affected_faces".into(),
            faces: faces.faces.clone(),
        }) {
            // retained
        }

        let projection_mode = params.plane;
        let mut pending = Vec::<(CornerId, [f32; 2])>::new();
        {
            let _bucket = ctx.clock.bucket("compute");
            for face in faces.faces.iter().copied() {
                if txn.mesh().face_edge(face).is_none() {
                    return Err(stale_face_error(
                        self.name(),
                        face,
                        report.artifacts.clone(),
                    ));
                }
                let plane = match projection_mode {
                    UvPlane::PerFaceFromGeometry => {
                        let (resolved, fell_back) =
                            dominant_plane(txn.mesh(), face, params.normal_epsilon);
                        if fell_back {
                            ctx.diagnostics.push(Diagnostic::new(
                                DiagLevel::Warn,
                                DiagCode::NumericToleranceIssue,
                                alloc::format!(
                                    "uv.planar face {} has degenerate normal; falling back to WorldXY",
                                    face.index()
                                ),
                            ));
                        }
                        resolved
                    }
                    mode => mode,
                };
                for corner in txn.mesh().face_loop(face) {
                    if params.write_missing_only && txn.corner_uv(corner).is_some() {
                        report.stats.counters.corners_skipped_existing = report
                            .stats
                            .counters
                            .corners_skipped_existing
                            .saturating_add(1);
                        continue;
                    }
                    let uv = project_corner(txn.mesh(), corner, plane, params.scale, params.offset);
                    pending.push((corner, uv));
                }
                report.stats.counters.faces_processed =
                    report.stats.counters.faces_processed.saturating_add(1);
            }
        }

        {
            let _bucket = ctx.clock.bucket("attrs");
            for (corner, uv) in pending {
                if txn.set_corner_uv(corner, uv) {
                    report.stats.counters.corners_written =
                        report.stats.counters.corners_written.saturating_add(1);
                }
            }
        }

        Ok(report)
    }
}

fn project_corner(
    mesh: &exedra::Mesh,
    corner: CornerId,
    plane: UvPlane,
    scale: f32,
    offset: [f32; 2],
) -> [f32; 2] {
    let vertex = mesh
        .from_vertex(corner)
        .expect("face loop corner must have source vertex");
    let position = mesh
        .vertex_position(vertex)
        .expect("live vertex must have builtin position");
    let base = match plane {
        UvPlane::WorldXY => [position[0], position[1]],
        UvPlane::WorldXZ => [position[0], position[2]],
        UvPlane::WorldYZ => [position[1], position[2]],
        UvPlane::PerFaceFromGeometry => {
            unreachable!("PerFaceFromGeometry must be resolved before project_corner")
        }
    };
    [base[0] * scale + offset[0], base[1] * scale + offset[1]]
}

fn dominant_plane(mesh: &exedra::Mesh, face: FaceId, epsilon: f32) -> (UvPlane, bool) {
    let Some([nx, ny, nz]) = face_normal(mesh, face) else {
        return (UvPlane::WorldXY, true);
    };
    let ax = nx.abs();
    let ay = ny.abs();
    let az = nz.abs();
    let max_axis = ax.max(ay).max(az);
    if max_axis < epsilon {
        return (UvPlane::WorldXY, true);
    }
    if ax + epsilon >= max_axis {
        return (UvPlane::WorldYZ, false);
    }
    if ay + epsilon >= max_axis {
        return (UvPlane::WorldXZ, false);
    }
    (UvPlane::WorldXY, false)
}

#[cfg(test)]
mod tests {
    use exedra::{ExtractParams, MeshBuilder};

    use super::{UvPlanar, UvPlanarParams, UvPlane, UvScope};
    use crate::OperatorRunner;

    #[test]
    fn uv_planar_writes_uvs_and_extracts_trimesh() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let mut runner = OperatorRunner::new();
        let op = UvPlanar;
        let result = runner
            .run_commit(
                &mut mesh,
                &op,
                &UvPlanarParams {
                    scope: UvScope::WholeMesh,
                    plane: UvPlane::WorldXY,
                    scale: 2.0,
                    offset: [0.5, 1.0],
                    write_missing_only: false,
                    normal_epsilon: 1.0e-6,
                },
            )
            .expect("uv.planar should succeed");

        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 4);
        let (tri, stats) = mesh.to_trimesh(&ExtractParams::default());
        assert_eq!(stats.triangle_count, 2);
        assert_eq!(tri.indices.len(), 6);
        assert!(tri.uvs.contains(&[0.5, 1.0]));
        assert!(tri.uvs.contains(&[2.5, 3.0]));
    }

    #[test]
    fn uv_planar_write_missing_only_skips_existing_values() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("triangle should be valid");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");
        {
            let mut txn = mesh.begin();
            assert!(txn.set_corner_uv(corner, [9.0, 9.0]));
            let _ = txn.commit();
        }

        let mut runner = OperatorRunner::new();
        let result = runner
            .run_commit(
                &mut mesh,
                &UvPlanar,
                &UvPlanarParams {
                    scope: UvScope::WholeMesh,
                    plane: UvPlane::WorldXY,
                    scale: 1.0,
                    offset: [0.0, 0.0],
                    write_missing_only: true,
                    normal_epsilon: 1.0e-6,
                },
            )
            .expect("uv.planar should succeed");

        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 2);
        assert_eq!(result.report.stats.counters.corners_skipped_existing, 1);
        let uv = mesh
            .attrs()
            .sparse(exedra::attr::CORNER_UV)
            .and_then(|layer| layer.get(corner.as_id()))
            .copied();
        assert_eq!(uv, Some([9.0, 9.0]));
    }

    #[test]
    fn uv_planar_per_face_degenerate_geometry_emits_warning() {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([0.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([2.0, 0.0, 0.0]);
        builder
            .add_face(&[0, 1, 2])
            .expect("topology is valid even when geometry is degenerate");
        let mut mesh = builder.build().expect("build should succeed").mesh;
        let mut runner = OperatorRunner::new();
        let _ = runner
            .run_commit(
                &mut mesh,
                &UvPlanar,
                &UvPlanarParams {
                    scope: UvScope::WholeMesh,
                    plane: UvPlane::PerFaceFromGeometry,
                    scale: 1.0,
                    offset: [0.0, 0.0],
                    write_missing_only: false,
                    normal_epsilon: 1.0e-6,
                },
            )
            .expect("uv.planar should succeed");

        assert!(runner.ctx.diagnostics.iter().any(|diag| {
            diag.level == crate::DiagLevel::Warn
                && diag.code == crate::DiagCode::NumericToleranceIssue
        }));
    }
}
