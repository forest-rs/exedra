// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Boundary-loop bridge operator.

use alloc::format;
use alloc::vec::Vec;

use exedra_mesh::{FaceId, HalfEdgeId, VertexId, op};

use crate::op_common::op_error;
use crate::patch::connect::{FrameOrientationState, add_frame_face_with_orientation};
use crate::selection::{EdgeSet, canonicalize_edge_set};
use crate::{Artifacts, DiagCode, EditOperator, OpContext, OpError, OpErrorKind, OpReport};

/// Parameters for [`BridgeBoundaryLoops`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BridgeBoundaryLoopsParams {
    /// First canonical boundary loop.
    pub loop_a: EdgeSet,
    /// Second canonical boundary loop.
    pub loop_b: EdgeSet,
}

/// Deterministic compiled plan payload for [`BridgeBoundaryLoops`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BridgeBoundaryLoopsPlan {
    loop_a: EdgeSet,
    loop_b: EdgeSet,
    aligned_a_vertices: Vec<VertexId>,
    aligned_b_vertices: Vec<VertexId>,
    selections_canonicalized: bool,
}

/// Typed output from [`BridgeBoundaryLoops`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BridgeBoundaryLoopsOutput {
    /// Created quad-strip faces in traversal order.
    pub bridge_faces: Vec<FaceId>,
}

/// `edit.bridge.loops` operator.
///
/// v0.1 scope:
/// - both inputs must be canonical boundary loops,
/// - both loops must have identical edge counts,
/// - loop pairing is deterministic and chosen by minimum geometric mismatch,
/// - generated bridge-strip boundaries are marked sharp by default.
#[derive(Copy, Clone, Debug, Default)]
pub struct BridgeBoundaryLoops;

impl EditOperator for BridgeBoundaryLoops {
    type Params = BridgeBoundaryLoopsParams;
    type Plan = BridgeBoundaryLoopsPlan;
    type Output = BridgeBoundaryLoopsOutput;

