// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh splitting along the intersection graph.
//!
//! After this stage, intersection curves lie exactly on mesh edges: graph
//! vertices on mesh edges become real vertices via kernel edge splits,
//! face-interior graph vertices become new vertices, and every crossed
//! face is re-partitioned along its chains — so patch classification
//! (`exe-07nj`) can walk edges instead of geometry.
//!
//! The stage rides the kernel's existing machinery: [`crate::op::split_edge`]
//! (attribute propagation via [`crate::PropagatePolicy`]) inserts on-edge
//! vertices, and face re-partitioning captures the face's region and
//! boundary-edge attributes, deletes the face, re-adds one face per
//! sub-loop, and re-applies the captured attributes. Every produced face is
//! traceable to its pre-split face through the returned origin mapping.
//!
//! Positions for new vertices narrow from the graph's f64 constructions to
//! f32 (round-to-nearest-even) — the same single-narrowing discipline as
//! tessellation.
//!
//! Closed intersection loops fully interior to one face (the through-hole
//! configuration: a prism drilled through a slab cap) re-face the host
//! into a triangulated ring (the original boundary with the loops as
//! holes, via `exedra_triangulate`'s hole bridging) plus a triangulated
//! disk per loop, so classification can separate the ring from the disk
//! along real mesh edges. When open chains and closed loops share a face, the
//! chains first define simple regions and each loop is assigned to exactly
//! one region before the same ring-and-disk refacing. This is the common
//! timber configuration of an end/shoulder cut plus a blind housing.
//!
//! Configurations outside the supported envelope are typed deferrals, never
//! silent: dangling chains ending inside a face, ambiguous region assignment,
//! and loops whose projection or triangulation degenerates all leave the face
//! unsplit and report [`BooleanFailureKind::SplitDeferred`]. Planar T-junctions
//! are decomposed into straight boundary chains followed by their residual
//! branches, so later cutters may meet earlier cut seams.

use alloc::vec::Vec;

use hashbrown::{HashMap, HashSet};

use exedra_triangulate::predicates::{Orientation, orient2d};

use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::edge_index::HalfEdgeIndex;
use super::graph::{IntersectionGraph, MeshAnchor};
use crate::{
    DeletePolicy, FaceId, Mesh, PropagatePolicy, VertexId, attr,
    op::{
        add_face, add_vertex, delete_faces, set_edge_seam, set_edge_sharpness, set_face_region,
        set_vertex_position, split_edge,
    },
};
use exedra_math::{dot, narrow, promote, sub};

/// Which mesh of the boolean pair is being split.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MeshSide {
    /// The first operand.
    A,
    /// The second operand.
    B,
}

/// Deterministic splitting counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitStats {
    /// Kernel edge splits performed.
    pub edge_splits: u64,
    /// Faces re-partitioned along chains.
    pub faces_split: u64,
    /// Faces created by re-partitioning.
    pub faces_created: u64,
    /// Interior graph vertices materialized.
    pub interior_vertices: u64,
    /// Faces deferred with a typed diagnostic.
    pub deferred_faces: u64,
    /// Graph edges skipped because they already lie on mesh edges.
    pub on_edge_edges: u64,
}

/// The result of splitting one mesh along the graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshSplitOutcome {
    /// `(created face, original face)` for every face produced by
    /// re-partitioning — the origin attribution later stages and
    /// constructive source maps compose with.
    pub face_origins: Vec<(FaceId, FaceId)>,
    /// Graph-vertex index → mesh vertex on this side, for every graph
    /// vertex that materialized (existing, on-edge, or interior).
    pub graph_vertices: Vec<Option<VertexId>>,
    /// Splitting counters.
    pub stats: SplitStats,
}

/// Splits `mesh` so the intersection curves of `graph` lie on mesh edges.
///
/// Deterministic: mesh edges split in ascending vertex-pair order with
/// points ordered along each edge; faces re-partition in ascending face
/// index; chains within a face process in ascending graph-vertex order.
/// Deferred configurations are diagnosed per face and leave that face
/// untouched (the mesh stays valid either way).
pub fn split_mesh_along_graph(
    mesh: &mut Mesh,
    graph: &IntersectionGraph,
    side: MeshSide,
    diagnostics: &mut BooleanDiagnostics,
) -> MeshSplitOutcome {
    let mut outcome = MeshSplitOutcome {
        graph_vertices: alloc::vec![None; graph.vertices.len()],
        ..MeshSplitOutcome::default()
    };

    let anchor_of = |index: usize| match side {
        MeshSide::A => graph.vertices[index].anchor_a,
        MeshSide::B => graph.vertices[index].anchor_b,
    };
    // Graph-vertex degrees over the cut network: degree-0 vertices are
    // touch points (or unwelded coincident duplicates of a welded curve
    // vertex); no cut passes through them, so splitting mesh edges for
    // them would only leave orphan coincident vertices behind.
    let mut degree = alloc::vec![0_u32; graph.vertices.len()];
    for edge in &graph.edges {
        for &vertex in &edge.vertices {
            degree[vertex as usize] += 1;
        }
    }
    let mut half_edge_index = HalfEdgeIndex::build(mesh);

    // --- Stage 1: resolve vertex-anchored and edge-anchored graph
    // vertices to mesh vertices (kernel edge splits for the latter).
    let mut on_edge: HashMap<(VertexId, VertexId), Vec<u32>> = HashMap::new();
    for (index, &vertex_degree) in degree.iter().enumerate() {
        match anchor_of(index) {
            MeshAnchor::Vertex(vertex) => {
                outcome.graph_vertices[index] = Some(vertex);
            }
            MeshAnchor::EdgeSpan(u, v) => {
                if vertex_degree > 0 && half_edge_index.find(u, v).is_some() {
                    on_edge
                        .entry((u, v))
                        .or_default()
                        .push(u32::try_from(index).unwrap_or(u32::MAX));
                }
                // Diagonal spans are face-interior at mesh level; they
                // materialize during face re-partitioning.
            }
            MeshAnchor::FaceInterior(_) => {}
        }
    }
    let mut edge_groups: Vec<((VertexId, VertexId), Vec<u32>)> = on_edge.into_iter().collect();
    edge_groups.sort_unstable_by_key(|((u, v), _)| (u.index(), v.index()));

    let policy = PropagatePolicy::default();
    {
        let mut session = mesh.edit();
        for ((u, v), mut points) in edge_groups {
            // Order points along the edge by the dominant axis of the
            // (promoted) edge vector: deterministic, division-free compare.
            let from = session.mesh().vertex_position(u).copied().map(promote);
            let to = session.mesh().vertex_position(v).copied().map(promote);
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            let axis = dominant_axis(sub(to, from));
            let ascending = to[axis] >= from[axis];
            points.sort_unstable_by(|&p, &q| {
                let a = graph.vertices[p as usize].position[axis];
                let b = graph.vertices[q as usize].position[axis];
                if ascending {
                    a.total_cmp(&b)
                } else {
                    b.total_cmp(&a)
                }
            });

            let mut cursor = u;
            for point in points {
                let Some(half_edge) = half_edge_index.find(cursor, v) else {
                    diagnostics.push(BooleanDiagnostic {
                        kind: BooleanFailureKind::InternalInvariantViolation,
                        a: None,
                        b: None,
                        detail: "edge disappeared while splitting crossing points",
                    });
                    break;
                };
                let Ok(new_vertex) = split_edge(&mut session, half_edge, &policy) else {
                    diagnostics.push(BooleanDiagnostic {
                        kind: BooleanFailureKind::InternalInvariantViolation,
                        a: None,
                        b: None,
                        detail: "kernel edge split failed on a live edge",
                    });
                    break;
                };
                half_edge_index.refresh_after_split(session.mesh(), half_edge, cursor, v);
                let position = narrow(graph.vertices[point as usize].position);
                let _ = set_vertex_position(&mut session, new_vertex, position);
                outcome.graph_vertices[point as usize] = Some(new_vertex);
                outcome.stats.edge_splits += 1;
                cursor = new_vertex;
            }
        }
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
    }

    // --- Stage 2: group graph edges by the face they cross on this side.
    let mut per_face: HashMap<FaceId, Vec<u32>> = HashMap::new();
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        // A subsegment whose endpoints already form a mesh edge (existing
        // edges, or edges produced by stage 1's splits) needs no face
        // re-partitioning: the cut already lies on the mesh.
        let [p, q] = edge.vertices;
        let on_mesh_edge = outcome.graph_vertices[p as usize]
            .zip(outcome.graph_vertices[q as usize])
            .is_some_and(|(u, v)| half_edge_index.find(u, v).is_some());
        if on_mesh_edge {
            outcome.stats.on_edge_edges += 1;
            continue;
        }
        let mut faces: Vec<FaceId> = edge
            .crossings
            .iter()
            .map(|&(a, b)| match side {
                MeshSide::A => a,
                MeshSide::B => b,
            })
            .collect();
        faces.sort_unstable_by_key(|f| f.index());
        faces.dedup();
        if faces.len() > 1 {
            // The subsegment is attributed to several faces on this side:
            // it runs along a shared mesh edge, so no split is needed here.
            outcome.stats.on_edge_edges += 1;
            continue;
        }
        per_face
            .entry(faces[0])
            .or_default()
            .push(u32::try_from(edge_index).unwrap_or(u32::MAX));
    }
    let mut faces: Vec<(FaceId, Vec<u32>)> = per_face.into_iter().collect();
    faces.sort_unstable_by_key(|(face, _)| face.index());

    // --- Stage 3: re-partition each crossed face along its chains.
    for (face, edge_indices) in faces {
        split_one_face(mesh, graph, face, &edge_indices, &mut outcome, diagnostics);
    }
    outcome
        .face_origins
        .sort_unstable_by_key(|(new, old)| (old.index(), new.index()));
    outcome
}

fn dominant_axis(v: [f64; 3]) -> usize {
    let abs = [v[0].abs(), v[1].abs(), v[2].abs()];
    if abs[0] >= abs[1] && abs[0] >= abs[2] {
        0
    } else if abs[1] >= abs[2] {
        1
    } else {
        2
    }
}

#[cfg(test)]
fn find_half_edge(mesh: &Mesh, from: VertexId, to: VertexId) -> Option<crate::HalfEdgeId> {
    HalfEdgeIndex::build(mesh).find(from, to)
}

