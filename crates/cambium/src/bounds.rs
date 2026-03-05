// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounds inspection operator.

use alloc::format;
use alloc::vec::Vec;

use exedra::FaceId;

use crate::math::FloatExt;
use crate::op_common::op_error;
use crate::selection::{FaceSet, canonicalize_face_set};
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

/// Face selection scope for [`InspectBounds`].
#[derive(Clone, Debug, PartialEq)]
pub enum BoundsScope {
    /// Use all mesh vertices.
    WholeMesh,
    /// Use vertices referenced by selected faces.
    FaceSet(FaceSet),
}

/// Parameters for [`InspectBounds`].
#[derive(Clone, Debug, PartialEq)]
pub struct BoundsParams {
    /// Selection scope.
    pub scope: BoundsScope,
}

impl Default for BoundsParams {
    fn default() -> Self {
        Self {
            scope: BoundsScope::WholeMesh,
        }
    }
}

/// Axis-aligned bounds summary.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundsSummary {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
    /// Arithmetic center of included points.
    pub centroid: [f32; 3],
    /// Length of diagonal (`|max - min|`).
    pub diagonal: f32,
}

/// Typed output from [`InspectBounds`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BoundsOutput {
    /// Computed bounds, `None` when no vertices are selected.
    pub bounds: Option<BoundsSummary>,
    /// Number of vertices included in the computation.
    pub vertex_count: u64,
    /// Number of faces processed by the selected scope.
    pub face_count: u64,
}

/// Deterministic bounds inspection operator.
///
/// # Example
/// ```rust
/// use cambium::{BoundsParams, InspectBounds, OperatorRunner};
/// use exedra::{BuildParams, Mesh};
///
/// let mut mesh = Mesh::from_indexed_triangles(
///     &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
///     &[[0, 1, 2]],
///     &BuildParams::default(),
/// )
/// .expect("triangle build should succeed");
/// let mut runner = OperatorRunner::new();
/// let result = runner
///     .run_commit(&mut mesh, &InspectBounds, &BoundsParams::default())
///     .expect("bounds should succeed");
/// let bounds = result.output.bounds.expect("triangle should have bounds");
/// assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
/// assert_eq!(bounds.max, [2.0, 2.0, 0.0]);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct InspectBounds;

impl EditOperator for InspectBounds {
    type Params = BoundsParams;
    type Output = BoundsOutput;

    fn name(&self) -> &'static str {
        "inspect.bounds"
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

        let mut acc = BoundsAccumulator::default();
        let mut face_count = 0_u64;

        match &params.scope {
            BoundsScope::WholeMesh => {
                face_count =
                    u64::try_from(txn.mesh().faces().count()).expect("face count should fit u64");
                report.stats.counters.faces_processed = face_count;
                for vertex in txn.mesh().vertices() {
                    if let Some(position) = txn.mesh().vertex_position(vertex) {
                        acc.push(*position);
                    }
                }
            }
            BoundsScope::FaceSet(input) => {
                let mut faces = input.clone();
                if canonicalize_face_set(&mut faces) {
                    report.stats.counters.selections_canonicalized = 1;
                }
                let mut vertices = Vec::<exedra::VertexId>::new();
                for face in faces {
                    if face == FaceId::OUTSIDE {
                        return Err(op_error(
                            ctx,
                            OpErrorKind::PreconditionFailed,
                            DiagCode::PreconditionFailed,
                            "selection contains FaceId::OUTSIDE",
                        ));
                    }
                    if txn.mesh().face_edge(face).is_none() {
                        return Err(op_error(
                            ctx,
                            OpErrorKind::PreconditionFailed,
                            DiagCode::PreconditionFailed,
                            format!("selection contains stale face id: {}", face.index()),
                        ));
                    }
                    face_count = face_count.saturating_add(1);
                    report.stats.counters.faces_processed =
                        report.stats.counters.faces_processed.saturating_add(1);
                    for corner in txn.mesh().face_loop(face) {
                        let vertex = txn.mesh().to_vertex(corner).ok_or_else(|| {
                            op_error(
                                ctx,
                                OpErrorKind::InvalidMesh,
                                DiagCode::InternalInvariantViolation,
                                "face loop contains corner with missing destination vertex",
                            )
                        })?;
                        vertices.push(vertex);
                    }
                }
                vertices.sort_unstable();
                vertices.dedup();
                for vertex in vertices {
                    if let Some(position) = txn.mesh().vertex_position(vertex) {
                        acc.push(*position);
                    }
                }
            }
        }

