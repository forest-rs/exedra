// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Direct boundary-loop bridging with inspectable geometric pairing.

use crate::patch::connect::{FrameOrientationState, add_frame_face_with_orientation};
use crate::patch::source::{CaptureError, FaceInput, capture_inputs};
use crate::selection::{EdgeSet, canonicalize_edge_set};
use alloc::vec::Vec;
use exedra_mesh::{FaceId, HalfEdgeId, Mesh, VertexId, op};

/// Identifies one of the two authored bridge loops.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BridgeLoop {
    /// First loop.
    First,
    /// Second loop.
    Second,
}
/// Typed bridge preparation or application failure.
#[derive(Clone, Debug, PartialEq)]
pub enum BridgeError {
    /// At least one selected loop is empty.
    EmptyLoop,
    /// Selected loops share an edge.
    OverlappingLoops,
    /// The two traversed loops have different vertex counts.
    UnequalLengths {
        /// First loop's vertex count.
        first: usize,
        /// Second loop's vertex count.
        second: usize,
    },
    /// Kernel boundary traversal failed.
    Boundary {
        /// Input loop being checked.
        input: BridgeLoop,
        /// Original typed topology error.
        error: exedra_mesh::BoundaryLoopError,
    },
    /// Selection does not contain exactly one complete canonical boundary loop.
    IncompleteLoop {
        /// Input loop being checked.
        input: BridgeLoop,
    },
    /// Boundary has a stale canonical edge or vertex.
    InvalidBoundary {
        /// Input loop being checked.
        input: BridgeLoop,
    },
    /// An adjacent source face cannot be captured for stale checks.
    SourceSelection(exedra_mesh::SelectedFacePatchError),
    /// Positions, consumed attributes or alignment scores are not representable.
    NumericLimit,
    /// Source revision or captured topology/geometry/attributes changed.
    StalePreparation,
    /// Kernel refused a strip face; earlier eager edits may remain.
    AddFace(op::AddFaceError),
    /// A generated face has no anchor edge.
    InvalidGeneratedFace,
}
impl core::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyLoop => f.write_str("bridge needs two nonempty boundary loops"),
            Self::OverlappingLoops => f.write_str("bridge loops overlap"),
            Self::UnequalLengths { first, second } => {
                write!(f, "bridge loop counts differ: {first} and {second}")
            }
            Self::Boundary { input, error } => write!(f, "bridge {input:?} loop: {error}"),
            Self::IncompleteLoop { input } => {
                write!(f, "bridge {input:?} selection is not one complete loop")
            }
            Self::InvalidBoundary { input } => {
                write!(f, "bridge {input:?} boundary is stale or malformed")
            }
            Self::SourceSelection(error) => write!(f, "bridge source faces: {error}"),
            Self::NumericLimit => f.write_str("bridge alignment exceeds numeric limits"),
            Self::StalePreparation => {
                f.write_str("bridge preparation no longer matches its source")
            }
            Self::AddFace(error) => write!(f, "bridge face insertion: {error}"),
            Self::InvalidGeneratedFace => {
                f.write_str("bridge generated a face without an anchor edge")
            }
        }
    }
}
impl core::error::Error for BridgeError {}
impl From<CaptureError> for BridgeError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Selection(error) => Self::SourceSelection(error),
            CaptureError::NumericLimit { .. } => Self::NumericLimit,
        }
    }
}
/// Completed bridge work, without execution-layer reports.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Authored boundary edges consumed, counting both loops.
    pub boundary_edges: u64,
    /// Preparation sorted or deduplicated either input.
    pub selections_canonicalized: bool,
    /// Frame orientation reversed to fit existing boundary direction.
    pub reversed_winding: bool,
}
/// Parameters for [`bridge_boundary_loops`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BridgeBoundaryLoopsParams {
    /// First canonical boundary loop.
    pub loop_a: EdgeSet,
    /// Second canonical boundary loop.
    pub loop_b: EdgeSet,
}

