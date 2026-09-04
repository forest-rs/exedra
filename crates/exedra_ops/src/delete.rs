// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Face/edge deletion and dissolve edit operators.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use exedra_mesh::op::{
    DeleteFacesError, DeleteVerticesError, DissolveEdgesError, DissolveVerticesError,
};
use exedra_mesh::{DeletePolicy, FaceId, HalfEdgeId, op};

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

/// Parameters for [`DissolveEdges`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DissolveEdgesParams {
    /// Canonical undirected edge selection.
    pub edges: EdgeSet,
}

/// Typed output from [`DissolveEdges`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DissolveEdgesOutput {
    /// Canonical edge selection that was applied.
    pub edges: EdgeSet,
    /// Canonical merged face set produced by the dissolve.
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

/// Parameters for [`DissolveVertices`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DissolveVerticesParams {
    /// Canonical vertex selection to dissolve.
    pub vertices: VertexSet,
}

/// Typed output from [`DissolveVertices`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DissolveVerticesOutput {
    /// Canonical vertex selection that was applied.
    pub vertices: VertexSet,
    /// Canonical rebuilt face set produced by the dissolve.
    pub faces: FaceSet,
}

/// `edit.delete.edges` operator.
///
/// # Example
/// ```rust
/// use exedra_ops::{DeleteEdges, DeleteEdgesParams, OperatorRunner};
/// use exedra_mesh::{BuildParams, DeletePolicy, Mesh};
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
///             && mesh.face(half_edge) != Some(exedra_mesh::FaceId::OUTSIDE)
///             && mesh.face(twin) != Some(exedra_mesh::FaceId::OUTSIDE)
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

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
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

        op::delete_faces(txn, &faces, params.policy)
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

/// `edit.dissolve.edges` operator.
///
/// # Example
/// ```rust
/// use exedra_ops::{DissolveEdges, DissolveEdgesParams, OperatorRunner};
/// use exedra_mesh::{BuildParams, FaceId, Mesh};
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
///             && mesh.face(half_edge) != Some(FaceId::OUTSIDE)
///             && mesh.face(twin) != Some(FaceId::OUTSIDE)
///     })
///     .expect("interior edge should exist");
///
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(&mesh, &DissolveEdges, &DissolveEdgesParams { edges: vec![edge] })
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &DissolveEdges, &plan)
///     .expect("dissolve edges should succeed");
/// assert_eq!(result.output.edges, vec![edge]);
/// assert_eq!(result.output.faces.len(), 1);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct DissolveEdges;

impl EditOperator for DissolveEdges {
    type Params = DissolveEdgesParams;
    type Plan = DissolveEdgesParams;
    type Output = DissolveEdgesOutput;

    fn name(&self) -> &'static str {
        "edit.dissolve.edges"
    }

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut edges = params.edges.clone();
        let canonicalized = canonicalize_edge_set(&mut edges);
        let faces =
            op::dissolve_edges(txn, &edges).map_err(|err| map_dissolve_edges_error(ctx, err))?;

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
        report.stats.elements_touched.half_edges =
            u64::try_from(edges.len()).expect("edge count should fit u64");
        report.stats.elements_touched.faces =
            u64::try_from(edges.len() * 2).expect("face count should fit u64");
        report.stats.elements_deleted.faces = report.stats.elements_touched.faces;
        report.stats.elements_created.faces =
            u64::try_from(faces.len()).expect("face count should fit u64");
        report.stats.counters.faces_processed = report.stats.elements_touched.faces;

        Ok((report, DissolveEdgesOutput { edges, faces }))
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        let mut edges = params.edges.clone();
        let _ = canonicalize_edge_set(&mut edges);
        validate_dissolve_edges_selection(mesh, &edges)
            .map_err(|err| map_dissolve_edges_error(ctx, err))?;
        Ok(DissolveEdgesParams { edges })
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