    fn name(&self) -> &'static str {
        "edit.bridge.loops"
    }

    fn compile(
        &self,
        mesh: &exedra_mesh::Mesh,
        params: &Self::Params,
        ctx: &mut OpContext,
    ) -> Result<Self::Plan, OpError> {
        let mut loop_a = params.loop_a.clone();
        let mut loop_b = params.loop_b.clone();
        let canonicalized_a = canonicalize_edge_set(&mut loop_a);
        let canonicalized_b = canonicalize_edge_set(&mut loop_b);

        if loop_a.is_empty() || loop_b.is_empty() {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "edit.bridge.loops requires two non-empty boundary loops",
            ));
        }
        if loop_a.iter().any(|edge| loop_b.binary_search(edge).is_ok()) {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                "edit.bridge.loops requires disjoint boundary loops",
            ));
        }

        let ordered_a = ordered_boundary_loop(mesh, &loop_a, ctx, "loop_a")?;
        let ordered_b = ordered_boundary_loop(mesh, &loop_b, ctx, "loop_b")?;
        if ordered_a.vertices.len() != ordered_b.vertices.len() {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!(
                    "edit.bridge.loops requires equal loop lengths (got {} and {})",
                    ordered_a.vertices.len(),
                    ordered_b.vertices.len()
                ),
            ));
        }

        let aligned_b = align_loop_vertices(mesh, &ordered_a.vertices, &ordered_b.vertices)
            .ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::PreconditionFailed,
                    DiagCode::PreconditionFailed,
                    "edit.bridge.loops requires loops with live vertex positions",
                )
            })?;

        Ok(BridgeBoundaryLoopsPlan {
            loop_a,
            loop_b,
            aligned_a_vertices: ordered_a.vertices,
            aligned_b_vertices: aligned_b,
            selections_canonicalized: canonicalized_a || canonicalized_b,
        })
    }

    fn apply_plan<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
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
        if plan.selections_canonicalized {
            report.stats.counters.selections_canonicalized = 1;
        }

        let count = plan.aligned_a_vertices.len();
        let mut bridge_faces = Vec::with_capacity(count);
        let mut orientation = FrameOrientationState::default();
        for i in 0..count {
            let current = plan.aligned_a_vertices[i];
            let next = plan.aligned_a_vertices[(i + 1) % count];
            let current_inner = plan.aligned_b_vertices[i];
            let next_inner = plan.aligned_b_vertices[(i + 1) % count];
            let face = add_frame_face_with_orientation(
                txn,
                current,
                next,
                current_inner,
                next_inner,
                &mut orientation,
                ctx,
                "bridge",
            )?;
            // Semantic bridge boundaries are hard by default; columns stay smooth.
            let first_edge = txn.mesh().face_edge(face).ok_or_else(|| {
                op_error(
                    ctx,
                    OpErrorKind::InvalidMesh,
                    DiagCode::InternalInvariantViolation,
                    "bridge created face without an anchor edge",
                )
            })?;
            let _ = op::set_edge_sharpness(txn, first_edge, 1.0);
            if let Some(next_edge) = txn.mesh().next(first_edge) {
                let _ = op::set_edge_sharpness(txn, next_edge, 0.0);
                if let Some(next_next_edge) = txn.mesh().next(next_edge) {
                    let _ = op::set_edge_sharpness(txn, next_next_edge, 1.0);
                    if let Some(last_edge) = txn.mesh().next(next_next_edge) {
                        let _ = op::set_edge_sharpness(txn, last_edge, 0.0);
                    }
                }
            }
            bridge_faces.push(face);
        }

        report.stats.elements_touched.half_edges =
            u64::try_from(plan.loop_a.len() + plan.loop_b.len())
                .expect("edge count should fit u64");
        report.stats.elements_created.faces =
            u64::try_from(bridge_faces.len()).expect("face count should fit u64");
        Ok((report, BridgeBoundaryLoopsOutput { bridge_faces }))
    }

    fn plan_fingerprint(&self, plan: &Self::Plan) -> crate::PlanFingerprint {
        let mut hasher = crate::plan::PlanHasher::new();
        hasher.write_str(self.name());
        hasher.write_len(plan.loop_a.len());
        for edge in &plan.loop_a {
            hasher.write_u32(edge.index());
        }
        hasher.write_len(plan.loop_b.len());
        for edge in &plan.loop_b {
            hasher.write_u32(edge.index());
        }
        for vertex in &plan.aligned_a_vertices {
            hasher.write_u32(vertex.index());
        }
        for vertex in &plan.aligned_b_vertices {
            hasher.write_u32(vertex.index());
        }
        hasher.write_u8(u8::from(plan.selections_canonicalized));
        hasher.finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OrderedBoundaryLoop {
    vertices: Vec<VertexId>,
}

fn ordered_boundary_loop(
    mesh: &exedra_mesh::Mesh,
    canonical_edges: &[HalfEdgeId],
    ctx: &OpContext,
    label: &str,
) -> Result<OrderedBoundaryLoop, OpError> {
    let seed = canonical_edges[0];
    let start = orient_boundary_edge(mesh, seed).ok_or_else(|| {
        op_error(
            ctx,
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("edit.bridge.loops {label} contains a stale or non-boundary edge"),
        )
    })?;

    let mut vertices = Vec::with_capacity(canonical_edges.len());
    let mut seen = Vec::<HalfEdgeId>::with_capacity(canonical_edges.len());
    let mut current = start;
    loop {
        if seen.contains(&current) {
            if current == start {
                break;
            }
            return Err(op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                format!("edit.bridge.loops {label} revisited a non-start boundary half-edge"),
            ));
        }
        seen.push(current);
        let canonical = mesh.canonical_edge(current).ok_or_else(|| {
            op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                format!("edit.bridge.loops {label} encountered stale canonical edge"),
            )
        })?;
        if canonical_edges.binary_search(&canonical).is_err() {
            return Err(op_error(
                ctx,
                OpErrorKind::PreconditionFailed,
                DiagCode::PreconditionFailed,
                format!("edit.bridge.loops {label} is not exactly one boundary loop"),
            ));
        }
        vertices.push(mesh.from_vertex(current).ok_or_else(|| {
            op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                format!("edit.bridge.loops {label} encountered stale boundary vertex"),
            )
        })?);
        let next = mesh.next(current).ok_or_else(|| {
            op_error(
                ctx,
                OpErrorKind::InvalidMesh,
                DiagCode::InternalInvariantViolation,
                format!("edit.bridge.loops {label} encountered missing next pointer"),
            )
        })?;
        if next == start {
            break;
        }
        current = next;
    }

    if seen.len() != canonical_edges.len() {
        return Err(op_error(
            ctx,
            OpErrorKind::PreconditionFailed,
            DiagCode::PreconditionFailed,
            format!("edit.bridge.loops {label} is not exactly one closed boundary loop"),
        ));
    }

    Ok(OrderedBoundaryLoop { vertices })
}

fn orient_boundary_edge(mesh: &exedra_mesh::Mesh, edge: HalfEdgeId) -> Option<HalfEdgeId> {
    let twin = mesh.twin(edge)?;
    let edge_face = mesh.face(edge)?;
    let twin_face = mesh.face(twin)?;
    if edge_face == FaceId::OUTSIDE {
        Some(edge)
    } else if twin_face == FaceId::OUTSIDE {
        Some(twin)
    } else {
        None
    }
}

fn align_loop_vertices(
    mesh: &exedra_mesh::Mesh,
    loop_a: &[VertexId],
    loop_b: &[VertexId],
) -> Option<Vec<VertexId>> {
    let len = loop_a.len();
    if len != loop_b.len() || len == 0 {
        return None;
    }

    let mut best_score = None::<u64>;
    let mut best = None::<Vec<VertexId>>;
    for reverse in [false, true] {
        for rotation in 0..len {
            let mut candidate = Vec::with_capacity(len);
            for i in 0..len {
                let index = if reverse {
                    (rotation + len - i) % len
                } else {
                    (rotation + i) % len
                };
                candidate.push(loop_b[index]);
            }
            let score = candidate_alignment_score(mesh, loop_a, &candidate)?;
            let replace = match best_score {
                Some(current) => score < current,
                None => true,
            };
            if replace {
                best_score = Some(score);
                best = Some(candidate);
            }
        }
    }
    best
}

