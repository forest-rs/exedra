// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face/edge deletion edit operators.

use alloc::format;
use alloc::string::String;

use exedra::{DeleteFacesError, DeletePolicy, DeleteVerticesError, FaceId, HalfEdgeId};

use crate::op_common::op_error;
use crate::plan::PlanHasher;
use crate::selection::{
    EdgeSet, FaceSet, VertexSet, canonicalize_edge_set, canonicalize_face_set,
    canonicalize_vertex_set,
};
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

/// Parameters for [`DeleteEdges`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteEdgesParams {
    /// Canonical undirected edge selection.
    pub edges: EdgeSet,
    /// Isolated-vertex cleanup policy.
    pub policy: DeletePolicy,
}

impl Default for DeleteEdgesParams {
    fn default() -> Self {
        Self {
            edges: EdgeSet::default(),
            policy: DeletePolicy::CleanupIsolated,
        }
    }
}

/// Typed output from [`DeleteEdges`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteEdgesOutput {
    /// Canonical edge selection that was applied.
    pub edges: EdgeSet,
    /// Canonical interior face set deleted as a consequence.
    pub faces: FaceSet,
}

/// Parameters for [`DeleteFaces`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteFacesParams {
    /// Canonical interior face selection.
    pub faces: FaceSet,
    /// Isolated-vertex cleanup policy.
    pub policy: DeletePolicy,
}

impl Default for DeleteFacesParams {
    fn default() -> Self {
        Self {
            faces: FaceSet::default(),
            policy: DeletePolicy::CleanupIsolated,
        }
    }
}

/// Typed output from [`DeleteFaces`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteFacesOutput {
    /// Canonical face selection that was applied.
    pub faces: FaceSet,
}

/// Deterministic compiled plan payload for [`DeleteFaces`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteFacesPlan {
    /// Canonical interior face selection.
    pub faces: FaceSet,
    /// Isolated-vertex cleanup policy.
    pub policy: DeletePolicy,
    /// Whether face selection order/duplication changed during compile.
    pub canonicalized: bool,
}

/// Parameters for [`DeleteVertices`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteVerticesParams {
    /// Canonical isolated vertex selection.
    pub vertices: VertexSet,
}

/// Typed output from [`DeleteVertices`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeleteVerticesOutput {
    /// Canonical vertex selection that was applied.
    pub vertices: VertexSet,
}

/// `edit.delete.edges` operator.
///
/// # Example
/// ```rust
/// use cambium::{DeleteEdges, DeleteEdgesParams, OperatorRunner};
/// use exedra::{BuildParams, DeletePolicy, Mesh};
///
/// let mut mesh = Mesh::from_indexed_triangles(
///     &[
///         [0.0, 0.0, 0.0],
///         [1.0, 0.0, 0.0],
///         [0.0, 1.0, 0.0],
///         [1.0, 1.0, 0.0],
///     ],
///     &[[0, 1, 2], [2, 1, 3]],
///     &BuildParams::default(),
/// )
/// .expect("strip build should succeed");
/// let edge = mesh
///     .faces()
///     .flat_map(|face| mesh.face_loop(face))
///     .find(|&half_edge| {
///         let Some(twin) = mesh.twin(half_edge) else {
///             return false;
///         };
///         core::cmp::min(half_edge, twin) == half_edge
///             && mesh.face(half_edge) != Some(exedra::FaceId::OUTSIDE)
///             && mesh.face(twin) != Some(exedra::FaceId::OUTSIDE)
///     })
///     .expect("interior edge should exist");
///
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(
///         &mesh,
///         &DeleteEdges,
///         &DeleteEdgesParams {
///             edges: vec![edge],
///             policy: DeletePolicy::CleanupIsolated,
///         },
///     )
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &DeleteEdges, &plan)
///     .expect("delete edges should succeed");
/// assert_eq!(result.output.edges, vec![edge]);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct DeleteEdges;

impl EditOperator for DeleteEdges {
    type Params = DeleteEdgesParams;
    type Plan = DeleteEdgesParams;
    type Output = DeleteEdgesOutput;