/// `edit.delete.faces` operator.
///
/// # Example
/// ```rust
/// use exedra_ops::{DeleteFaces, DeleteFacesParams, OperatorRunner};
/// use exedra_mesh::{BuildParams, DeletePolicy, Mesh};
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
        mesh: &exedra_mesh::Mesh,
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

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        plan: &Self::Plan,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        op::delete_faces(txn, &plan.faces, plan.policy)
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
/// use exedra_ops::{DeleteVertices, DeleteVerticesParams, OperatorRunner};
/// use exedra_mesh::{op, Mesh};
///
/// let mut mesh = Mesh::new();
/// let v0 = {
///     let mut txn = mesh.edit();
///     let v0 = op::add_vertex(&mut txn, [0.0, 0.0, 0.0]);
///     let _v1 = op::add_vertex(&mut txn, [1.0, 0.0, 0.0]);
///     let _: () = txn.finish();
///     v0
/// };
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

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut vertices = params.vertices.clone();
        let canonicalized = canonicalize_vertex_set(&mut vertices);
        op::delete_vertices(txn, &vertices).map_err(|err| map_delete_vertices_error(ctx, err))?;

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

/// `edit.dissolve.vertices` operator.
///
/// # Example
/// ```rust
/// use exedra_ops::{DissolveVertices, DissolveVerticesParams, OperatorRunner};
/// use exedra_mesh::{BuildParams, Mesh, PropagatePolicy, op};
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
///             && mesh.face(half_edge) != Some(exedra_mesh::FaceId::OUTSIDE)
///             && mesh.face(twin) != Some(exedra_mesh::FaceId::OUTSIDE)
///     })
///     .expect("interior edge should exist");
/// let inserted = {
///     let mut edit = mesh.edit();
///     let vertex = op::split_edge(&mut edit, edge, &PropagatePolicy::default())
///         .expect("split should succeed");
///     let _: () = edit.finish();
///     vertex
/// };
///
/// let mut runner = OperatorRunner::new();
/// let plan = runner
///     .compile(
///         &mesh,
///         &DissolveVertices,
///         &DissolveVerticesParams {
///             vertices: vec![inserted],
///         },
///     )
///     .expect("compile should succeed");
/// let result = runner
///     .apply_in_place(&mut mesh, &DissolveVertices, &plan)
///     .expect("dissolve vertices should succeed");
/// assert_eq!(result.output.vertices, vec![inserted]);
/// assert_eq!(result.output.faces.len(), 2);
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct DissolveVertices;

impl EditOperator for DissolveVertices {
    type Params = DissolveVerticesParams;
    type Plan = DissolveVerticesParams;
    type Output = DissolveVerticesOutput;