fn defer(
    detail: &'static str,
    outcome: &mut MeshSplitOutcome,
    diagnostics: &mut BooleanDiagnostics,
) {
    outcome.stats.deferred_faces += 1;
    diagnostics.push(BooleanDiagnostic {
        kind: BooleanFailureKind::SplitDeferred,
        a: None,
        b: None,
        detail,
    });
}

/// A face-partition vertex before the partition has been accepted.
///
/// Graph vertices stay symbolic while chains are traced and validated. This
/// keeps every typed deferral truly non-mutating: only a complete, simple,
/// incrementally rebuildable partition is allowed to allocate mesh vertices.
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum FacePartitionVertex {
    Mesh(VertexId),
    Graph(u32),
}

fn split_one_face(
    mesh: &mut Mesh,
    graph: &IntersectionGraph,
    face: FaceId,
    edge_indices: &[u32],
    outcome: &mut MeshSplitOutcome,
    diagnostics: &mut BooleanDiagnostics,
) {
    // Face-local adjacency over graph vertices.
    let mut local: HashMap<u32, Vec<u32>> = HashMap::new();
    for &edge_index in edge_indices {
        let [p, q] = graph.edges[edge_index as usize].vertices;
        local.entry(p).or_default().push(q);
        local.entry(q).or_default().push(p);
    }
    for neighbors in local.values_mut() {
        neighbors.sort_unstable();
    }
    let loop_vertices: Vec<VertexId> = mesh
        .face_loop(face)
        .filter_map(|he| mesh.to_vertex(he))
        .collect();
    let is_boundary_vertex = |graph_vertex: u32| {
        outcome.graph_vertices[graph_vertex as usize]
            .is_some_and(|vertex| loop_vertices.contains(&vertex))
    };

    // Degree-three vertices are normal when a later cutter meets an earlier
    // cut: the new face carries an outer crossing chain with an interior
    // branch into the old seam. Continue straight through such junctions
    // first. That produces boundary-to-boundary chains before the residual
    // branch chains, whose endpoints become real vertices when the straight
    // chains are cut below.
    let mut terminals: Vec<u32> = local
        .iter()
        .filter(|(vertex, neighbors)| neighbors.len() == 1 || is_boundary_vertex(**vertex))
        .map(|(&vertex, _)| vertex)
        .collect();
    terminals.sort_unstable();
    if terminals.is_empty()
        && !local.is_empty()
        && local.values().all(|neighbors| neighbors.len() == 2)
    {
        split_face_with_interior_loops(mesh, graph, face, &local, outcome, diagnostics);
        return;
    }

    let edge_key = |a: u32, b: u32| (a.min(b), a.max(b));
    let mut visited: HashMap<(u32, u32), bool> = HashMap::new();
    let mut chains: Vec<Vec<u32>> = Vec::new();
    let mut closed_loops: Vec<Vec<u32>> = Vec::new();
    let trace = |start: u32, next: u32, visited: &mut HashMap<(u32, u32), bool>| -> Vec<u32> {
        let mut chain = alloc::vec![start, next];
        let mut previous = start;
        let mut current = next;
        visited.insert(edge_key(start, next), true);
        loop {
            // A cut that reaches the face boundary must stop there. Letting a
            // chain pass through that vertex would place the same mesh vertex
            // both on the old loop and inside the new chord, producing a
            // repeated vertex and corrupting the OUTSIDE half-edge cycle.
            if current != start && is_boundary_vertex(current) {
                break;
            }
            let candidates: Vec<u32> = local[&current]
                .iter()
                .copied()
                .filter(|&candidate| !visited.contains_key(&edge_key(current, candidate)))
                .collect();
            if candidates.is_empty() {
                break;
            }
            let step = if local[&current].len() == 2 {
                candidates.first().copied()
            } else {
                // At a branch, pair the incoming edge with the most nearly
                // straight continuation. Intersections computed by adjacent
                // triangle pairs can differ by a few f64 rounding bits even
                // when their source edges are collinear, so exact collinearity
                // is too strict here. The squared projection score compares
                // direction without a tolerance or a square root; only an
                // obtuse continuation can win, and graph-index order breaks
                // exact ties deterministically.
                let incoming = sub(
                    graph.vertices[previous as usize].position,
                    graph.vertices[current as usize].position,
                );
                candidates
                    .iter()
                    .copied()
                    .filter_map(|candidate| {
                        let outgoing = sub(
                            graph.vertices[candidate as usize].position,
                            graph.vertices[current as usize].position,
                        );
                        let projection = dot(incoming, outgoing);
                        let score = projection * projection / dot(outgoing, outgoing);
                        (projection < 0.0 && score.is_finite()).then_some((candidate, score))
                    })
                    .max_by(|(a, score_a), (b, score_b)| {
                        score_a.total_cmp(score_b).then_with(|| b.cmp(a))
                    })
                    .map(|(candidate, _)| candidate)
            };
            let Some(step) = step else { break };
            visited.insert(edge_key(current, step), true);
            chain.push(step);
            previous = current;
            current = step;
        }
        chain
    };
    for &start in &terminals {
        for &next in &local[&start] {
            let key = edge_key(start, next);
            if visited.contains_key(&key) {
                continue;
            }
            chains.push(trace(start, next, &mut visited));
        }
    }

    // Trace residual branch-to-branch chains after the terminal chains. Once
    // only degree-two components remain, preserve them as closed loops. They
    // are assigned to the open-chain partitions below in face space; cutter
    // order must not decide whether a blind housing can coexist with a
    // shoulder cut on the same timber face.
    loop {
        let mut branch_starts: Vec<u32> = edge_indices
            .iter()
            .filter_map(|&edge_index| {
                let [a, b] = graph.edges[edge_index as usize].vertices;
                (!visited.contains_key(&edge_key(a, b))).then_some([a, b])
            })
            .flatten()
            .filter(|vertex| local[vertex].len() != 2)
            .collect();
        branch_starts.sort_unstable();
        branch_starts.dedup();
        let remaining = edge_indices.iter().any(|&edge_index| {
            let [a, b] = graph.edges[edge_index as usize].vertices;
            !visited.contains_key(&edge_key(a, b))
        });
        if !remaining {
            break;
        }
        let Some(start) = branch_starts.first().copied() else {
            while let Some([start, next]) = edge_indices.iter().find_map(|&edge_index| {
                let [a, b] = graph.edges[edge_index as usize].vertices;
                (!visited.contains_key(&edge_key(a, b))).then_some([a, b])
            }) {
                let mut ring = alloc::vec![start];
                let mut previous = start;
                let mut current = next;
                visited.insert(edge_key(start, next), true);
                while current != start {
                    ring.push(current);
                    let Some(step) = local[&current].iter().copied().find(|&candidate| {
                        candidate != previous
                            && !visited.contains_key(&edge_key(current, candidate))
                    }) else {
                        defer(
                            "mixed interior cut loop is not a closed degree-two component",
                            outcome,
                            diagnostics,
                        );
                        return;
                    };
                    visited.insert(edge_key(current, step), true);
                    previous = current;
                    current = step;
                }
                if ring.len() < 3 {
                    defer(
                        "mixed interior cut loop has fewer than three vertices",
                        outcome,
                        diagnostics,
                    );
                    return;
                }
                closed_loops.push(ring);
            }
            break;
        };
        let next = local[&start]
            .iter()
            .copied()
            .find(|&candidate| !visited.contains_key(&edge_key(start, candidate)))
            .expect("remaining branch start has an unvisited edge");
        chains.push(trace(start, next, &mut visited));
    }

    // Cut any chain whose endpoints currently share one sub-loop. Straight
    // chains through a junction run first because their original boundary
    // endpoints are already resolved; materializing their interior vertices
    // then makes residual branch endpoints eligible without geometric guesses.
    let partition_vertex = |graph_vertex: u32, outcome: &MeshSplitOutcome| {
        outcome.graph_vertices[graph_vertex as usize].map_or(
            FacePartitionVertex::Graph(graph_vertex),
            FacePartitionVertex::Mesh,
        )
    };
    let boundary_partition: Vec<FacePartitionVertex> = loop_vertices
        .iter()
        .copied()
        .map(FacePartitionVertex::Mesh)
        .collect();
    let mut sub_loops = alloc::vec![boundary_partition.clone()];
    let mut pending: Vec<&Vec<u32>> = chains.iter().collect();
    pending.sort_unstable_by_key(|chain| {
        let first = *chain.first().expect("nonempty");
        let last = *chain.last().expect("nonempty");
        (first.min(last), first.max(last))
    });

    while !pending.is_empty() {
        let mut ready = None;
        for (pending_index, chain) in pending.iter().enumerate() {
            let first = partition_vertex(*chain.first().expect("nonempty"), outcome);
            let last = partition_vertex(*chain.last().expect("nonempty"), outcome);
            let slots: Vec<usize> = sub_loops
                .iter()
                .enumerate()
                .filter_map(|(slot, sub)| {
                    (sub.contains(&first) && sub.contains(&last)).then_some(slot)
                })
                .collect();
            if slots.len() == 1 {
                ready = Some((pending_index, slots[0], first, last));
                break;
            }
            if slots.len() > 1 {
                defer(
                    "chain endpoints lie on a previous cut (ambiguous cut)",
                    outcome,
                    diagnostics,
                );
                return;
            }
        }
        let Some((pending_index, slot, first, last)) = ready else {
            let unresolved = pending.iter().any(|chain| {
                [chain.first(), chain.last()].into_iter().any(|endpoint| {
                    endpoint.is_none_or(|&graph_vertex| {
                        let vertex = partition_vertex(graph_vertex, outcome);
                        !sub_loops.iter().any(|sub_loop| sub_loop.contains(&vertex))
                    })
                })
            });
            defer(
                if unresolved {
                    "intersection chain ends inside the face (dangling cut)"
                } else {
                    "chain endpoints span different sub-loops (ambiguous cut)"
                },
                outcome,
                diagnostics,
            );
            return;
        };
        let chain = pending.remove(pending_index);

        let sub = sub_loops.swap_remove(slot);
        let iu = sub.iter().position(|&v| v == first).expect("contained");
        let iw = sub.iter().position(|&v| v == last).expect("contained");
        let n = sub.len();
        if chain.len() == 2 && ((iu + 1) % n == iw || (iw + 1) % n == iu) {
            // The chain coincides with an existing sub-loop edge; nothing
            // to cut.
            outcome.stats.on_edge_edges += 1;
            sub_loops.push(sub);
            continue;
        }
        let interior: Vec<FacePartitionVertex> = chain[1..chain.len() - 1]
            .iter()
            .map(|&vertex| partition_vertex(vertex, outcome))
            .collect();

        // Loop 1: first .. last along the loop, then chain reversed back.
        let mut loop_one = Vec::new();
        let mut i = iu;
        loop {
            loop_one.push(sub[i]);
            if i == iw {
                break;
            }
            i = (i + 1) % n;
        }
        loop_one.extend(interior.iter().rev().copied());
        // Loop 2: last .. first along the loop, then chain forward.
        let mut loop_two = Vec::new();
        let mut i = iw;
        loop {
            loop_two.push(sub[i]);
            if i == iu {
                break;
            }
            i = (i + 1) % n;
        }
        loop_two.extend(interior.iter().copied());

        sub_loops.push(loop_one);
        sub_loops.push(loop_two);
    }
    if sub_loops.len() <= 1 {
        return; // Nothing actually cut.
    }

    // Chain tracing can revisit a junction while consuming a residual cycle.
    // Such a walk is not a simple polygon: `add_face` will reject its repeated
    // vertex. Validate the whole batch before deleting the original face,
    // because edit sessions are eager and cannot roll back a partially rebuilt
    // partition when one later sub-loop is invalid.
    if sub_loops.iter().any(|sub_loop| !is_simple_loop(sub_loop)) {
        defer(
            "face partition produced a non-simple sub-loop",
            outcome,
            diagnostics,
        );
        return;
    }

    if !closed_loops.is_empty() {
        split_face_with_partitioned_interior_loops(
            mesh,
            graph,
            face,
            &boundary_partition,
            &sub_loops,
            &closed_loops,
            outcome,
            diagnostics,
        );
        return;
    }

    // Rebuilding the partition one face at a time must keep the OUTSIDE
    // boundary a set of simple loops after every addition. A geometrically
    // valid partition can otherwise panic the incremental stitcher when a
    // later sub-loop first touches the growing region at a bare vertex.
    let Some(rebuild_order) = order_sub_loops(&boundary_partition, &sub_loops) else {
        defer(
            "face partition has no ambiguity-free incremental rebuild order",
            outcome,
            diagnostics,
        );
        return;
    };

    // The partition is now fully accepted. Materialize its symbolic graph
    // vertices once, immediately before the mesh rebuild. No earlier typed
    // deferral can leave an isolated vertex, grow an arena, or advance the
    // mesh revision.
    let mut materialized_sub_loops = Vec::with_capacity(sub_loops.len());
    let mut session = mesh.edit();
    for sub_loop in sub_loops {
        let mut materialized = Vec::with_capacity(sub_loop.len());
        for vertex in sub_loop {
            let vertex = match vertex {
                FacePartitionVertex::Mesh(vertex) => vertex,
                FacePartitionVertex::Graph(graph_vertex) => {
                    if let Some(vertex) = outcome.graph_vertices[graph_vertex as usize] {
                        vertex
                    } else {
                        let position = narrow(graph.vertices[graph_vertex as usize].position);
                        let vertex = add_vertex(&mut session, position);
                        outcome.graph_vertices[graph_vertex as usize] = Some(vertex);
                        outcome.stats.interior_vertices += 1;
                        vertex
                    }
                }
            };
            materialized.push(vertex);
        }
        materialized_sub_loops.push(materialized);
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    let sub_loops = materialized_sub_loops;

    // Capture attributes, delete, re-add, re-apply, record origins.
    let region = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face.as_id()).copied());
    let boundary_attrs: Vec<(VertexId, VertexId, Option<f32>, Option<bool>)> = {
        let mut captured = Vec::new();
        let loop_edges: Vec<crate::HalfEdgeId> = mesh.face_loop(face).collect();
        for half_edge in loop_edges {
            let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
            else {
                continue;
            };
            let sharpness = mesh.edge_sharpness(half_edge).filter(|s| *s != 0.0);
            let seam = mesh.edge_seam(half_edge).filter(|s| *s);
            if sharpness.is_some() || seam.is_some() {
                captured.push((from, to, sharpness, seam));
            }
        }
        captured
    };

    let mut session = mesh.edit();
    if delete_faces(&mut session, &[face], DeletePolicy::KeepIsolated).is_err() {
        diagnostics.push(BooleanDiagnostic {
            kind: BooleanFailureKind::InternalInvariantViolation,
            a: None,
            b: None,
            detail: "crossed face could not be deleted for re-partitioning",
        });
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
        return;
    }
    let mut created = 0_u64;
    for index in rebuild_order {
        let sub = &sub_loops[index];
        match add_face(&mut session, sub) {
            Ok(new_face) => {
                outcome.face_origins.push((new_face, face));
                if let Some(region) = region {
                    let _ = set_face_region(&mut session, new_face, region);
                }
                created += 1;
            }
            Err(_) => {
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::InternalInvariantViolation,
                    a: None,
                    b: None,
                    detail: "re-partitioned sub-loop was rejected by add_face",
                });
            }
        }
    }
    // Re-apply captured boundary edge attributes where the edges survive.
    for (from, to, sharpness, seam) in boundary_attrs {
        if let Some(half_edge) = session.find_half_edge(from, to) {
            if let Some(sharpness) = sharpness {
                let _ = set_edge_sharpness(&mut session, half_edge, sharpness);
            }
            if let Some(seam) = seam {
                let _ = set_edge_seam(&mut session, half_edge, seam);
            }
        }
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    outcome.stats.faces_split += 1;
    outcome.stats.faces_created += created;
}