        let bounds = acc.bounds();
        let vertex_count = acc.count;
        report.stats.elements_touched.vertices = vertex_count;
        report.stats.elements_touched.faces = face_count;

        Ok((
            report,
            BoundsOutput {
                bounds,
                vertex_count,
                face_count,
            },
        ))
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct BoundsAccumulator {
    min: Option<[f32; 3]>,
    max: [f32; 3],
    sum: [f32; 3],
    count: u64,
}

impl BoundsAccumulator {
    fn push(&mut self, point: [f32; 3]) {
        if let Some(min) = self.min.as_mut() {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            min[2] = min[2].min(point[2]);
            self.max[0] = self.max[0].max(point[0]);
            self.max[1] = self.max[1].max(point[1]);
            self.max[2] = self.max[2].max(point[2]);
        } else {
            self.min = Some(point);
            self.max = point;
        }
        self.sum[0] += point[0];
        self.sum[1] += point[1];
        self.sum[2] += point[2];
        self.count = self.count.saturating_add(1);
    }

    fn bounds(self) -> Option<BoundsSummary> {
        let min = self.min?;
        let inv = 1.0 / (self.count as f32);
        let centroid = [self.sum[0] * inv, self.sum[1] * inv, self.sum[2] * inv];
        let dx = self.max[0] - min[0];
        let dy = self.max[1] - min[1];
        let dz = self.max[2] - min[2];
        Some(BoundsSummary {
            min,
            max: self.max,
            centroid,
            diagonal: (dx * dx + dy * dy + dz * dz).sqrt_ext(),
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra::{BuildParams, FaceId, Id, Mesh};

    use super::{BoundsParams, BoundsScope, InspectBounds};
    use crate::{OpErrorKind, OperatorRunner};

    #[test]
    fn bounds_empty_mesh_returns_none() {
        let mut mesh = Mesh::new();
        let mut runner = OperatorRunner::new();
        let result = runner
            .run_commit(&mut mesh, &InspectBounds, &BoundsParams::default())
            .expect("bounds should succeed");
        assert!(result.output.bounds.is_none());
        assert_eq!(result.output.vertex_count, 0);
        assert_eq!(result.output.face_count, 0);
    }

    #[test]
    fn bounds_whole_mesh_triangle_matches_expected_values() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let mut runner = OperatorRunner::new();
        let result = runner
            .run_commit(&mut mesh, &InspectBounds, &BoundsParams::default())
            .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [2.0, 2.0, 0.0]);
        assert_eq!(bounds.centroid, [2.0 / 3.0, 2.0 / 3.0, 0.0]);
        assert!((bounds.diagonal - (8.0_f32).sqrt()).abs() < 1e-6);
        assert_eq!(result.output.vertex_count, 3);
        assert_eq!(result.output.face_count, 1);
    }

    #[test]
    fn bounds_face_set_uses_selected_faces_only() {
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
        let result = runner
            .run_commit(
                &mut mesh,
                &InspectBounds,
                &BoundsParams {
                    scope: BoundsScope::FaceSet(vec![faces[1]]),
                },
            )
            .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 1.0, 0.0]);
        assert_eq!(result.output.vertex_count, 3);
        assert_eq!(result.output.face_count, 1);
    }

    #[test]
    fn bounds_face_set_multiple_faces_dedupes_vertices() {
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
        let result = runner
            .run_commit(
                &mut mesh,
                &InspectBounds,
                &BoundsParams {
                    scope: BoundsScope::FaceSet(vec![faces[1], faces[0]]),
                },
            )
            .expect("bounds should succeed");
        let bounds = result.output.bounds.expect("bounds should exist");
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 1.0, 0.0]);
        assert_eq!(bounds.centroid, [0.5, 0.5, 0.0]);
        assert!((bounds.diagonal - (2.0_f32).sqrt()).abs() < 1e-6);
        assert_eq!(result.output.vertex_count, 4);
        assert_eq!(result.output.face_count, 2);
    }

    #[test]
    fn bounds_face_set_rejects_stale_face() {
        let mut mesh = Mesh::new();
        let stale = FaceId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();
        let err = runner
            .run_commit(
                &mut mesh,
                &InspectBounds,
                &BoundsParams {
                    scope: BoundsScope::FaceSet(vec![stale]),
                },
            )
            .expect_err("stale face should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }
}