    fn name(&self) -> &'static str {
        "edit.dissolve.vertices"
    }

    fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<(OpReport, Self::Output), OpError> {
        let mut vertices = params.vertices.clone();
        let canonicalized = canonicalize_vertex_set(&mut vertices);
        let mut faces = op::dissolve_vertices(txn, &vertices)
            .map_err(|err| map_dissolve_vertices_error(ctx, err))?;
        canonicalize_face_set(&mut faces);

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
        report.stats.elements_touched.faces =
            u64::try_from(faces.len()).expect("face count should fit u64");
        report.stats.elements_deleted.vertices = report.stats.elements_touched.vertices;
        report.stats.elements_deleted.faces =
            u64::try_from(faces.len()).expect("face count should fit u64");
        report.stats.elements_created.faces =
            u64::try_from(faces.len()).expect("face count should fit u64");
        report.stats.counters.faces_processed =
            u64::try_from(faces.len()).expect("face count should fit u64");

        Ok((report, DissolveVerticesOutput { vertices, faces }))
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        let mut vertices = params.vertices.clone();
        let _ = canonicalize_vertex_set(&mut vertices);
        validate_dissolve_vertices_selection(mesh, &vertices)
            .map_err(|err| map_dissolve_vertices_error(ctx, err))?;
        Ok(DissolveVerticesParams { vertices })
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

fn incident_faces_for_edges(
    mesh: &exedra_mesh::Mesh,
    edges: &[HalfEdgeId],
) -> Result<FaceSet, String> {
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

fn map_dissolve_vertices_error(ctx: &OpContext, err: DissolveVerticesError) -> OpError {
    let (kind, code, message) = match err {
        DissolveVerticesError::NonCanonicalVertexSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("vertex set must be sorted and deduplicated"),
        ),
        DissolveVerticesError::VertexNotLive { vertex } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("vertex selection contains stale vertex id: {vertex}"),
        ),
        DissolveVerticesError::BoundaryVertexNotDissolvable { vertex } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("boundary vertex cannot be dissolved: {vertex}"),
        ),
        DissolveVerticesError::UnsupportedVertexDegree { vertex, degree } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!(
                "vertex dissolve requires an interior valence-2 vertex: {vertex} has degree {degree}"
            ),
        ),
        DissolveVerticesError::UnsupportedVertexTopology { vertex } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("vertex dissolve does not support this vertex star: {vertex}"),
        ),
        DissolveVerticesError::IncidentFaceTooSmall { .. } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("vertex dissolve failed: {err}"),
        ),
        DissolveVerticesError::OverlappingVertexSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("vertex set contains overlapping dissolve regions"),
        ),
        DissolveVerticesError::FaceDeleteFailed(inner) => {
            return map_delete_faces_error(ctx, inner);
        }
        DissolveVerticesError::VertexDeleteFailed(inner) => {
            return map_delete_vertices_error(ctx, inner);
        }
        DissolveVerticesError::FaceCreateFailed(_) => (
            OpErrorKind::InvalidMesh,
            DiagCode::NonManifoldInput,
            format!("vertex dissolve failed: {err}"),
        ),
    };
    op_error(ctx, kind, code, message)
}