    fn name(&self) -> &'static str {
        "edit.delete.edges"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut edges = params.edges.clone();
        let canonicalized = canonicalize_edge_set(&mut edges);
        let faces = incident_faces_for_edges(txn.mesh(), &edges).map_err(|err| {
            op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                err,
            )
        })?;

        txn.delete_faces(&faces, params.policy)
            .map_err(|err| map_delete_faces_error(ctx, err))?;

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
        report.stats.elements_touched.faces =
            u64::try_from(faces.len()).expect("face count should fit u64");
        report.stats.elements_touched.half_edges =
            u64::try_from(edges.len()).expect("edge count should fit u64");
        report.stats.elements_deleted.faces = report.stats.elements_touched.faces;

        Ok((report, DeleteEdgesOutput { edges, faces }))
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

/// `edit.delete.faces` operator.
///
/// # Example
/// ```rust
/// use cambium::{DeleteFaces, DeleteFacesParams, OperatorRunner};
/// use exedra::{BuildParams, DeletePolicy, Mesh};
///
/// let mut mesh = Mesh::from_indexed_triangles(
///     &[
///         [0.0, 0.0, 0.0],
///         [1.0, 0.0, 0.0],
///         [1.0, 1.0, 0.0],
///         [0.0, 1.0, 0.0],
///     ],
///     &[[0, 1, 2], [0, 2, 3]],
///     &BuildParams::default(),
/// )
/// .expect("quad build should succeed");
/// let faces = mesh.faces().collect::<Vec<_>>();
///
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(
///         &mesh,
///         &DeleteFaces,
///         &DeleteFacesParams {
///             faces: vec![faces[0]],
///             policy: DeletePolicy::KeepIsolated,
///         },
///     )
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &DeleteFaces, &plan)
///     .expect("delete faces should succeed");
/// assert_eq!(result.output.faces, vec![faces[0]]);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct DeleteFaces;

impl EditOperator for DeleteFaces {
    type Params = DeleteFacesParams;
    type Plan = DeleteFacesPlan;
    type Output = DeleteFacesOutput;

    fn name(&self) -> &'static str {
        "edit.delete.faces"
    }

    fn compile(
        &self,
        mesh: &exedra::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        let mut faces = params.faces.clone();
        let canonicalized = canonicalize_face_set(&mut faces);
        for &face in &faces {
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
        }
        Ok(DeleteFacesPlan {
            faces,
            policy: params.policy,
            canonicalized,
        })
    }

    fn apply_plan(
        &self,
        txn: &mut exedra::EditSession<'_>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        txn.delete_faces(&plan.faces, plan.policy)
            .map_err(|err| map_delete_faces_error(ctx, err))?;

        let mut report = OpReport::new(
            self.name(),
            Artifacts::new(
                ctx.policy.limits.max_artifact_items,
                ctx.policy.limits.max_artifact_bytes,
            ),
        );
        if plan.canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }
        report.stats.counters.faces_processed =
            u64::try_from(plan.faces.len()).expect("face count should fit u64");
        report.stats.elements_touched.faces = report.stats.counters.faces_processed;
        report.stats.elements_deleted.faces = report.stats.counters.faces_processed;
        Ok((
            report,
            DeleteFacesOutput {
                faces: plan.faces.clone(),
            },
        ))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_face_set(&plan.faces);
        hasher.write_u8(match plan.policy {
            DeletePolicy::KeepIsolated => 0,
            DeletePolicy::CleanupIsolated => 1,
        });
        hasher.write_u8(u8::from(plan.canonicalized));
        hasher.finish()
    }
}

/// `edit.delete.vertices` operator.
///
/// # Example
/// ```rust
/// use cambium::{DeleteVertices, DeleteVerticesParams, OperatorRunner};
/// use exedra::Mesh;
///
/// let mut mesh = Mesh::new();
/// let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
/// let _v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
///
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(
///         &mesh,
///         &DeleteVertices,
///         &DeleteVerticesParams {
///             vertices: vec![v0],
///         },
///     )
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &DeleteVertices, &plan)
///     .expect("delete vertices should succeed");
/// assert_eq!(result.output.vertices, vec![v0]);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct DeleteVertices;