/// Returns whether a generated face loop can be submitted to `add_face` as
/// one simple polygon.
///
/// Length and vertex uniqueness cover the structural failures the splitter
/// can introduce. All vertices came from the live source face or were created
/// earlier in this function, so liveness does not need another pass here.
fn is_simple_loop<T: PartialEq>(vertices: &[T]) -> bool {
    vertices.len() >= 3
        && vertices
            .iter()
            .enumerate()
            .all(|(index, vertex)| !vertices[(index + 1)..].contains(vertex))
}

/// Orders polygonal sub-loops so every face addition attaches along an open
/// edge at each vertex it shares with the growing partition.
///
/// The original boundary is already present through neighboring faces after
/// the crossed face is deleted. Internal edges admit two new faces; original
/// boundary edges admit one. This is the polygonal counterpart of
/// [`order_fragments`] for drilled-face triangles.
fn order_sub_loops<T>(boundary: &[T], sub_loops: &[Vec<T>]) -> Option<Vec<usize>>
where
    T: Copy + Eq + core::hash::Hash + Ord,
{
    let edge_key = |a: T, b: T| {
        if a <= b { (a, b) } else { (b, a) }
    };
    let mut present: HashSet<T> = boundary.iter().copied().collect();
    let mut capacity: HashMap<(T, T), u8> = HashMap::new();
    for index in 0..boundary.len() {
        capacity.insert(
            edge_key(boundary[index], boundary[(index + 1) % boundary.len()]),
            1,
        );
    }

    let mut order = Vec::with_capacity(sub_loops.len());
    let mut added = alloc::vec![false; sub_loops.len()];
    loop {
        let mut progressed = false;
        for (index, sub_loop) in sub_loops.iter().enumerate() {
            if added[index] {
                continue;
            }
            let open = |a: T, b: T, capacity: &HashMap<(T, T), u8>| {
                capacity.get(&edge_key(a, b)).copied().unwrap_or(0) > 0
            };
            let mut attaches = false;
            let mut safe = true;
            for corner in 0..sub_loop.len() {
                let vertex = sub_loop[corner];
                if !present.contains(&vertex) {
                    continue;
                }
                let previous = sub_loop[(corner + sub_loop.len() - 1) % sub_loop.len()];
                let next = sub_loop[(corner + 1) % sub_loop.len()];
                if open(vertex, previous, &capacity) || open(vertex, next, &capacity) {
                    attaches = true;
                } else {
                    safe = false;
                    break;
                }
            }
            if !safe || !attaches {
                continue;
            }
            added[index] = true;
            order.push(index);
            for corner in 0..sub_loop.len() {
                let a = sub_loop[corner];
                let b = sub_loop[(corner + 1) % sub_loop.len()];
                let entry = capacity.entry(edge_key(a, b)).or_insert(2);
                *entry = entry.saturating_sub(1);
                present.insert(a);
            }
            progressed = true;
        }
        if order.len() == sub_loops.len() {
            return Some(order);
        }
        if !progressed {
            return None;
        }
    }
}

/// Orders fragment triangles so each addition attaches to the growing
/// region along an open (boundary) edge at every vertex it already
/// touches. The incremental stitcher requires boundary loops to stay
/// simple: a face that meets the region at a bare vertex — or that leaves
/// a vertex with two boundary gaps — makes the local OUTSIDE stitch
/// ambiguous. Labels `0..outer_len` are the face's boundary ring (whose
/// edges already border surviving neighbor faces); the rest are loop
/// vertices. Returns `None` when no safe order exists.
fn order_fragments(outer_len: u32, fragments: &[[u32; 3]]) -> Option<Vec<usize>> {
    let key = |a: u32, b: u32| (a.min(b), a.max(b));
    let total = fragments
        .iter()
        .flat_map(|f| f.iter().copied())
        .max()
        .map_or(0, |m| m as usize + 1);
    let mut present: Vec<bool> = alloc::vec![false; total.max(outer_len as usize)];
    for slot in present.iter_mut().take(outer_len as usize) {
        *slot = true;
    }
    // Remaining face capacity per undirected edge: interior edges take
    // two fragments; the boundary ring's edges already have their outside
    // neighbor, so they take one.
    let mut capacity: HashMap<(u32, u32), u8> = HashMap::new();
    for i in 0..outer_len {
        capacity.insert(key(i, (i + 1) % outer_len), 1);
    }

    let mut order = Vec::with_capacity(fragments.len());
    let mut added = alloc::vec![false; fragments.len()];
    loop {
        let mut progressed = false;
        for (index, fragment) in fragments.iter().enumerate() {
            if added[index] {
                continue;
            }
            let open = |a: u32, b: u32, capacity: &HashMap<(u32, u32), u8>| {
                capacity.get(&key(a, b)).copied().unwrap_or(0) > 0
            };
            let mut attaches = false;
            let mut safe = true;
            for corner in 0..3 {
                let vertex = fragment[corner];
                if !present[vertex as usize] {
                    continue;
                }
                let previous = fragment[(corner + 2) % 3];
                let next = fragment[(corner + 1) % 3];
                if open(vertex, previous, &capacity) || open(vertex, next, &capacity) {
                    attaches = true;
                } else {
                    // The fragment touches the region at a bare vertex.
                    safe = false;
                    break;
                }
            }
            if !safe || !attaches {
                continue;
            }
            added[index] = true;
            order.push(index);
            for corner in 0..3 {
                let a = fragment[corner];
                let b = fragment[(corner + 1) % 3];
                let entry = capacity.entry(key(a, b)).or_insert(2);
                *entry = entry.saturating_sub(1);
                present[a as usize] = true;
            }
            progressed = true;
        }
        if order.len() == fragments.len() {
            return Some(order);
        }
        if !progressed {
            return None;
        }
    }
}