fn validate_dissolve_vertices_selection(
    mesh: &exedra_mesh::Mesh,
    vertices: &[exedra_mesh::VertexId],
) -> Result<(), DissolveVerticesError> {
    let mut touched_faces = BTreeSet::<FaceId>::new();
    for &vertex in vertices {
        if mesh.vertex_position(vertex).is_none() {
            return Err(DissolveVerticesError::VertexNotLive {
                vertex: vertex.index(),
            });
        }
        let star = mesh.vertex_star(vertex).collect::<Vec<_>>();
        if star.len() != 2 {
            return Err(DissolveVerticesError::UnsupportedVertexDegree {
                vertex: vertex.index(),
                degree: star.len(),
            });
        }
        let mut faces = FaceSet::new();
        for half_edge in star {
            let face = mesh
                .face(half_edge)
                .ok_or(DissolveVerticesError::VertexNotLive {
                    vertex: vertex.index(),
                })?;
            if face == FaceId::OUTSIDE {
                return Err(DissolveVerticesError::BoundaryVertexNotDissolvable {
                    vertex: vertex.index(),
                });
            }
            if !faces.contains(&face) {
                let degree = mesh.face_loop(face).count();
                if degree < 4 {
                    return Err(DissolveVerticesError::IncidentFaceTooSmall {
                        vertex: vertex.index(),
                        face: face.index(),
                        degree,
                    });
                }
                faces.push(face);
            }
        }
        canonicalize_face_set(&mut faces);
        if faces.len() != 2 {
            return Err(DissolveVerticesError::UnsupportedVertexTopology {
                vertex: vertex.index(),
            });
        }
        for face in faces {
            if !touched_faces.insert(face) {
                return Err(DissolveVerticesError::OverlappingVertexSet);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn two_disjoint_strip_mesh() -> exedra_mesh::Mesh {
    exedra_mesh::Mesh::from_indexed_triangles(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 1.0, 0.0],
            [3.0, 1.0, 0.0],
        ],
        &[[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        &exedra_mesh::BuildParams::default(),
    )
    .expect("two disjoint strips should build")
}

#[cfg(test)]
fn split_disjoint_strip_vertices() -> (exedra_mesh::Mesh, VertexSet) {
    let mut mesh = two_disjoint_strip_mesh();
    let mut edges = mesh
        .faces()
        .flat_map(|face| mesh.face_loop(face))
        .filter(|&half_edge| {
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
        .collect::<Vec<_>>();
    edges.sort_unstable();
    let vertices = {
        let mut edit = mesh.edit();
        let mut inserted = edges
            .into_iter()
            .map(|edge| {
                op::split_edge(&mut edit, edge, &exedra_mesh::PropagatePolicy::default())
                    .expect("split should succeed")
            })
            .collect::<Vec<_>>();
        let _: () = edit.finish();
        inserted.sort_unstable();
        inserted
    };
    (mesh, vertices)
}

fn map_dissolve_edges_error(ctx: &OpContext, err: DissolveEdgesError) -> OpError {
    let (kind, code, message) = match err {
        DissolveEdgesError::NonCanonicalEdgeSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("edge set must be sorted and deduplicated"),
        ),
        DissolveEdgesError::HalfEdgeNotLive { half_edge } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("edge selection contains stale half-edge id: {half_edge}"),
        ),
        DissolveEdgesError::BoundaryEdgeNotDissolvable { half_edge } => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("boundary edge cannot be dissolved: {half_edge}"),
        ),
        DissolveEdgesError::OverlappingEdgeSet => (
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            String::from("edge set contains overlapping dissolve regions"),
        ),
        DissolveEdgesError::MergedLoopTooShort { .. }
        | DissolveEdgesError::MergedLoopRepeatedVertex { .. } => (
            OpErrorKind::InvalidMesh,
            DiagCode::NonManifoldInput,
            format!("edge dissolve failed: {err}"),
        ),
        DissolveEdgesError::FaceDeleteFailed(inner) => {
            return map_delete_faces_error(ctx, inner);
        }
        DissolveEdgesError::FaceCreateFailed(_) => (
            OpErrorKind::InvalidMesh,
            DiagCode::NonManifoldInput,
            format!("edge dissolve failed: {err}"),
        ),
    };
    op_error(ctx, kind, code, message)
}

fn validate_dissolve_edges_selection(
    mesh: &exedra_mesh::Mesh,
    edges: &[HalfEdgeId],
) -> Result<(), DissolveEdgesError> {
    let mut touched_faces = BTreeSet::<FaceId>::new();
    for &edge in edges {
        let twin = mesh.twin(edge).ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: edge.index(),
        })?;
        if core::cmp::min(edge, twin) != edge {
            return Err(DissolveEdgesError::NonCanonicalEdgeSet);
        }
        let face = mesh.face(edge).ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: edge.index(),
        })?;
        let twin_face = mesh.face(twin).ok_or(DissolveEdgesError::HalfEdgeNotLive {
            half_edge: edge.index(),
        })?;
        if face == FaceId::OUTSIDE || twin_face == FaceId::OUTSIDE {
            return Err(DissolveEdgesError::BoundaryEdgeNotDissolvable {
                half_edge: edge.index(),
            });
        }
        if !touched_faces.insert(face) || !touched_faces.insert(twin_face) {
            return Err(DissolveEdgesError::OverlappingEdgeSet);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra_mesh::{BuildParams, FaceId, HalfEdgeId, Id, PropagatePolicy, VertexId};

    use super::{
        DeleteEdges, DeleteEdgesOutput, DeleteEdgesParams, DeleteFaces, DeleteFacesOutput,
        DeleteFacesParams, DeleteVertices, DeleteVerticesOutput, DeleteVerticesParams,
        DissolveEdges, DissolveEdgesOutput, DissolveEdgesParams, DissolveVertices,
        DissolveVerticesOutput, DissolveVerticesParams,
    };
    use crate::{OpErrorKind, OperatorRunner, mesh_signature, test_support::commit};

    fn two_tri_strip_mesh() -> exedra_mesh::Mesh {
        exedra_mesh::Mesh::from_indexed_triangles(
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

    fn canonical_interior_edge(mesh: &exedra_mesh::Mesh) -> HalfEdgeId {
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

    fn split_strip_vertex() -> (exedra_mesh::Mesh, VertexId) {
        let mut mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edge(&mesh);
        let inserted = {
            let mut edit = mesh.edit();
            let inserted =
                exedra_mesh::op::split_edge(&mut edit, edge, &PropagatePolicy::default())
                    .expect("split should succeed");
            let _: () = edit.finish();
            inserted
        };
        (mesh, inserted)
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
                policy: exedra_mesh::DeletePolicy::CleanupIsolated,
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
        assert_eq!(
            result.report.stats.counters.deleted_vertices,
            u64::try_from(result.change_set.deleted_vertices.len()).expect("count should fit u64")
        );
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
                policy: exedra_mesh::DeletePolicy::CleanupIsolated,
            },
        )
        .expect_err("stale edge should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn dissolve_edges_applies_and_returns_typed_output() {
        let mut mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edge(&mesh);
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DissolveEdges,
            &DissolveEdgesParams { edges: vec![edge] },
        )
        .expect("dissolve edges should succeed");
        assert_eq!(
            result.output,
            DissolveEdgesOutput {
                edges: vec![edge],
                faces: result.output.faces.clone(),
            }
        );
        assert_eq!(result.output.faces.len(), 1);
        assert_eq!(mesh.faces().count(), 1);
        assert_eq!(result.report.stats.elements_deleted.faces, 2);
        assert_eq!(result.report.stats.elements_created.faces, 1);
    }

    #[test]
    fn dissolve_edges_rejects_boundary_edge() {
        let mut mesh = two_tri_strip_mesh();
        let edge = mesh
            .face_loop(mesh.faces().next().expect("face should exist"))
            .find(|&half_edge| {
                let twin = mesh
                    .twin(half_edge)
                    .expect("live half-edge should have twin");
                mesh.face(twin) == Some(FaceId::OUTSIDE)
            })
            .expect("boundary edge should exist");
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &DissolveEdges,
            &DissolveEdgesParams { edges: vec![edge] },
        )
        .expect_err("boundary edge should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn dissolve_edges_compile_canonicalizes_and_rejects_stale_edge() {
        let mesh = two_tri_strip_mesh();
        let edge = canonical_interior_edge(&mesh);
        let stale = HalfEdgeId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();

        let plan = runner
            .compile(
                &mesh,
                &DissolveEdges,
                &DissolveEdgesParams {
                    edges: vec![edge, edge],
                },
            )
            .expect("compile should canonicalize duplicate edge selection");
        assert_eq!(plan.payload.edges, vec![edge]);

        let err = runner
            .compile(
                &mesh,
                &DissolveEdges,
                &DissolveEdgesParams { edges: vec![stale] },
            )
            .expect_err("stale edge should fail at compile time");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn dissolve_vertices_applies_and_returns_typed_output() {
        let (mut mesh, inserted) = split_strip_vertex();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DissolveVertices,
            &DissolveVerticesParams {
                vertices: vec![inserted],
            },
        )
        .expect("dissolve vertices should succeed");
        assert_eq!(
            result.output,
            DissolveVerticesOutput {
                vertices: vec![inserted],
                faces: result.output.faces.clone(),
            }
        );
        assert_eq!(result.output.faces.len(), 2);
        assert_eq!(mesh.vertices().count(), 4);
        assert_eq!(result.report.stats.elements_deleted.vertices, 1);
        assert_eq!(result.report.stats.elements_created.faces, 2);
    }

    #[test]
    fn dissolve_vertices_compile_rejects_boundary_vertex() {
        let mesh = exedra_mesh::Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad build should succeed");
        let vertex = mesh.vertices().next().expect("vertex should exist");
        let mut runner = OperatorRunner::new();
        let err = runner
            .compile(
                &mesh,
                &DissolveVertices,
                &DissolveVerticesParams {
                    vertices: vec![vertex],
                },
            )
            .expect_err("boundary vertex should fail at compile time");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn dissolve_vertices_compile_canonicalizes_and_rejects_stale_vertex() {
        let (mesh, inserted) = split_strip_vertex();
        let stale = VertexId::from(Id::new(999, NonZeroU32::MIN));
        let mut runner = OperatorRunner::new();

        let plan = runner
            .compile(
                &mesh,
                &DissolveVertices,
                &DissolveVerticesParams {
                    vertices: vec![inserted, inserted],
                },
            )
            .expect("compile should canonicalize duplicate vertex selection");
        assert_eq!(plan.payload.vertices, vec![inserted]);

        let err = runner
            .compile(
                &mesh,
                &DissolveVertices,
                &DissolveVerticesParams {
                    vertices: vec![stale],
                },
            )
            .expect_err("stale vertex should fail at compile time");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn dissolve_vertices_applies_disjoint_batch_and_reports_deleted_faces() {
        let (mut mesh, inserted) = super::split_disjoint_strip_vertices();
        let mut runner = OperatorRunner::new();
        let result = commit(
            &mut runner,
            &mut mesh,
            &DissolveVertices,
            &DissolveVerticesParams {
                vertices: inserted.clone(),
            },
        )
        .expect("disjoint batch dissolve should succeed");
        assert_eq!(
            result.output,
            DissolveVerticesOutput {
                vertices: inserted,
                faces: result.output.faces.clone(),
            }
        );
        assert_eq!(result.output.faces.len(), 4);
        assert_eq!(mesh.faces().count(), 4);
        assert_eq!(mesh.vertices().count(), 8);
        assert_eq!(result.report.stats.elements_deleted.vertices, 2);
        assert_eq!(result.report.stats.elements_deleted.faces, 4);
        assert_eq!(result.report.stats.elements_created.faces, 4);
    }

    #[test]
    fn delete_faces_applies_and_returns_typed_output() {
        let mut mesh = exedra_mesh::Mesh::from_indexed_triangles(
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
                policy: exedra_mesh::DeletePolicy::KeepIsolated,
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
        assert_eq!(
            result.report.stats.counters.deleted_vertices,
            u64::try_from(result.change_set.deleted_vertices.len()).expect("count should fit u64")
        );
    }

    #[test]
    fn delete_faces_rejects_outside_face() {
        let mut mesh = exedra_mesh::Mesh::new();
        let mut runner = OperatorRunner::new();
        let err = commit(
            &mut runner,
            &mut mesh,
            &DeleteFaces,
            &DeleteFacesParams {
                faces: vec![FaceId::OUTSIDE],
                policy: exedra_mesh::DeletePolicy::CleanupIsolated,
            },
        )
        .expect_err("outside face should fail");
        assert_eq!(err.kind, OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn delete_faces_compile_is_deterministic_for_identical_mesh_state() {
        let mesh = exedra_mesh::Mesh::from_indexed_triangles(
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
            policy: exedra_mesh::DeletePolicy::KeepIsolated,
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
        let mut mesh = exedra_mesh::Mesh::new();
        let v0 = {
            let mut txn = mesh.edit();
            let v0 = exedra_mesh::op::add_vertex(&mut txn, [0.0, 0.0, 0.0]);
            let _v1 = exedra_mesh::op::add_vertex(&mut txn, [1.0, 0.0, 0.0]);
            let _: () = txn.finish();
            v0
        };
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
        assert_eq!(result.report.stats.counters.deleted_vertices, 1);
        assert_eq!(mesh.vertices().count(), 1);
    }

    #[test]
    fn delete_vertices_rejects_non_isolated_vertex() {
        let mut mesh = exedra_mesh::Mesh::from_indexed_triangles(
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