fn candidate_alignment_score(
    mesh: &exedra_mesh::Mesh,
    loop_a: &[VertexId],
    loop_b: &[VertexId],
) -> Option<u64> {
    let mut score = 0.0_f64;
    for (&a, &b) in loop_a.iter().zip(loop_b.iter()) {
        let pa = mesh.vertex_position(a)?;
        let pb = mesh.vertex_position(b)?;
        let dx = f64::from(pa[0] - pb[0]);
        let dy = f64::from(pa[1] - pb[1]);
        let dz = f64::from(pa[2] - pb[2]);
        score += dx * dx + dy * dy + dz * dz;
    }
    Some(score.to_bits())
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use exedra_mesh::Mesh;

    use super::{BridgeBoundaryLoops, BridgeBoundaryLoopsParams};
    use crate::{
        MeshEdit, OperatorRunner, SelectBoundaryEdgeLoop, SelectBoundaryEdgeLoopParams,
        test_support::commit,
    };

    fn two_parallel_quads() -> (Mesh, exedra_mesh::FaceId, exedra_mesh::FaceId) {
        let mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            &[&[0, 1, 2, 3], &[4, 7, 6, 5]],
        )
        .expect("parallel quads should build");
        let faces = mesh.faces().collect::<Vec<_>>();
        (mesh, faces[0], faces[1])
    }

    #[test]
    fn bridge_parallel_quads_creates_quad_strip() {
        let (mut mesh, face_a, face_b) = two_parallel_quads();
        let mut runner = OperatorRunner::new();
        let seed_a = mesh.face_edge(face_a).expect("face edge should exist");
        let seed_b = mesh.face_edge(face_b).expect("face edge should exist");
        let loop_a = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_a },
        )
        .expect("boundary select should succeed")
        .output;
        let loop_b = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_b },
        )
        .expect("boundary select should succeed")
        .output;

        let result = commit(
            &mut runner,
            &mut mesh,
            &BridgeBoundaryLoops,
            &BridgeBoundaryLoopsParams { loop_a, loop_b },
        )
        .expect("bridge should succeed");
        assert_eq!(result.output.bridge_faces.len(), 4);
        assert_eq!(mesh.faces().count(), 6);
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn bridge_rejects_unequal_length_loops() {
        let mut mesh = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
                [3.0, 0.0, 0.0],
                [4.0, 0.0, 0.0],
                [4.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3], &[4, 5, 6]],
        )
        .expect("mixed loops should build");
        let faces = mesh.faces().collect::<Vec<_>>();
        let mut runner = OperatorRunner::new();
        let seed_a = mesh.face_edge(faces[0]).expect("face edge should exist");
        let seed_b = mesh.face_edge(faces[1]).expect("face edge should exist");
        let loop_a = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_a },
        )
        .expect("boundary select should succeed")
        .output;
        let loop_b = commit(
            &mut runner,
            &mut mesh,
            &SelectBoundaryEdgeLoop,
            &SelectBoundaryEdgeLoopParams { seed_edge: seed_b },
        )
        .expect("boundary select should succeed")
        .output;

        let err = runner
            .compile(
                &mesh,
                &BridgeBoundaryLoops,
                &BridgeBoundaryLoopsParams { loop_a, loop_b },
            )
            .expect_err("unequal loops should fail");
        assert_eq!(err.kind, crate::OpErrorKind::PreconditionFailed);
    }

    #[test]
    fn mesh_edit_bridge_matches_direct_operator() {
        let (mut direct_mesh, face_a, face_b) = two_parallel_quads();
        let mut fluent_mesh = direct_mesh.clone();
        let seed_a = direct_mesh
            .face_edge(face_a)
            .expect("face edge should exist");
        let seed_b = direct_mesh
            .face_edge(face_b)
            .expect("face edge should exist");
        let loop_a = crate::select_boundary_edge_loop(&direct_mesh, seed_a)
            .expect("loop query should succeed")
            .edges;
        let loop_b = crate::select_boundary_edge_loop(&direct_mesh, seed_b)
            .expect("loop query should succeed")
            .edges;

        let mut direct_runner = OperatorRunner::new();
        let plan = direct_runner
            .compile(
                &direct_mesh,
                &BridgeBoundaryLoops,
                &BridgeBoundaryLoopsParams {
                    loop_a: loop_a.clone(),
                    loop_b: loop_b.clone(),
                },
            )
            .expect("bridge compile should succeed");
        let _ = direct_runner
            .apply_in_place(&mut direct_mesh, &BridgeBoundaryLoops, &plan)
            .expect("bridge apply should succeed");

        let flow = MeshEdit::new()
            .select(crate::Selection::from(loop_a))
            .bridge_boundary_loops(loop_b);
        let mut fluent_runner = OperatorRunner::new();
        let _ = flow
            .apply(&mut fluent_runner, &mut fluent_mesh)
            .expect("fluent bridge should succeed");

        assert_eq!(
            crate::mesh_signature(&direct_mesh),
            crate::mesh_signature(&fluent_mesh)
        );
    }
}