/// Reinserts labels the triangulator dropped as exactly-collinear ring
/// vertices. Every label indexes `points`; a dropped label lies exactly on
/// the chord that replaced it. The label is reinserted only on a boundary
/// edge of this ring's fragment cover (one incident triangle), never on an
/// arbitrary interior diagonal or another aligned ring. Missing labels
/// resolve in ascending order, so chains of collinear vertices on one chord
/// reinsert incrementally. Returns `false` when a missing label lies on no
/// eligible fragment boundary edge (the caller defers, typed).
fn reinsert_dropped_labels(
    points: &[[f64; 2]],
    expected: core::ops::Range<usize>,
    fragments: &mut Vec<[u32; 3]>,
) -> bool {
    let ring_len = expected.end.saturating_sub(expected.start);
    if ring_len < 3 {
        return false;
    }
    let mut used = alloc::vec![false; points.len()];
    for fragment in fragments.iter() {
        for &label in fragment {
            if let Some(slot) = used.get_mut(label as usize) {
                *slot = true;
            }
        }
    }
    for missing in expected {
        if used[missing] {
            continue;
        }
        let Ok(label) = u32::try_from(missing) else {
            return false;
        };
        let point = points[missing];
        // Hole bridges can subdivide the chord that replaced a collinear
        // input label. Recover the label from the current triangulation
        // boundary, not from arbitrary containing triangle diagonals: the
        // ring edge has exactly one incident fragment. Several aligned
        // boundary edges can contain the point, so prefer the shortest and
        // resolve exact ties by endpoint labels.
        let mut incidence: HashMap<(u32, u32), usize> = HashMap::new();
        for fragment in fragments.iter() {
            for corner in 0..3 {
                let a = fragment[corner];
                let b = fragment[(corner + 1) % 3];
                *incidence.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let mut target: Option<(u32, u32, f64)> = None;
        for (&(a, b), &count) in &incidence {
            if count != 1 || !strictly_between(points[a as usize], points[b as usize], point) {
                continue;
            }
            let delta = [
                points[b as usize][0] - points[a as usize][0],
                points[b as usize][1] - points[a as usize][1],
            ];
            let length2 = delta[0] * delta[0] + delta[1] * delta[1];
            if target.is_none_or(|(best_a, best_b, best_length2)| {
                length2 < best_length2 || (length2 == best_length2 && (a, b) < (best_a, best_b))
            }) {
                target = Some((a, b, length2));
            }
        }
        let Some((target_a, target_b, _)) = target else {
            return false;
        };
        let mut inserted = false;
        let mut index = 0;
        while index < fragments.len() {
            let fragment = fragments[index];
            for corner in 0..3 {
                let a = fragment[corner];
                let b = fragment[(corner + 1) % 3];
                let c = fragment[(corner + 2) % 3];
                if (a.min(b), a.max(b)) != (target_a, target_b) {
                    continue;
                }
                fragments[index] = [a, label, c];
                fragments.push([label, b, c]);
                inserted = true;
                break;
            }
            index += 1;
        }
        if !inserted {
            return false;
        }
        used[missing] = true;
    }
    true
}

/// Exact test that `p` lies strictly inside segment `ab`: distinct from
/// both endpoints, exactly collinear, and inside the segment's bounding
/// box (which, given collinearity and distinctness, means strictly
/// between).
fn strictly_between(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    if p == a || p == b || orient2d(a, b, p) != Orientation::Collinear {
        return false;
    }
    let (lo0, hi0) = (a[0].min(b[0]), a[0].max(b[0]));
    let (lo1, hi1) = (a[1].min(b[1]), a[1].max(b[1]));
    lo0 <= p[0] && p[0] <= hi0 && lo1 <= p[1] && p[1] <= hi1
}

/// Signed doubled area of a projected ring (shoelace).
fn projected_area2(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum
}

/// Classifies a point against a projected polygon, returning `None` when it
/// lies on the boundary.
///
/// Mixed open and closed cuts should never touch without sharing graph
/// vertices. Treating boundary contact as ambiguous keeps that invariant
/// explicit instead of assigning a hole to whichever partition happens to
/// be visited first.
fn point_in_projected_loop(point: [f64; 2], polygon: &[[f64; 2]]) -> Option<bool> {
    let mut winding = 0_i32;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        let orientation = orient2d(a, b, point);
        let within_x = point[0] >= a[0].min(b[0]) && point[0] <= a[0].max(b[0]);
        let within_y = point[1] >= a[1].min(b[1]) && point[1] <= a[1].max(b[1]);
        if orientation == Orientation::Collinear && within_x && within_y {
            return None;
        }
        // The half-open y tests count a vertex once, and the adaptive exact
        // predicate decides which side of the edge the point occupies. This
        // avoids a division and keeps region ownership stable across scale.
        if a[1] <= point[1] && b[1] > point[1] && orientation == Orientation::Ccw {
            winding += 1;
        } else if a[1] > point[1] && b[1] <= point[1] && orientation == Orientation::Cw {
            winding -= 1;
        }
    }
    Some(winding != 0)
}

/// Re-faces open-chain partitions that also contain closed interior loops.
///
/// Open chains first divide the original face into simple polygonal regions.
/// Each closed loop is then assigned geometrically to exactly one region and
/// triangulated twice: once as a hole in that region and once as the disk on
/// the other side of the cut. Planning and insertion ordering complete before
/// the mesh is mutated, preserving all-or-defer for typed geometric failures.
/// A later mesh-edit rejection is an internal invariant violation, as in the
/// existing pure-loop re-facer, and is reported as such rather than disguised
/// as an unsupported input.
#[expect(
    clippy::too_many_arguments,
    reason = "the split transaction is explicit"
)]
fn split_face_with_partitioned_interior_loops(
    mesh: &mut Mesh,
    graph: &IntersectionGraph,
    face: FaceId,
    boundary: &[FacePartitionVertex],
    regions: &[Vec<FacePartitionVertex>],
    loops: &[Vec<u32>],
    outcome: &mut MeshSplitOutcome,
    diagnostics: &mut BooleanDiagnostics,
) {
    let position = |vertex: FacePartitionVertex| -> Option<[f64; 3]> {
        match vertex {
            FacePartitionVertex::Mesh(vertex) => mesh.vertex_position(vertex).copied().map(promote),
            FacePartitionVertex::Graph(vertex) => {
                graph.vertices.get(vertex as usize).map(|v| v.position)
            }
        }
    };
    let boundary3: Vec<[f64; 3]> = boundary.iter().copied().filter_map(position).collect();
    if boundary3.len() != boundary.len() || boundary3.len() < 3 {
        defer(
            "mixed-cut face has an unresolvable boundary loop",
            outcome,
            diagnostics,
        );
        return;
    }
    let mut normal = [0.0_f64; 3];
    for index in 0..boundary3.len() {
        let a = boundary3[index];
        let b = boundary3[(index + 1) % boundary3.len()];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let axis = dominant_axis(normal);
    if normal[axis] == 0.0 || !normal[axis].is_finite() {
        defer(
            "mixed-cut face has a degenerate projection plane",
            outcome,
            diagnostics,
        );
        return;
    }
    let (u, v) = if normal[axis] > 0.0 {
        ((axis + 1) % 3, (axis + 2) % 3)
    } else {
        ((axis + 2) % 3, (axis + 1) % 3)
    };
    let project = |point: [f64; 3]| [point[u], point[v]];

    let mut region_symbols = regions.to_vec();
    let mut region_points = Vec::with_capacity(regions.len());
    for symbols in &mut region_symbols {
        let Some(mut points) = symbols
            .iter()
            .copied()
            .map(|vertex| position(vertex).map(project))
            .collect::<Option<Vec<_>>>()
        else {
            defer(
                "mixed-cut partition has an unresolvable vertex",
                outcome,
                diagnostics,
            );
            return;
        };
        if projected_area2(&points) < 0.0 {
            points.reverse();
            symbols.reverse();
        }
        region_points.push(points);
    }

    let mut loop_symbols: Vec<Vec<FacePartitionVertex>> = loops
        .iter()
        .map(|ring| {
            ring.iter()
                .copied()
                .map(FacePartitionVertex::Graph)
                .collect()
        })
        .collect();
    let mut loop_points: Vec<Vec<[f64; 2]>> = loops
        .iter()
        .map(|ring| {
            ring.iter()
                .map(|&vertex| project(graph.vertices[vertex as usize].position))
                .collect()
        })
        .collect();
    for (symbols, points) in loop_symbols.iter_mut().zip(&mut loop_points) {
        // Hole rings wind opposite the counter-clockwise region boundary.
        if projected_area2(points) > 0.0 {
            points.reverse();
            symbols.reverse();
        }
    }

    let mut assigned: Vec<Vec<usize>> = alloc::vec![Vec::new(); regions.len()];
    for (loop_index, points) in loop_points.iter().enumerate() {
        let owners: Vec<usize> = region_points
            .iter()
            .enumerate()
            .filter_map(|(region_index, region)| {
                points
                    .iter()
                    .all(|&point| point_in_projected_loop(point, region) == Some(true))
                    .then_some(region_index)
            })
            .collect();
        let [owner] = owners.as_slice() else {
            defer(
                "interior cut loop does not belong to exactly one face partition",
                outcome,
                diagnostics,
            );
            return;
        };
        assigned[*owner].push(loop_index);
    }

    let params = exedra_triangulate::TriParams::default();
    let mut fragments: Vec<Vec<FacePartitionVertex>> = Vec::new();
    for region_index in 0..region_symbols.len() {
        let holes: Vec<&[[f64; 2]]> = assigned[region_index]
            .iter()
            .map(|&loop_index| loop_points[loop_index].as_slice())
            .collect();
        let input = exedra_triangulate::PolygonInput {
            outer: &region_points[region_index],
            holes: &holes,
        };
        let Ok(triangulation) = exedra_triangulate::triangulate(&input, &params) else {
            defer(
                "mixed-cut partition failed hole triangulation",
                outcome,
                diagnostics,
            );
            return;
        };
        let mut labels = region_symbols[region_index].clone();
        let mut points = region_points[region_index].clone();
        for &loop_index in &assigned[region_index] {
            labels.extend(loop_symbols[loop_index].iter().copied());
            points.extend(loop_points[loop_index].iter().copied());
        }
        let mut triangles = triangulation.triangles;
        let outer_len = region_symbols[region_index].len();
        if !reinsert_dropped_labels(&points, 0..outer_len, &mut triangles) {
            defer(
                "mixed-cut partition lost a collinear boundary vertex",
                outcome,
                diagnostics,
            );
            return;
        }
        let mut base = outer_len;
        for &loop_index in &assigned[region_index] {
            let end = base + loop_symbols[loop_index].len();
            if !reinsert_dropped_labels(&points, base..end, &mut triangles) {
                defer(
                    "mixed-cut partition lost a collinear loop vertex",
                    outcome,
                    diagnostics,
                );
                return;
            }
            base = end;
        }
        fragments.extend(triangles.into_iter().map(|triangle| {
            triangle
                .into_iter()
                .map(|label| labels[label as usize])
                .collect()
        }));

        // The disk is required as well as the ring: classification needs a
        // real face on each side of every cut-loop edge.
        for &loop_index in &assigned[region_index] {
            let disk_points: Vec<[f64; 2]> =
                loop_points[loop_index].iter().rev().copied().collect();
            let disk_symbols: Vec<FacePartitionVertex> =
                loop_symbols[loop_index].iter().rev().copied().collect();
            let disk_input = exedra_triangulate::PolygonInput {
                outer: &disk_points,
                holes: &[],
            };
            let Ok(disk) = exedra_triangulate::triangulate(&disk_input, &params) else {
                defer(
                    "mixed-cut loop disk failed triangulation",
                    outcome,
                    diagnostics,
                );
                return;
            };
            let mut triangles = disk.triangles;
            if !reinsert_dropped_labels(&disk_points, 0..disk_points.len(), &mut triangles) {
                defer(
                    "mixed-cut loop disk lost a collinear vertex",
                    outcome,
                    diagnostics,
                );
                return;
            }
            fragments.extend(triangles.into_iter().map(|triangle| {
                triangle
                    .into_iter()
                    .map(|label| disk_symbols[label as usize])
                    .collect()
            }));
        }
    }

    if fragments.iter().any(|fragment| !is_simple_loop(fragment)) {
        defer(
            "mixed-cut triangulation produced a degenerate fragment",
            outcome,
            diagnostics,
        );
        return;
    }
    // Mixed regions are triangulated, so use the stricter fragment scheduler
    // used by drilled faces. Original boundary labels come first; every
    // chain and loop vertex follows in stable first-seen order.
    let mut labels = boundary.to_vec();
    let fragment_labels: Vec<[u32; 3]> = fragments
        .iter()
        .map(|fragment| {
            core::array::from_fn(|corner| {
                let vertex = fragment[corner];
                let index = if let Some(index) = labels.iter().position(|&item| item == vertex) {
                    index
                } else {
                    labels.push(vertex);
                    labels.len() - 1
                };
                u32::try_from(index).unwrap_or(u32::MAX)
            })
        })
        .collect();
    let Some(order) = order_fragments(
        u32::try_from(boundary.len()).unwrap_or(u32::MAX),
        &fragment_labels,
    ) else {
        defer(
            "mixed-cut fragments admit no safe insertion order",
            outcome,
            diagnostics,
        );
        return;
    };

    let region = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face.as_id()).copied());
    let boundary_attrs: Vec<(VertexId, VertexId, Option<f32>, Option<bool>)> = {
        let mut captured = Vec::new();
        for half_edge in mesh.face_loop(face) {
            let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
            else {
                continue;
            };
            let sharpness = mesh.edge_sharpness(half_edge).filter(|value| *value != 0.0);
            let seam = mesh.edge_seam(half_edge).filter(|value| *value);
            if sharpness.is_some() || seam.is_some() {
                captured.push((from, to, sharpness, seam));
            }
        }
        captured
    };

    let mut materialized = fragments;
    let mut session = mesh.edit();
    for fragment in &mut materialized {
        for vertex in fragment {
            if let FacePartitionVertex::Graph(graph_vertex) = *vertex {
                let mesh_vertex =
                    if let Some(existing) = outcome.graph_vertices[graph_vertex as usize] {
                        existing
                    } else {
                        let created = add_vertex(
                            &mut session,
                            narrow(graph.vertices[graph_vertex as usize].position),
                        );
                        outcome.graph_vertices[graph_vertex as usize] = Some(created);
                        outcome.stats.interior_vertices += 1;
                        created
                    };
                *vertex = FacePartitionVertex::Mesh(mesh_vertex);
            }
        }
    }
    if delete_faces(&mut session, &[face], DeletePolicy::KeepIsolated).is_err() {
        diagnostics.push(BooleanDiagnostic {
            kind: BooleanFailureKind::InternalInvariantViolation,
            a: None,
            b: None,
            detail: "mixed-cut face could not be deleted for re-facing",
        });
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
        return;
    }
    let mut created = 0_u64;
    for index in order {
        let vertices: Vec<VertexId> = materialized[index]
            .iter()
            .map(|vertex| match vertex {
                FacePartitionVertex::Mesh(vertex) => *vertex,
                FacePartitionVertex::Graph(_) => unreachable!("materialized above"),
            })
            .collect();
        match add_face(&mut session, &vertices) {
            Ok(new_face) => {
                outcome.face_origins.push((new_face, face));
                if let Some(region) = region {
                    let _ = set_face_region(&mut session, new_face, region);
                }
                created += 1;
            }
            Err(_) => diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::InternalInvariantViolation,
                a: None,
                b: None,
                detail: "mixed-cut fragment was rejected by add_face",
            }),
        }
    }
    for (from, to, sharpness, seam) in boundary_attrs {
        if let Some(half_edge) = session.find_half_edge(from, to) {
            if let Some(sharpness) = sharpness {
                let _ = set_edge_sharpness(&mut session, half_edge, sharpness);
            }
            if let Some(seam) = seam {
                let _ = set_edge_seam(&mut session, half_edge, seam);
            }
        }
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    outcome.stats.faces_split += 1;
    outcome.stats.faces_created += created;
}

