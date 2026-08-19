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
//! Configurations outside the v1 envelope are typed deferrals, never
//! silent: faces containing junction vertices (local degree ≥ 3), closed
//! loops fully interior to one face, dangling chains ending inside a face,
//! and chains whose sub-loop assignment is ambiguous all leave the face
//! unsplit and report [`BooleanFailureKind::SplitDeferred`].

use alloc::vec::Vec;

use hashbrown::HashMap;

use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::graph::{IntersectionGraph, MeshAnchor};
use crate::{
    DeletePolicy, FaceId, Mesh, PropagatePolicy, VertexId, attr,
    op::{
        add_face, add_vertex, delete_faces, set_edge_seam, set_edge_sharpness, set_face_region,
        set_vertex_position, split_edge,
    },
};

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
    // --- Stage 1: resolve vertex-anchored and edge-anchored graph
    // vertices to mesh vertices (kernel edge splits for the latter).
    let mut on_edge: HashMap<(VertexId, VertexId), Vec<u32>> = HashMap::new();
    for index in 0..graph.vertices.len() {
        match anchor_of(index) {
            MeshAnchor::Vertex(vertex) => {
                outcome.graph_vertices[index] = Some(vertex);
            }
            MeshAnchor::EdgeSpan(u, v) => {
                if find_half_edge(mesh, u, v).is_some() {
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
            let from = session.mesh().vertex_position(u).map(promote);
            let to = session.mesh().vertex_position(v).map(promote);
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
                let Some(half_edge) = find_half_edge(session.mesh(), cursor, v) else {
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
            .is_some_and(|(u, v)| find_half_edge(mesh, u, v).is_some());
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

fn promote(p: &[f32; 3]) -> [f64; 3] {
    [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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

/// The single documented f64 -> f32 narrowing for split geometry.
fn narrow(p: [f64; 3]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "deliberate narrowing of constructed intersection positions"
    )]
    {
        [p[0] as f32, p[1] as f32, p[2] as f32]
    }
}

/// Finds a half-edge between `from` and `to` in either direction, if the
/// mesh has that undirected edge. Linear in the half-edge count (v1;
/// splitting is a construction stage).
fn find_half_edge(mesh: &Mesh, from: VertexId, to: VertexId) -> Option<crate::HalfEdgeId> {
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            let ends = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge));
            if ends == (Some(from), Some(to)) || ends == (Some(to), Some(from)) {
                return Some(half_edge);
            }
        }
    }
    None
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
    if local.values().any(|n| n.len() >= 3) {
        defer(
            "face contains an intersection junction (local degree >= 3)",
            outcome,
            diagnostics,
        );
        return;
    }

    // Trace face-local chains between boundary-resolved endpoints.
    let mut terminals: Vec<u32> = local
        .iter()
        .filter(|(_, neighbors)| neighbors.len() == 1)
        .map(|(&vertex, _)| vertex)
        .collect();
    terminals.sort_unstable();
    if terminals.is_empty() && !local.is_empty() {
        defer(
            "closed intersection loop fully interior to one face",
            outcome,
            diagnostics,
        );
        return;
    }

    let mut visited: HashMap<(u32, u32), bool> = HashMap::new();
    let mut chains: Vec<Vec<u32>> = Vec::new();
    for &start in &terminals {
        for &next in &local[&start] {
            let key = (start.min(next), start.max(next));
            if visited.contains_key(&key) {
                continue;
            }
            let mut chain = alloc::vec![start];
            let mut previous = start;
            let mut current = next;
            visited.insert(key, true);
            chain.push(current);
            while local[&current].len() == 2 {
                let &step = local[&current]
                    .iter()
                    .find(|&&candidate| candidate != previous)
                    .expect("degree-2 vertex has another neighbor");
                let key = (current.min(step), current.max(step));
                if visited.contains_key(&key) {
                    break;
                }
                visited.insert(key, true);
                chain.push(step);
                previous = current;
                current = step;
            }
            chains.push(chain);
        }
    }

    // Every chain endpoint must already be a mesh vertex on this face's
    // loop; interiors must be face-interior graph vertices.
    let loop_vertices: Vec<VertexId> = mesh
        .face_loop(face)
        .filter_map(|he| mesh.to_vertex(he))
        .collect();
    for chain in &chains {
        let first = *chain.first().expect("chains are nonempty");
        let last = *chain.last().expect("chains are nonempty");
        for endpoint in [first, last] {
            let resolved = outcome.graph_vertices[endpoint as usize];
            let on_loop = resolved.is_some_and(|v| loop_vertices.contains(&v));
            if !on_loop {
                defer(
                    "intersection chain ends inside the face (dangling cut)",
                    outcome,
                    diagnostics,
                );
                return;
            }
        }
    }

    // Cut the face polygon along each chain in deterministic order.
    let mut sub_loops: Vec<Vec<VertexId>> = alloc::vec![loop_vertices];
    let mut pending: Vec<&Vec<u32>> = chains.iter().collect();
    pending.sort_unstable_by_key(|chain| {
        let first = *chain.first().expect("nonempty");
        let last = *chain.last().expect("nonempty");
        (first.min(last), first.max(last))
    });

    // Interior chain vertices materialize lazily, once per graph vertex.
    let mut session = mesh.edit();
    let resolve = |vertex_index: u32,
                   session: &mut crate::EditSession<'_, crate::DiscardChanges>,
                   outcome: &mut MeshSplitOutcome|
     -> VertexId {
        if let Some(vertex) = outcome.graph_vertices[vertex_index as usize] {
            return vertex;
        }
        let position = narrow(graph.vertices[vertex_index as usize].position);
        let vertex = add_vertex(session, position);
        outcome.graph_vertices[vertex_index as usize] = Some(vertex);
        outcome.stats.interior_vertices += 1;
        vertex
    };

    for chain in pending {
        let first = outcome.graph_vertices[*chain.first().expect("nonempty") as usize]
            .expect("endpoints validated on the loop");
        let last = outcome.graph_vertices[*chain.last().expect("nonempty") as usize]
            .expect("endpoints validated on the loop");
        // Locate the sub-loop containing both endpoints.
        let mut candidates = sub_loops
            .iter()
            .enumerate()
            .filter(|(_, sub)| sub.contains(&first) && sub.contains(&last));
        let Some((slot, _)) = candidates.next() else {
            defer(
                "chain endpoints span different sub-loops (ambiguous cut)",
                outcome,
                diagnostics,
            );
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
            return;
        };
        if candidates.next().is_some() {
            defer(
                "chain endpoints lie on a previous cut (ambiguous cut)",
                outcome,
                diagnostics,
            );
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
            return;
        }

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
        let interior: Vec<VertexId> = chain[1..chain.len() - 1]
            .iter()
            .map(|&v| resolve(v, &mut session, outcome))
            .collect();

        // Loop 1: first .. last along the loop, then chain reversed back.
        let mut loop_one: Vec<VertexId> = Vec::new();
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
        let mut loop_two: Vec<VertexId> = Vec::new();
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
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }

    if sub_loops.len() <= 1 {
        return; // Nothing actually cut.
    }

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
    for sub in &sub_loops {
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
        if let Some(half_edge) = find_half_edge(session.mesh(), from, to) {
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
    use crate::boolean::{
        BooleanBvh, BooleanDiagnostics, BooleanScratch, IntersectionGraph,
        build_intersection_graph, narrow_phase,
    };
    use crate::{FaceTriangulation, MeshBuilder};

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

    struct EndToEnd {
        mesh_a: Mesh,
        mesh_b: Mesh,
        graph: IntersectionGraph,
        outcome_a: MeshSplitOutcome,
        outcome_b: MeshSplitOutcome,
        diagnostics: BooleanDiagnostics,
    }

    fn run_two_cubes() -> EndToEnd {
        let mut mesh_a = cube([0.0, 0.0, 0.0]);
        let mut mesh_b = cube([0.5, 0.5, 0.5]);
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
}