impl EditOperator for DeleteVertices {
    type Params = DeleteVerticesParams;
    type Plan = DeleteVerticesParams;
    type Output = DeleteVerticesOutput;

    fn name(&self) -> &'static str {
        "edit.delete.vertices"
    }

    fn apply(
        &self,
        txn: &mut exedra::EditSession<'_>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut vertices = params.vertices.clone();
        let canonicalized = canonicalize_vertex_set(&mut vertices);
        txn.delete_vertices(&vertices)
            .map_err(|err| map_delete_vertices_error(ctx, err))?;

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
        report.stats.elements_touched.vertices =
            u64::try_from(vertices.len()).expect("vertex count should fit u64");
        report.stats.elements_deleted.vertices = report.stats.elements_touched.vertices;
        Ok((report, DeleteVerticesOutput { vertices }))
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

fn incident_faces_for_edges(mesh: &exedra::Mesh, edges: &[HalfEdgeId]) -> Result<FaceSet, String> {
    let mut faces = FaceSet::new();
    for &edge in edges {
        let twin = mesh.twin(edge).ok_or_else(|| {
            format!(
                "edge selection contains stale half-edge id: {}",
                edge.index()
            )
        })?;
        if core::cmp::min(edge, twin) != edge {
            return Err("edge selection must use canonical undirected ids".into());
        }
        let face = mesh.face(edge).ok_or_else(|| {
            format!(
                "edge selection contains stale half-edge id: {}",
                edge.index()
            )
        })?;
        let twin_face = mesh.face(twin).ok_or_else(|| {
            format!(
                "edge selection contains stale half-edge id: {}",
                edge.index()
            )
        })?;
        if face != FaceId::OUTSIDE {
            faces.push(face);
        }
        if twin_face != FaceId::OUTSIDE {
            faces.push(twin_face);
        }
    }
    faces.sort_unstable();
    faces.dedup();
    Ok(faces)
}

fn map_delete_faces_error(ctx: &OpContext, err: DeleteFacesError) -> OpError {
    let (kind, code, message) = match err {
        DeleteFacesError::NonCanonicalFaceSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("face set must be sorted and deduplicated"),
        ),
        DeleteFacesError::OutsideFaceNotAllowed => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("face selection cannot contain FaceId::OUTSIDE"),
        ),
        DeleteFacesError::FaceNotLive { face } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("face selection contains stale face id: {face}"),
        ),
        DeleteFacesError::BoundaryContinuationAmbiguous { .. } => (
            OpErrorKind::InvalidMesh,
            DiagCode::NonManifoldInput,
            format!("delete_faces preflight failed: {err}"),
        ),
    };
    op_error(ctx, kind, code, message)
}