/// Re-faces a face crossed only by closed interior loops — the
/// through-hole configuration. The face becomes a triangulated ring (its
/// original boundary with every loop as a hole, via the shared
/// deterministic triangulator's hole bridging) plus a triangulated disk
/// per loop. Afterwards every cut-loop edge is a real mesh edge shared by
/// a ring triangle and a disk triangle, so patch classification separates
/// the two regions without any special casing. All fragments keep the
/// original face's region, map to it in the origin table, and re-apply
/// its captured boundary-edge attributes.
///
/// Everything is triangulated before the first mesh mutation: a
/// degenerate projection or triangulation defers with the face untouched.
fn split_face_with_interior_loops(
    mesh: &mut Mesh,
    graph: &IntersectionGraph,
    face: FaceId,
    local: &HashMap<u32, Vec<u32>>,
    outcome: &mut MeshSplitOutcome,
    diagnostics: &mut BooleanDiagnostics,
) {
    // Trace the loops deterministically: the lowest unconsumed vertex
    // starts a loop, stepping first to its lowest-index neighbor. Every
    // vertex is degree 2 here: the caller routes only pure closed-loop
    // components to this path, so each loop is a simple cycle.
    let mut loop_starts: Vec<u32> = local.keys().copied().collect();
    loop_starts.sort_unstable();
    let mut visited: HashMap<(u32, u32), bool> = HashMap::new();
    let mut loops: Vec<Vec<u32>> = Vec::new();
    for &start in &loop_starts {
        let next = local[&start]
            .iter()
            .copied()
            .find(|&n| n != start && !visited.contains_key(&(start.min(n), start.max(n))));
        let Some(mut current) = next else {
            continue;
        };
        let mut ring = alloc::vec![start];
        let mut previous = start;
        visited.insert((start.min(current), start.max(current)), true);
        while current != start {
            ring.push(current);
            let Some(&step) = local[&current]
                .iter()
                .find(|&&candidate| candidate != previous)
            else {
                defer(
                    "interior cut loop is degenerate (duplicate edge)",
                    outcome,
                    diagnostics,
                );
                return;
            };
            visited.insert((current.min(step), current.max(step)), true);
            previous = current;
            current = step;
        }
        if ring.len() < 3 {
            defer(
                "interior cut loop has fewer than three vertices",
                outcome,
                diagnostics,
            );
            return;
        }
        loops.push(ring);
    }
    if loops.is_empty() {
        return;
    }

    // Face boundary and its Newell normal, promoted to f64.
    let loop_vertices: Vec<VertexId> = mesh
        .face_loop(face)
        .filter_map(|he| mesh.to_vertex(he))
        .collect();
    let outer3: Vec<[f64; 3]> = loop_vertices
        .iter()
        .filter_map(|&v| mesh.vertex_position(v))
        .copied()
        .map(promote)
        .collect();
    if outer3.len() != loop_vertices.len() || outer3.len() < 3 {
        defer(
            "drilled face has an unresolvable boundary loop",
            outcome,
            diagnostics,
        );
        return;
    }
    let mut normal = [0.0_f64; 3];
    for i in 0..outer3.len() {
        let a = outer3[i];
        let b = outer3[(i + 1) % outer3.len()];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let axis = dominant_axis(normal);
    if normal[axis] == 0.0 || !normal[axis].is_finite() {
        defer(
            "drilled face has a degenerate projection plane",
            outcome,
            diagnostics,
        );
        return;
    }
    // Project onto the plane axes in cyclic order; a negative dominant
    // component means the cyclic projection winds clockwise, so swap the
    // axes to hand the triangulator counter-clockwise polygons whose
    // mirrored output winds consistently with the face loop (the same
    // idiom as `Mesh::triangulate_face_robust`).
    let (u, v) = if normal[axis] > 0.0 {
        ((axis + 1) % 3, (axis + 2) % 3)
    } else {
        ((axis + 2) % 3, (axis + 1) % 3)
    };
    let project = |p: [f64; 3]| [p[u], p[v]];
    let outer_projected: Vec<[f64; 2]> = outer3.iter().map(|&p| project(p)).collect();

    // Hole rings must wind opposite the outer loop in the projected
    // frame: reverse any loop that projects counter-clockwise. Vertex
    // orders travel with their points so index mapping stays aligned.
    let mut hole_projected: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut hole_indices: Vec<Vec<u32>> = Vec::new();
    for ring in &loops {
        let mut points: Vec<[f64; 2]> = ring
            .iter()
            .map(|&index| project(graph.vertices[index as usize].position))
            .collect();
        let mut indices = ring.clone();
        if projected_area2(&points) > 0.0 {
            points.reverse();
            indices.reverse();
        }
        hole_projected.push(points);
        hole_indices.push(indices);
    }

    // Triangulate the ring and every disk BEFORE mutating the mesh, so a
    // typed failure leaves the face untouched.
    let params = exedra_triangulate::TriParams::default();
    let holes: Vec<&[[f64; 2]]> = hole_projected.iter().map(Vec::as_slice).collect();
    let ring_input = exedra_triangulate::PolygonInput {
        outer: &outer_projected,
        holes: &holes,
    };
    let Ok(ring_triangles) = exedra_triangulate::triangulate(&ring_input, &params) else {
        defer(
            "drilled face ring failed hole triangulation",
            outcome,
            diagnostics,
        );
        return;
    };
    let mut disk_triangles: Vec<exedra_triangulate::Triangulation> = Vec::new();
    for points in &hole_projected {
        // The disk fills the hole: its outer ring is the hole reversed
        // (counter-clockwise in the projected frame).
        let disk_points: Vec<[f64; 2]> = points.iter().rev().copied().collect();
        let disk_input = exedra_triangulate::PolygonInput {
            outer: &disk_points,
            holes: &[],
        };
        let Ok(triangles) = exedra_triangulate::triangulate(&disk_input, &params) else {
            defer(
                "drilled face disk failed triangulation",
                outcome,
                diagnostics,
            );
            return;
        };
        disk_triangles.push(triangles);
    }

    // Assemble every fragment in the outer ++ holes label space (ring
    // triangles already index it; disk triangles index the reversed hole
    // ring) and find an insertion order the incremental stitcher accepts
    // — still before any mutation.
    let outer_len = u32::try_from(outer_projected.len()).unwrap_or(u32::MAX);
    let label_points: Vec<[f64; 2]> = outer_projected
        .iter()
        .chain(hole_projected.iter().flatten())
        .copied()
        .collect();

    // The ear clipper removes exactly-collinear ring vertices (the covered
    // area is unchanged), but every label here is a real mesh vertex
    // shared with neighboring faces — and the ring and each disk drop
    // *different* vertices along the hole rings they share, leaving
    // T-junctions both against the rest of the mesh and against each
    // other. Reinsert each side's missing labels onto the fragment edges
    // they lie on (exact collinearity + strict betweenness), splitting
    // those fragments in place, before combining the two sides.
    let mut fragment_labels: Vec<[u32; 3]> = ring_triangles.triangles.clone();
    if !reinsert_dropped_labels(&label_points, 0..outer_len as usize, &mut fragment_labels) {
        defer(
            "drilled face ring lost a collinear cut vertex the fragments cannot host",
            outcome,
            diagnostics,
        );
        return;
    }
    let mut ring_base = outer_len;
    for points in &hole_projected {
        let count = u32::try_from(points.len()).unwrap_or(u32::MAX);
        if !reinsert_dropped_labels(
            &label_points,
            ring_base as usize..(ring_base + count) as usize,
            &mut fragment_labels,
        ) {
            defer(
                "drilled face ring lost a collinear cut vertex the fragments cannot host",
                outcome,
                diagnostics,
            );
            return;
        }
        ring_base += count;
    }
    let mut base = outer_len;
    for (points, triangles) in hole_projected.iter().zip(&disk_triangles) {
        let count = u32::try_from(points.len()).unwrap_or(u32::MAX);
        let mut disk_labels: Vec<[u32; 3]> = triangles
            .triangles
            .iter()
            .map(|t| {
                [
                    base + (count - 1 - t[0]),
                    base + (count - 1 - t[1]),
                    base + (count - 1 - t[2]),
                ]
            })
            .collect();
        let expected = base as usize..(base + count) as usize;
        if !reinsert_dropped_labels(&label_points, expected, &mut disk_labels) {
            defer(
                "drilled face disk lost a collinear cut vertex the fragments cannot host",
                outcome,
                diagnostics,
            );
            return;
        }
        fragment_labels.extend(disk_labels);
        base += count;
    }

    let Some(order) = order_fragments(outer_len, &fragment_labels) else {
        defer(
            "drilled face fragments admit no safe insertion order",
            outcome,
            diagnostics,
        );
        return;
    };

    // Capture attributes with the same discipline as the chain path.
    let region = mesh
        .attrs()
        .dense(attr::FACE_REGION)
        .and_then(|layer| layer.get(face.as_id()).copied());
    let boundary_attrs: Vec<(VertexId, VertexId, Option<f32>, Option<bool>)> = {
        let mut captured = Vec::new();
        let loop_edges: Vec<crate::HalfEdgeId> = mesh.face_loop(face).collect();
        for half_edge in loop_edges {
            let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
            else {
                continue;
            };
            let sharpness = mesh.edge_sharpness(half_edge).filter(|s| *s != 0.0);
            let seam = mesh.edge_seam(half_edge).filter(|s| *s);
            if sharpness.is_some() || seam.is_some() {
                captured.push((from, to, sharpness, seam));
            }
        }
        captured
    };

    // Materialize loop vertices, delete the face, add the fragments.
    let mut session = mesh.edit();
    for indices in &hole_indices {
        for &index in indices {
            if outcome.graph_vertices[index as usize].is_none() {
                let position = narrow(graph.vertices[index as usize].position);
                let vertex = add_vertex(&mut session, position);
                outcome.graph_vertices[index as usize] = Some(vertex);
                outcome.stats.interior_vertices += 1;
            }
        }
    }
    // Triangulator indices address the outer ++ holes concatenation.
    let mut index_map: Vec<VertexId> = loop_vertices;
    for indices in &hole_indices {
        for &index in indices {
            index_map
                .push(outcome.graph_vertices[index as usize].expect("materialized just above"));
        }
    }
    if delete_faces(&mut session, &[face], DeletePolicy::KeepIsolated).is_err() {
        diagnostics.push(BooleanDiagnostic {
            kind: BooleanFailureKind::InternalInvariantViolation,
            a: None,
            b: None,
            detail: "drilled face could not be deleted for re-facing",
        });
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
        return;
    }
    let mut created = 0_u64;
    for &fragment_index in &order {
        let fragment = fragment_labels[fragment_index]
            .map(|label| index_map[usize::try_from(label).unwrap_or(usize::MAX)]);
        match add_face(&mut session, &fragment) {
            Ok(new_face) => {
                outcome.face_origins.push((new_face, face));
                if let Some(region) = region {
                    let _ = set_face_region(&mut session, new_face, region);
                }
                created += 1;
            }
            Err(_) => {
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::InternalInvariantViolation,
                    a: None,
                    b: None,
                    detail: "drilled-face fragment was rejected by add_face",
                });
            }
        }
    }
    // Re-apply captured boundary edge attributes where the edges survive.
    for (from, to, sharpness, seam) in boundary_attrs {
        if let Some(half_edge) = session.find_half_edge(from, to) {
            if let Some(sharpness) = sharpness {
                let _ = set_edge_sharpness(&mut session, half_edge, sharpness);
            }
            if let Some(seam) = seam {
                let _ = set_edge_seam(&mut session, half_edge, seam);
            }
        }
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    outcome.stats.faces_split += 1;
    outcome.stats.faces_created += created;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::graph::{GraphEdge, GraphVertex};
    use crate::boolean::{
        BooleanBvh, BooleanDiagnostics, BooleanScratch, IntersectionGraph,
        build_intersection_graph, narrow_phase,
    };
    use crate::{FaceTriangulation, MeshBuilder};

    /// A collinear vertex dropped by triangulation returns on the fragment
    /// boundary chord that replaced its original ring edges.
    #[test]
    fn reinsert_recovers_dropped_collinear_label() {
        // Square with a collinear midpoint (label 1) on its bottom edge;
        // the triangulation below skipped it.
        let points = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let mut fragments = alloc::vec![[0_u32, 2, 3], [0, 3, 4]];
        assert!(reinsert_dropped_labels(
            &points,
            0..points.len(),
            &mut fragments
        ));
        assert_eq!(fragments, alloc::vec![[0, 1, 3], [0, 3, 4], [1, 2, 3]]);
        // A label on no edge is a typed refusal, not a silent gap.
        let stray = [[0.0, 0.0], [5.0, 5.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let mut fragments = alloc::vec![[0_u32, 2, 3], [0, 3, 4]];
        assert!(!reinsert_dropped_labels(
            &stray,
            0..stray.len(),
            &mut fragments
        ));
    }

    #[test]
    fn deferred_crossing_chains_do_not_materialize_partition_vertices() {
        // The first diagonal can split this quad and contains one interior
        // graph vertex. The second diagonal then spans the two new sub-loops,
        // forcing a typed ambiguity. Planning must stay symbolic so that the
        // rejected batch leaves neither the center vertex nor a revision bump.
        let mut builder = MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ] {
            builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2, 3]).expect("valid quad");
        let mut mesh = builder.build().expect("valid open quad").mesh;
        let face = mesh.faces().next().expect("quad face");
        let boundary: Vec<VertexId> = mesh
            .face_loop(face)
            .filter_map(|half_edge| mesh.to_vertex(half_edge))
            .collect();
        let [a, b, c, d] = boundary.as_slice() else {
            panic!("quad must have four boundary vertices");
        };
        let vertex = |mesh_vertex: VertexId| GraphVertex {
            position: promote(
                *mesh
                    .vertex_position(mesh_vertex)
                    .expect("boundary vertex is live"),
            ),
            anchor_a: MeshAnchor::Vertex(mesh_vertex),
            anchor_b: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            faces_a: alloc::vec![face],
            faces_b: Vec::new(),
        };
        // Graph order makes a-center-c the first planned chain, followed by
        // b-d. Edge endpoint arrays retain the graph's canonical order.
        let graph = IntersectionGraph {
            vertices: alloc::vec![
                vertex(*a),
                vertex(*c),
                vertex(*b),
                vertex(*d),
                GraphVertex {
                    position: [1.0, 1.0, 0.0],
                    anchor_a: MeshAnchor::FaceInterior(face),
                    anchor_b: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
                    faces_a: alloc::vec![face],
                    faces_b: Vec::new(),
                },
            ],
            edges: alloc::vec![
                GraphEdge {
                    vertices: [0, 4],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [1, 4],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [2, 3],
                    crossings: Vec::new(),
                },
            ],
            ..IntersectionGraph::default()
        };
        let mut outcome = MeshSplitOutcome {
            graph_vertices: alloc::vec![Some(*a), Some(*c), Some(*b), Some(*d), None],
            ..MeshSplitOutcome::default()
        };
        let baseline_vertices = mesh.vertices().count();
        let baseline_revision = mesh.revision();
        let mut diagnostics = BooleanDiagnostics::default();

        split_one_face(
            &mut mesh,
            &graph,
            face,
            &[0, 1, 2],
            &mut outcome,
            &mut diagnostics,
        );

        assert_eq!(outcome.stats.deferred_faces, 1);
        assert_eq!(outcome.stats.interior_vertices, 0);
        assert_eq!(outcome.graph_vertices[4], None);
        assert_eq!(mesh.vertices().count(), baseline_vertices);
        assert_eq!(mesh.revision(), baseline_revision);
        assert_eq!(mesh.faces().collect::<Vec<_>>(), alloc::vec![face]);
        assert!(mesh.validate_deep().is_empty());
        assert!(diagnostics.entries().iter().any(|diagnostic| {
            diagnostic.kind == BooleanFailureKind::SplitDeferred
                && diagnostic.detail == "chain endpoints span different sub-loops (ambiguous cut)"
        }));
    }

    #[test]
    fn open_partition_and_interior_loop_reface_together() {
        // A shoulder cut can cross a timber face while blind housings make
        // closed loops in both resulting regions. Every seam must become a
        // mesh edge in one atomic re-face; cutter order must not defer either
        // housing merely because the shoulder chain was traced first.
        let mut mesh = slab();
        let face = mesh
            .faces()
            .find(|&face| {
                mesh.face_loop(face).all(|half_edge| {
                    mesh.to_vertex(half_edge)
                        .and_then(|vertex| mesh.vertex_position(vertex))
                        .is_some_and(|point| point[2] == 0.5)
                })
            })
            .expect("slab top face");
        let tagged_edge = mesh.face_loop(face).next().expect("top face has edges");
        let tagged_endpoints = (
            mesh.from_vertex(tagged_edge).expect("edge start"),
            mesh.to_vertex(tagged_edge).expect("edge end"),
        );
        {
            let mut session = mesh.edit();
            set_face_region(&mut session, face, 37).expect("tag source face");
            set_edge_sharpness(&mut session, tagged_edge, 2.5).expect("tag source edge");
            set_edge_seam(&mut session, tagged_edge, true).expect("mark source seam");
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
        }
        let vertex_at = |position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture corner exists")
        };
        let a = vertex_at([-2.0, -2.0, 0.5]);
        let c = vertex_at([2.0, 2.0, 0.5]);
        let boundary_vertex = |mesh_vertex: VertexId| GraphVertex {
            position: promote(
                *mesh
                    .vertex_position(mesh_vertex)
                    .expect("boundary vertex is live"),
            ),
            anchor_a: MeshAnchor::Vertex(mesh_vertex),
            anchor_b: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            faces_a: alloc::vec![face],
            faces_b: Vec::new(),
        };
        let interior_vertex = |position| GraphVertex {
            position,
            anchor_a: MeshAnchor::FaceInterior(face),
            anchor_b: MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            faces_a: alloc::vec![face],
            faces_b: Vec::new(),
        };
        let graph = IntersectionGraph {
            vertices: alloc::vec![
                boundary_vertex(a),
                boundary_vertex(c),
                interior_vertex([0.5, -1.25, 0.5]),
                interior_vertex([1.25, -1.25, 0.5]),
                interior_vertex([1.25, -0.5, 0.5]),
                interior_vertex([0.5, -0.5, 0.5]),
                interior_vertex([-1.25, 0.5, 0.5]),
                interior_vertex([-0.5, 0.5, 0.5]),
                interior_vertex([-0.5, 1.25, 0.5]),
                interior_vertex([-1.25, 1.25, 0.5]),
            ],
            edges: alloc::vec![
                GraphEdge {
                    vertices: [0, 1],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [2, 3],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [3, 4],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [4, 5],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [2, 5],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [6, 7],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [7, 8],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [8, 9],
                    crossings: Vec::new(),
                },
                GraphEdge {
                    vertices: [6, 9],
                    crossings: Vec::new(),
                },
            ],
            ..IntersectionGraph::default()
        };
        let mut outcome = MeshSplitOutcome {
            graph_vertices: alloc::vec![
                Some(a),
                Some(c),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            ..MeshSplitOutcome::default()
        };
        let mut diagnostics = BooleanDiagnostics::default();

        split_one_face(
            &mut mesh,
            &graph,
            face,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            &mut outcome,
            &mut diagnostics,
        );

        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert_eq!(outcome.stats.deferred_faces, 0);
        assert_eq!(outcome.stats.faces_split, 1);
        assert!(mesh.validate_deep().is_empty());
        let descendants: Vec<FaceId> = outcome
            .face_origins
            .iter()
            .filter_map(|&(created, original)| (original == face).then_some(created))
            .collect();
        assert_eq!(
            descendants.len() as u64,
            outcome.stats.faces_created,
            "every mixed-cut fragment retains its source face"
        );
        for descendant in descendants {
            assert_eq!(
                mesh.attrs()
                    .dense(attr::FACE_REGION)
                    .and_then(|regions| regions.get(descendant.as_id()))
                    .copied(),
                Some(37),
                "source face region propagates to every fragment"
            );
        }
        let tagged_edge = find_half_edge(&mesh, tagged_endpoints.0, tagged_endpoints.1)
            .or_else(|| find_half_edge(&mesh, tagged_endpoints.1, tagged_endpoints.0))
            .expect("tagged boundary edge survives re-facing");
        assert_eq!(mesh.edge_sharpness(tagged_edge), Some(2.5));
        assert_eq!(mesh.edge_seam(tagged_edge), Some(true));
        for edge in &graph.edges {
            let [from, to] = edge.vertices.map(|index| {
                outcome.graph_vertices[index as usize].expect("cut vertex materialized")
            });
            assert!(
                find_half_edge(&mesh, from, to).is_some()
                    || find_half_edge(&mesh, to, from).is_some(),
                "every open-chain and loop segment becomes a mesh edge"
            );
        }
    }

    #[test]
    fn projected_loop_ownership_handles_winding_scale_and_boundary_points() {
        // Region assignment must not depend on polygon winding or model
        // scale, and a loop touching a partition seam must remain ambiguous
        // instead of being assigned to whichever region is visited first.
        for scale in [1.0e-9, 1.0, 1.0e9] {
            let mut square = alloc::vec![[0.0, 0.0], [scale, 0.0], [scale, scale], [0.0, scale],];
            let inside = [scale * 0.25, scale * 0.75];
            let outside = [scale * 1.25, scale * 0.75];
            let boundary = [scale * 0.5, 0.0];
            assert_eq!(point_in_projected_loop(inside, &square), Some(true));
            assert_eq!(point_in_projected_loop(outside, &square), Some(false));
            assert_eq!(point_in_projected_loop(boundary, &square), None);

            square.reverse();
            assert_eq!(point_in_projected_loop(inside, &square), Some(true));
            assert_eq!(point_in_projected_loop(outside, &square), Some(false));
            assert_eq!(point_in_projected_loop(boundary, &square), None);
        }
    }

    fn cube(origin: [f32; 3]) -> Mesh {
        let o = origin;
        let positions = [
            [o[0], o[1], o[2]],
            [o[0] + 1.0, o[1], o[2]],
            [o[0] + 1.0, o[1] + 1.0, o[2]],
            [o[0], o[1] + 1.0, o[2]],
            [o[0], o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1], o[2] + 1.0],
            [o[0] + 1.0, o[1] + 1.0, o[2] + 1.0],
            [o[0], o[1] + 1.0, o[2] + 1.0],
        ];
        let faces: [[u32; 4]; 6] = [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(&face).expect("valid cube face");
        }
        builder.build().expect("valid cube").mesh
    }

    /// A 4 x 4 x 1 box centered at the origin: the drill-through target.
    fn slab() -> Mesh {
        let positions = [
            [-2.0, -2.0, -0.5],
            [2.0, -2.0, -0.5],
            [2.0, 2.0, -0.5],
            [-2.0, 2.0, -0.5],
            [-2.0, -2.0, 0.5],
            [2.0, -2.0, 0.5],
            [2.0, 2.0, 0.5],
            [-2.0, 2.0, 0.5],
        ];
        let faces: [[u32; 4]; 6] = [
            [3, 2, 1, 0],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ];
        let mut builder = MeshBuilder::new();
        for p in positions {
            builder.push_vertex(p);
        }
        for face in faces {
            builder.add_face(&face).expect("valid slab face");
        }
        builder.build().expect("valid slab").mesh
    }

    /// A regular 16-gon prism, radius 0.8, z in [-1.5, 1.5]: pierces the
    /// slab completely so each slab cap sees a closed interior loop.
    fn drill_prism() -> Mesh {
        let n = 16_u32;
        let mut builder = MeshBuilder::new();
        for z in [-1.5_f64, 1.5] {
            for i in 0..n {
                let angle = core::f64::consts::TAU * f64::from(i) / f64::from(n);
                let position = [0.8 * angle.cos(), 0.8 * angle.sin(), z];
                builder.push_vertex(narrow(position));
            }
        }
        let bottom: Vec<u32> = (0..n).rev().collect();
        builder.add_face(&bottom).expect("bottom cap");
        let top: Vec<u32> = (n..2 * n).collect();
        builder.add_face(&top).expect("top cap");
        for i in 0..n {
            let j = (i + 1) % n;
            builder.add_face(&[i, j, n + j, n + i]).expect("side wall");
        }
        builder.build().expect("valid prism").mesh
    }

    struct EndToEnd {
        mesh_a: Mesh,
        mesh_b: Mesh,
        graph: IntersectionGraph,
        outcome_a: MeshSplitOutcome,
        outcome_b: MeshSplitOutcome,
        diagnostics: BooleanDiagnostics,
    }

    fn run_pair(mut mesh_a: Mesh, mut mesh_b: Mesh) -> EndToEnd {
        let mut scratch = BooleanScratch::new();
        let strategy = FaceTriangulation::Fan;
        let bvh_a = BooleanBvh::build(&mesh_a, strategy, &mut scratch);
        let bvh_b = BooleanBvh::build(&mesh_b, strategy, &mut scratch);
        let mut pairs = Vec::new();
        bvh_a.query_overlaps(&bvh_b, &mut scratch, &mut pairs);
        let mut segments = Vec::new();
        let mut diagnostics = BooleanDiagnostics::default();
        narrow_phase(
            &mesh_a,
            &mesh_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut segments,
            &mut diagnostics,
        );
        let graph = build_intersection_graph(
            &mesh_a,
            &mesh_b,
            &segments,
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        let outcome_a = split_mesh_along_graph(&mut mesh_a, &graph, MeshSide::A, &mut diagnostics);
        let outcome_b = split_mesh_along_graph(&mut mesh_b, &graph, MeshSide::B, &mut diagnostics);
        EndToEnd {
            mesh_a,
            mesh_b,
            graph,
            outcome_a,
            outcome_b,
            diagnostics,
        }
    }

    fn run_two_cubes() -> EndToEnd {
        let mut mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, 0.5, 0.5]);
        // Distinct regions per face of A so propagation is observable.
        {
            let mut session = mesh_a.edit();
            let faces: Vec<FaceId> = session.mesh().faces().collect();
            for (index, face) in faces.into_iter().enumerate() {
                let region = u32::try_from(index).expect("small") + 10;
                set_face_region(&mut session, face, region).expect("live face");
            }
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
        }
        // A sharp edge on A's top-face boundary that survives splitting.
        {
            let mut session = mesh_a.edit();
            let top = session.mesh().faces().nth(1).expect("cube has a top face");
            let first = session
                .mesh()
                .face_loop(top)
                .next()
                .expect("top face has edges");
            set_edge_sharpness(&mut session, first, 1.0).expect("live edge");
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
        }

        run_pair(mesh_a, mesh_b)
    }

    #[test]
    fn two_cube_split_keeps_both_meshes_valid() {
        let result = run_two_cubes();
        assert!(
            result.diagnostics.is_clean(),
            "{:?}",
            result.diagnostics.entries()
        );
        let errors_a = result.mesh_a.validate_deep();
        assert!(errors_a.is_empty(), "mesh A: {errors_a:?}");
        let errors_b = result.mesh_b.validate_deep();
        assert!(errors_b.is_empty(), "mesh B: {errors_b:?}");

        // Three faces of each cube are crossed by the corner-overlap loop;
        // each splits into two.
        assert_eq!(result.outcome_a.stats.faces_split, 3);
        assert_eq!(result.outcome_a.stats.faces_created, 6);
        assert_eq!(result.outcome_b.stats.faces_split, 3);
        assert_eq!(result.outcome_a.stats.deferred_faces, 0);
        assert_eq!(result.outcome_b.stats.deferred_faces, 0);
        assert_eq!(result.mesh_a.faces().count(), 9, "6 - 3 + 6 faces");
        assert_eq!(result.mesh_b.faces().count(), 9);
    }

    #[test]
    fn curve_lies_on_mesh_edges_after_split() {
        let result = run_two_cubes();
        // Every polyline vertex materialized on both meshes at its
        // narrowed position (touch-only vertices never join cut chains and
        // are not required to materialize).
        let mut cut_vertices: Vec<u32> = result
            .graph
            .polylines
            .iter()
            .flat_map(|p| p.vertices.iter().copied())
            .collect();
        cut_vertices.sort_unstable();
        cut_vertices.dedup();
        for &index in &cut_vertices {
            let vertex = &result.graph.vertices[index as usize];
            for (mesh, resolved) in [
                (
                    &result.mesh_a,
                    result.outcome_a.graph_vertices[index as usize],
                ),
                (
                    &result.mesh_b,
                    result.outcome_b.graph_vertices[index as usize],
                ),
            ] {
                let mesh_vertex = resolved.expect("cut vertex materialized");
                let position = mesh.vertex_position(mesh_vertex).expect("live vertex");
                let expected = narrow(vertex.position);
                assert_eq!(*position, expected, "graph vertex {index}");
            }
        }
        // Every cut-loop edge is now a real mesh edge on both meshes.
        for polyline in &result.graph.polylines {
            let count = polyline.vertices.len();
            let edges = if polyline.closed { count } else { count - 1 };
            for i in 0..edges {
                let p = polyline.vertices[i] as usize;
                let q = polyline.vertices[(i + 1) % count] as usize;
                for (mesh, outcome) in [
                    (&result.mesh_a, &result.outcome_a),
                    (&result.mesh_b, &result.outcome_b),
                ] {
                    let a = outcome.graph_vertices[p].expect("materialized");
                    let b = outcome.graph_vertices[q].expect("materialized");
                    assert!(
                        find_half_edge(mesh, a, b).is_some(),
                        "cut edge {p}-{q} must be a mesh edge"
                    );
                }
            }
        }
    }

    #[test]
    fn origins_and_attributes_survive_splitting() {
        let result = run_two_cubes();
        assert!(!result.outcome_a.face_origins.is_empty());
        let regions = result
            .mesh_a
            .attrs()
            .dense(attr::FACE_REGION)
            .expect("region layer");
        let mut checked = 0;
        for &(new_face, original) in &result.outcome_a.face_origins {
            // Original cube faces got regions 10 + index.
            let original_region = 10 + original.index();
            let new_region = regions.get(new_face.as_id()).copied().expect("region");
            assert_eq!(
                new_region, original_region,
                "region must propagate from the original face"
            );
            checked += 1;
        }
        assert_eq!(checked, 6, "six new faces on mesh A");

        // The sharp boundary edge on the top face survived re-partition.
        let sharp_edges = result
            .mesh_a
            .faces()
            .flat_map(|face| result.mesh_a.face_loop(face))
            .filter(|&he| result.mesh_a.edge_sharpness(he).unwrap_or(0.0) > 0.5)
            .count();
        assert!(
            sharp_edges > 0,
            "captured boundary sharpness must be re-applied"
        );
    }

    #[test]
    fn splitting_is_deterministic() {
        let first = run_two_cubes();
        let second = run_two_cubes();
        assert_eq!(first.outcome_a, second.outcome_a);
        assert_eq!(first.outcome_b, second.outcome_b);
        // Structural snapshot: face loops as vertex indices plus vertex
        // position bits, in deterministic iteration order.
        let snapshot = |mesh: &Mesh| {
            let faces: Vec<Vec<u32>> = mesh
                .faces()
                .map(|face| {
                    mesh.face_loop(face)
                        .filter_map(|he| mesh.to_vertex(he))
                        .map(|v| v.index())
                        .collect()
                })
                .collect();
            let positions: Vec<[u32; 3]> = mesh
                .vertices()
                .filter_map(|v| mesh.vertex_position(v))
                .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
                .collect();
            (faces, positions)
        };
        assert_eq!(snapshot(&first.mesh_a), snapshot(&second.mesh_a));
        assert_eq!(snapshot(&first.mesh_b), snapshot(&second.mesh_b));
    }

    #[test]
    fn interior_loops_reface_drilled_caps() {
        let result = run_pair(slab(), drill_prism());
        // The former "closed intersection loop fully interior to one face"
        // deferral is gone: both slab caps re-face cleanly.
        assert!(
            result.diagnostics.is_clean(),
            "{:?}",
            result.diagnostics.entries()
        );
        assert_eq!(result.outcome_a.stats.deferred_faces, 0);
        assert_eq!(result.outcome_b.stats.deferred_faces, 0);
        assert_eq!(
            result.outcome_a.stats.faces_split, 2,
            "both slab caps re-face"
        );
        assert_eq!(
            result.outcome_b.stats.faces_split, 16,
            "every prism wall splits into three bands"
        );
        let errors_a = result.mesh_a.validate_deep();
        assert!(errors_a.is_empty(), "mesh A: {errors_a:?}");
        let errors_b = result.mesh_b.validate_deep();
        assert!(errors_b.is_empty(), "mesh B: {errors_b:?}");

        // Every closed cut loop lies on real mesh edges on both sides.
        let mut closed_loops = 0;
        for polyline in &result.graph.polylines {
            if !polyline.closed {
                continue;
            }
            closed_loops += 1;
            let count = polyline.vertices.len();
            for i in 0..count {
                let p = polyline.vertices[i] as usize;
                let q = polyline.vertices[(i + 1) % count] as usize;
                for (mesh, outcome) in [
                    (&result.mesh_a, &result.outcome_a),
                    (&result.mesh_b, &result.outcome_b),
                ] {
                    let a = outcome.graph_vertices[p].expect("loop vertex materialized");
                    let b = outcome.graph_vertices[q].expect("loop vertex materialized");
                    assert!(
                        find_half_edge(mesh, a, b).is_some(),
                        "loop edge {p}-{q} must be a mesh edge"
                    );
                }
            }
        }
        assert_eq!(closed_loops, 2, "one loop per slab cap");

        // Ring and disk fragments all trace to the two original caps.
        let cap_origins: Vec<FaceId> = {
            let mut origins: Vec<FaceId> = result
                .outcome_a
                .face_origins
                .iter()
                .map(|&(_, old)| old)
                .collect();
            origins.sort_unstable_by_key(|face| face.index());
            origins.dedup();
            origins
        };
        assert_eq!(cap_origins.len(), 2, "fragments trace to the two caps");
    }

    #[test]
    fn drilled_cap_refacing_is_deterministic() {
        let first = run_pair(slab(), drill_prism());
        let second = run_pair(slab(), drill_prism());
        assert_eq!(first.outcome_a, second.outcome_a);
        assert_eq!(first.outcome_b, second.outcome_b);
    }
}
