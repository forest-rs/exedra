// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic cylinder UV projection operator.

use alloc::vec::Vec;

use exedra::CornerId;

use crate::{
    Artifact, Artifacts, EditOperator, OpContext, OpError, OpReport, UvScope,
    math::FloatExt,
    uv_common::{select_faces, stale_face_error},
};

const TAU: f32 = core::f32::consts::PI * 2.0;

/// Cylinder axis selection.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CylinderAxis {
    /// Cylinder axis aligned with +X/-X.
    X,
    /// Cylinder axis aligned with +Y/-Y.
    Y,
    /// Cylinder axis aligned with +Z/-Z.
    Z,
}

/// Parameters for [`UvCylinder`].
#[derive(Clone, Debug, PartialEq)]
pub struct UvCylinderParams {
    /// Face scope.
    pub scope: UvScope,
    /// Cylinder axis.
    pub axis: CylinderAxis,
    /// Seam angle offset in radians.
    pub seam_offset_radians: f32,
    /// UV scale.
    ///
    /// The V coordinate is raw world-space distance along the selected axis before
    /// scale and offset (it is not normalized to `[0, 1]`).
    pub scale: [f32; 2],
    /// UV offset.
    pub offset: [f32; 2],
    /// When true, only writes missing corner UV values.
    pub write_missing_only: bool,
}

impl Default for UvCylinderParams {
    fn default() -> Self {
        Self {
            scope: UvScope::WholeMesh,
            axis: CylinderAxis::Y,
            seam_offset_radians: 0.0,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            write_missing_only: false,
        }
    }
}

/// Deterministic cylinder projection UV operator.
#[derive(Copy, Clone, Debug, Default)]
pub struct UvCylinder;

impl EditOperator for UvCylinder {
    type Params = UvCylinderParams;
    type Plan = UvCylinderParams;
    type Output = crate::FaceSet;

    fn name(&self) -> &'static str {
        "uv.cylinder"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
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
            return Ok((report, crate::FaceSet::new()));
        }
        let _ = report.artifacts.push(Artifact::FaceSet {
            name: "uv.cylinder.affected_faces".into(),
            faces: faces.faces.clone(),
        });

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
                for corner in txn.mesh().face_loop(face) {
                    if params.write_missing_only && txn.corner_uv(corner).is_some() {
                        report.stats.counters.corners_skipped_existing = report
                            .stats
                            .counters
                            .corners_skipped_existing
                            .saturating_add(1);
                        continue;
                    }
                    pending.push((corner, project_corner_cylinder(txn.mesh(), corner, params)));
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
        Ok((report, faces.faces))
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

fn project_corner_cylinder(
    mesh: &exedra::Mesh,
    corner: CornerId,
    params: &UvCylinderParams,
) -> [f32; 2] {
    let vertex = mesh
        .from_vertex(corner)
        .expect("face loop corner must have source vertex");
    let p = *mesh
        .vertex_position(vertex)
        .expect("live vertex must have builtin position");

    let (radial_a, radial_b, axial) = match params.axis {
        CylinderAxis::X => (p[1], p[2], p[0]),
        CylinderAxis::Y => (p[2], p[0], p[1]),
        CylinderAxis::Z => (p[0], p[1], p[2]),
    };
    let theta = radial_a.atan2_ext(radial_b) + params.seam_offset_radians;
    let u = (theta / TAU).rem_euclid_ext(1.0);
    let v = axial;
    [
        u * params.scale[0] + params.offset[0],
        v * params.scale[1] + params.offset[1],
    ]
}

#[cfg(test)]
mod tests {
    use exedra::MeshBuilder;

    use super::{CylinderAxis, UvCylinder, UvCylinderParams};
    use crate::{OperatorRunner, UvScope, test_support::commit};

    fn side_strip_mesh() -> exedra::Mesh {
        let mut builder = MeshBuilder::new();
        builder.push_vertex([1.0, 0.0, 0.0]);
        builder.push_vertex([1.0, 1.0, 0.0]);
        builder.push_vertex([0.0, 1.0, 1.0]);
        builder.push_vertex([0.0, 0.0, 1.0]);
        builder
            .add_face(&[0, 1, 2, 3])
            .expect("quad should be valid");
        builder.build().expect("build should succeed").mesh
    }

    #[test]
    fn uv_cylinder_projects_with_configurable_axis() {
        let mut mesh = side_strip_mesh();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &UvCylinder,
            &UvCylinderParams {
                scope: UvScope::WholeMesh,
                axis: CylinderAxis::Y,
                seam_offset_radians: 0.0,
                scale: [1.0, 1.0],
                offset: [0.0, 0.0],
                write_missing_only: false,
            },
        )
        .expect("uv.cylinder should succeed");
        assert_eq!(result.report.stats.counters.faces_processed, 1);
        assert_eq!(result.report.stats.counters.corners_written, 4);
    }

    #[test]
    fn uv_cylinder_seam_placement_is_deterministic() {
        let mut mesh_a = side_strip_mesh();
        let mut mesh_b = side_strip_mesh();
        let mut runner_a = OperatorRunner::new();
        let mut runner_b = OperatorRunner::new();
        let params = UvCylinderParams {
            scope: UvScope::WholeMesh,
            axis: CylinderAxis::Z,
            seam_offset_radians: 0.75,
            scale: [1.0, 1.0],
            offset: [0.0, 0.0],
            write_missing_only: false,
        };

        let _ =
            commit(&mut runner_a, &mut mesh_a, &UvCylinder, &params).expect("run should succeed");
        let _ =
            commit(&mut runner_b, &mut mesh_b, &UvCylinder, &params).expect("run should succeed");

        let (tri_a, _) = mesh_a.to_trimesh(&exedra::ExtractParams::default());
        let (tri_b, _) = mesh_b.to_trimesh(&exedra::ExtractParams::default());
        assert_eq!(tri_a.uvs, tri_b.uvs);
        assert!(tri_a.uvs.iter().all(|uv| uv[0] >= 0.0 && uv[0] < 1.0));
    }
}