fn map_delete_vertices_error(ctx: &OpContext, err: DeleteVerticesError) -> OpError {
    let (kind, code, message) = match err {
        DeleteVerticesError::NonCanonicalVertexSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("vertex set must be sorted and deduplicated"),
        ),
        DeleteVerticesError::VertexNotLive { vertex } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("vertex selection contains stale vertex id: {vertex}"),
        ),
        DeleteVerticesError::VertexNotIsolated { vertex } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("vertex is not isolated: {vertex}"),
        ),
    };
    op_error(ctx, kind, code, message)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra::{BuildParams, FaceId, HalfEdgeId, Id};

    use super::{
        DeleteEdges, DeleteEdgesOutput, DeleteEdgesParams, DeleteFaces, DeleteFacesOutput,
        DeleteFacesParams, DeleteVertices, DeleteVerticesOutput, DeleteVerticesParams,
    };
    use crate::{OpErrorKind, OperatorRunner, mesh_signature, test_support::commit};

    fn two_tri_strip_mesh() -> exedra::Mesh {
        exedra::Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("strip build should succeed")
    }

    fn canonical_interior_edge(mesh: &exedra::Mesh) -> HalfEdgeId {
        mesh.faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                let Some(twin) = mesh.twin(half_edge) else {
                    return false;
                };
                if core::cmp::min(half_edge, twin) != half_edge {
                    return false;
                }
                let Some(face) = mesh.face(half_edge) else {
                    return false;
                };
                let Some(twin_face) = mesh.face(twin) else {
                    return false;
                };
                face != FaceId::OUTSIDE && twin_face != FaceId::OUTSIDE
            })
            .expect("strip should have one canonical interior edge")
    }

    #[test]
    fn delete_edges_applies_and_returns_typed_output() {
        let mut mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edge(&mesh);
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DeleteEdges,
            &DeleteEdgesParams {
                edges: vec![edge],
                policy: exedra::DeletePolicy::CleanupIsolated,
            },
        )
        .expect("delete edges should succeed");
        assert_eq!(
            result.output,
            DeleteEdgesOutput {
                edges: vec![edge],
                faces: result.output.faces.clone(),
            }
        );
        assert_eq!(result.output.faces.len(), 2);
        assert_eq!(mesh.faces().count(), 0);
        assert_eq!(result.report.stats.elements_deleted.faces, 2);
    }

    #[test]
    fn delete_edges_rejects_stale_edge() {
        let mut mesh = two_tri_strip_mesh();
        let stale = HalfEdgeId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &DeleteEdges,
            &DeleteEdgesParams {
                edges: vec![stale],
                policy: exedra::DeletePolicy::CleanupIsolated,
            },
        )
        .expect_err("stale edge should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn delete_faces_applies_and_returns_typed_output() {
        let mut mesh = exedra::Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("quad build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: vec![faces[1], faces[0]],
                policy: exedra::DeletePolicy::KeepIsolated,
            },
        )
        .expect("delete faces should succeed");
        assert_eq!(
            result.output,
            DeleteFacesOutput {
                faces: vec![faces[0], faces[1]],
            }
        );
        assert_eq!(result.report.stats.counters.selections_canonicalized, 1);
        assert_eq!(mesh.faces().count(), 0);
    }

    #[test]
    fn delete_faces_rejects_outside_face() {
        let mut mesh = exedra::Mesh::new();
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: vec![FaceId::OUTSIDE],
                policy: exedra::DeletePolicy::CleanupIsolated,
            },
        )
        .expect_err("outside face should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn delete_faces_compile_is_deterministic_for_identical_mesh_state() {
        let mesh = exedra::Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [0, 2, 3]],
            &BuildParams::default(),
        )
        .expect("quad build should succeed");
        let faces = mesh.faces().collect::<Vec<_>>();
        let params = DeleteFacesParams {
            faces: vec![faces[1], faces[0], faces[1]],
            policy: exedra::DeletePolicy::KeepIsolated,
        };
        let signature = mesh_signature(&mesh);

        let mut runner = OperatorRunner::new();
        let plan_a = runner
            .compile(&mesh, &DeleteFaces, &params)
            .expect("compile should succeed");
        let plan_b = runner
            .compile(&mesh, &DeleteFaces, &params)
            .expect("compile should succeed");
        assert_eq!(signature, mesh_signature(&mesh));
        assert_eq!(plan_a.fingerprint, plan_b.fingerprint);
        assert_eq!(plan_a.payload, plan_b.payload);
        assert_eq!(plan_a.payload.faces, vec![faces[0], faces[1]]);
    }

    #[test]
    fn delete_vertices_applies_and_returns_typed_output() {
        let mut mesh = exedra::Mesh::new();
        let v0 = mesh.add_vertex([0.0, 0.0, 0.0]);
        let _v1 = mesh.add_vertex([1.0, 0.0, 0.0]);
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DeleteVertices,
            &DeleteVerticesParams { vertices: vec![v0] },
        )
        .expect("delete vertices should succeed");
        assert_eq!(result.output, DeleteVerticesOutput { vertices: vec![v0] });
        assert_eq!(result.report.stats.elements_deleted.vertices, 1);
        assert_eq!(mesh.vertices().count(), 1);
    }

    #[test]
    fn delete_vertices_rejects_non_isolated_vertex() {
        let mut mesh = exedra::Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let vertex = mesh.vertices().next().expect("vertex should exist");
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &DeleteVertices,
            &DeleteVerticesParams {
                vertices: vec![vertex],
            },
        )
        .expect_err("non-isolated vertex should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }
}