/// Deterministic compiled plan payload for [`bridge_boundary_loops`].
#[derive(Clone, Debug)]
pub struct BridgeBoundaryLoopsPlan {
    loop_a: EdgeSet,
    loop_b: EdgeSet,
    aligned_a_vertices: Vec<VertexId>,
    aligned_b_vertices: Vec<VertexId>,
    selections_canonicalized: bool,
    source_faces: Vec<FaceId>,
    inputs: Vec<FaceInput>,
    revision: exedra_mesh::MeshRevision,
}

/// Typed output from [`bridge_boundary_loops`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BridgeBoundaryLoopsOutput {
    /// Created quad-strip faces in traversal order.
    pub bridge_faces: Vec<FaceId>,
}

impl BridgeBoundaryLoopsPlan {
    /// Pairs two complete, disjoint canonical boundary loops of equal length.
    /// All cyclic shifts and both directions are compared by summed squared
    /// distance. Ties keep traversal order. This is a geometric pairing heuristic,
    /// not a guarantee against twisted or self-intersecting strips.
    pub fn prepare(mesh: &Mesh, params: &BridgeBoundaryLoopsParams) -> Result<Self, BridgeError> {
        let mut loop_a = params.loop_a.clone();
        let mut loop_b = params.loop_b.clone();
        let canonicalized_a = canonicalize_edge_set(&mut loop_a);
        let canonicalized_b = canonicalize_edge_set(&mut loop_b);

        if loop_a.is_empty() || loop_b.is_empty() {
            return Err(BridgeError::EmptyLoop);
        }
        if loop_a.iter().any(|edge| loop_b.binary_search(edge).is_ok()) {
            return Err(BridgeError::OverlappingLoops);
        }

        let ordered_a = ordered_boundary_loop(mesh, &loop_a, BridgeLoop::First)?;
        let ordered_b = ordered_boundary_loop(mesh, &loop_b, BridgeLoop::Second)?;
        if ordered_a.vertices.len() != ordered_b.vertices.len() {
            return Err(BridgeError::UnequalLengths {
                first: ordered_a.vertices.len(),
                second: ordered_b.vertices.len(),
            });
        }

        let aligned_b = align_loop_vertices(mesh, &ordered_a.vertices, &ordered_b.vertices)
            .ok_or(BridgeError::NumericLimit)?;

        let mut source_faces = Vec::new();
        for &edge in loop_a.iter().chain(&loop_b) {
            for candidate in [Some(edge), mesh.twin(edge)].into_iter().flatten() {
                if let Some(face) = mesh.face(candidate).filter(|&face| face != FaceId::OUTSIDE) {
                    source_faces.push(face);
                }
            }
        }
        source_faces.sort_unstable();
        source_faces.dedup();
        let inputs = capture_inputs(mesh, &source_faces)?;
        Ok(Self {
            source_faces,
            inputs,
            revision: mesh.revision(),
            loop_a,
            loop_b,
            aligned_a_vertices: ordered_a.vertices,
            aligned_b_vertices: aligned_b,
            selections_canonicalized: canonicalized_a || canonicalized_b,
        })
    }
    /// Canonical first input loop.
    #[must_use]
    pub fn loop_a(&self) -> &[HalfEdgeId] {
        &self.loop_a
    }
    /// Canonical second input loop.
    #[must_use]
    pub fn loop_b(&self) -> &[HalfEdgeId] {
        &self.loop_b
    }
    /// First loop vertices, in strip order.
    #[must_use]
    pub fn aligned_a_vertices(&self) -> &[VertexId] {
        &self.aligned_a_vertices
    }
    /// Second loop vertices, paired with the first loop.
    #[must_use]
    pub fn aligned_b_vertices(&self) -> &[VertexId] {
        &self.aligned_b_vertices
    }
    /// Whether preparation sorted or deduplicated either input.
    #[must_use]
    pub fn selections_canonicalized(&self) -> bool {
        self.selections_canonicalized
    }
    /// Builds the prepared quad strip after checking source revision and exact
    /// adjacent-face state, including unfinished edits. Equivalent clones are
    /// accepted. Strip rims are sharp and columns smooth. Generated faces have no
    /// source face, so region, UVs, normal overrides and caller-defined layers
    /// start empty; nothing is deleted, so no value is lost.
    /// Kernel failures may leave partial edits; finish the caller's change sink
    /// after success or failure. This method does not imply rollback.
    pub fn apply<S: exedra_mesh::ChangeSink>(
        &self,
        txn: &mut exedra_mesh::EditSession<'_, S>,
    ) -> Result<(BridgeStats, BridgeBoundaryLoopsOutput), BridgeError> {
        let plan = self;
        if txn.mesh().revision() != self.revision
            || capture_inputs(txn.mesh(), &self.source_faces).ok().as_ref() != Some(&self.inputs)
        {
            return Err(BridgeError::StalePreparation);
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
            )
            .map_err(BridgeError::AddFace)?;
            // Semantic bridge boundaries are hard by default; columns stay smooth.
            let first_edge = txn
                .mesh()
                .face_edge(face)
                .ok_or(BridgeError::InvalidGeneratedFace)?;
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

        Ok((
            BridgeStats {
                boundary_edges: u64::try_from(plan.loop_a.len() + plan.loop_b.len())
                    .expect("edge count fits u64"),
                selections_canonicalized: plan.selections_canonicalized,
                reversed_winding: !orientation.prefers_forward_outer_edge(),
            },
            BridgeBoundaryLoopsOutput { bridge_faces },
        ))
    }
}
/// Prepares and bridges two mesh boundary loops in an eager edit session.
/// See [`BridgeBoundaryLoopsPlan`] for pairing and failure semantics.
pub fn bridge_boundary_loops<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    params: &BridgeBoundaryLoopsParams,
) -> Result<(BridgeStats, BridgeBoundaryLoopsOutput), BridgeError> {
    BridgeBoundaryLoopsPlan::prepare(txn.mesh(), params)?.apply(txn)
}
struct OrderedBoundaryLoop {
    vertices: Vec<VertexId>,
}
fn ordered_boundary_loop(
    mesh: &Mesh,
    canonical_edges: &[HalfEdgeId],
    input: BridgeLoop,
) -> Result<OrderedBoundaryLoop, BridgeError> {
    let boundary = mesh
        .boundary_loop(canonical_edges[0])
        .map_err(|error| BridgeError::Boundary { input, error })?;
    let mut vertices = Vec::with_capacity(boundary.len());
    for &edge in &boundary {
        let canonical = mesh
            .canonical_edge(edge)
            .ok_or(BridgeError::InvalidBoundary { input })?;
        if canonical_edges.binary_search(&canonical).is_err() {
            return Err(BridgeError::IncompleteLoop { input });
        }
        vertices.push(
            mesh.from_vertex(edge)
                .ok_or(BridgeError::InvalidBoundary { input })?,
        );
    }
    if boundary.len() != canonical_edges.len() {
        return Err(BridgeError::IncompleteLoop { input });
    }
    Ok(OrderedBoundaryLoop { vertices })
}
fn align_loop_vertices(
    mesh: &Mesh,
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

fn candidate_alignment_score(mesh: &Mesh, loop_a: &[VertexId], loop_b: &[VertexId]) -> Option<u64> {
    let mut score = 0.0_f64;
    for (&a, &b) in loop_a.iter().zip(loop_b.iter()) {
        let pa = mesh.vertex_position(a)?;
        let pb = mesh.vertex_position(b)?;
        let dx = f64::from(pa[0] - pb[0]);
        let dy = f64::from(pa[1] - pb[1]);
        let dz = f64::from(pa[2] - pb[2]);
        score += dx * dx + dy * dy + dz * dz;
    }
    score.is_finite().then_some(score.to_bits())
}

#[cfg(test)]
mod tests;
