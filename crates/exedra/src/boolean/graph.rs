// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Intersection graph construction: narrow-phase segments connected into
//! polylines and loops with mesh-level attribution on both meshes.
//!
//! Welding is provenance-based, never tolerance-based. Endpoint provenance
//! from the narrow phase resolves to mesh-level anchors (a concrete vertex,
//! a vertex-pair span, or a face interior); endpoints weld when their
//! anchors agree. Vertex-anchored endpoints additionally merge across
//! meshes by exact position bits — vertex positions pass through the
//! narrow phase bit-exactly, so this closes vertex-vertex coincidences
//! without any epsilon.
//!
//! Touch contacts ([`SegmentKind::Touch`]) are recorded as isolated touch
//! points: they carry classification hints for later stages but never join
//! cut polylines. A touch is one geometric point, so its two reported
//! endpoints weld into a single graph vertex with the sharper anchor per
//! mesh. Coplanar pairs never reach this stage as segments — the coplanar
//! contact stage ([`super::collect_coplanar_contacts`]) owns them.
//!
//! All output ordering is deterministic: graph vertices in first-contact
//! order over the (already deterministic) segment stream, edges sorted by
//! vertex indices, polylines traced from the lowest-index terminal with
//! lowest-index neighbor preference.

use alloc::vec::Vec;

use hashbrown::HashMap;

use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::narrow::{EndpointSource, IntersectionSegment, SegmentKind};
use super::{BooleanScratch, BooleanTriangleRef};
use crate::{FaceId, FaceTriangulation, Mesh, VertexId};

/// Mesh-level resolution of an endpoint's provenance on one mesh.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum MeshAnchor {
    /// The endpoint is exactly this mesh vertex.
    Vertex(VertexId),
    /// The endpoint lies on the span between two mesh vertices — a mesh
    /// edge when the vertices are loop-adjacent, otherwise a triangulation
    /// diagonal (which is face-interior at mesh level).
    EdgeSpan(VertexId, VertexId),
    /// The endpoint lies in the interior of this face.
    FaceInterior(FaceId),
}

/// One welded graph vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphVertex {
    /// Constructed position (f64, from the first contributing endpoint in
    /// deterministic order; vertex-anchored positions are exact).
    pub position: [f64; 3],
    /// Mesh-level anchor on mesh A.
    pub anchor_a: MeshAnchor,
    /// Mesh-level anchor on mesh B.
    pub anchor_b: MeshAnchor,
    /// Faces of mesh A this vertex touches (sorted, deduplicated).
    pub faces_a: Vec<FaceId>,
    /// Faces of mesh B this vertex touches (sorted, deduplicated).
    pub faces_b: Vec<FaceId>,
}

/// One welded graph edge (a cut-curve subsegment).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    /// Endpoint graph-vertex indices, ascending.
    pub vertices: [u32; 2],
    /// `(face of A, face of B)` crossings that produced this edge
    /// (sorted, deduplicated).
    pub crossings: Vec<(FaceId, FaceId)>,
}

/// A traced intersection polyline.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    /// Graph-vertex indices along the polyline. For closed loops the first
    /// vertex is not repeated at the end.
    pub vertices: Vec<u32>,
    /// True when the polyline closes on itself.
    pub closed: bool,
}

/// Deterministic graph-construction counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct IntersectionGraphStats {
    /// Segment endpoints processed.
    pub endpoints: u64,
    /// Endpoints welded onto existing graph vertices.
    pub welded_endpoints: u64,
    /// Graph vertices created.
    pub vertices: u64,
    /// Graph edges created.
    pub edges: u64,
    /// Open polylines traced.
    pub open_polylines: u64,
    /// Closed loops traced.
    pub closed_loops: u64,
    /// Touch points recorded.
    pub touch_points: u64,
    /// Vertices with three or more incident edges (junctions).
    pub junction_vertices: u64,
}

/// The intersection graph: welded vertices, cut edges, and traced
/// polylines, with mesh-level attribution throughout.
///
/// This is also the primary boolean debug artifact: polylines are directly
/// exportable for inspection (positions plus per-vertex face attribution).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntersectionGraph {
    /// Welded vertices in first-contact order.
    pub vertices: Vec<GraphVertex>,
    /// Cut edges, sorted by vertex indices.
    pub edges: Vec<GraphEdge>,
    /// Traced polylines: open chains first, then closed loops, each
    /// starting from its lowest-index terminal.
    pub polylines: Vec<Polyline>,
    /// Isolated touch contacts (graph-vertex indices, ascending).
    pub touch_points: Vec<u32>,
    /// Construction counters.
    pub stats: IntersectionGraphStats,
}

/// Welding key: provenance-first, with vertex anchors taking precedence so
/// the same physical point keys identically regardless of which triangle
/// pair reported it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum WeldKey {
    VertexA(VertexId),
    VertexB(VertexId),
    Span(AnchorKey, AnchorKey),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum AnchorKey {
    Edge(VertexId, VertexId),
    Face(FaceId),
}

/// Builds the intersection graph from narrow-phase segments.
///
/// `strategy` must match the strategy the segments were produced under
/// (recorded in the broad-phase stats); triangle provenance is resolved to
/// mesh level through [`Mesh::face_triangles_into`] using `scratch`'s
/// reusable buffers.
///
/// Anomalies (transversal segments welding to a point, provenance that no
/// longer resolves) are reported into `diagnostics` and skipped — never
/// silently dropped, never a panic.
pub fn build_intersection_graph(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    segments: &[IntersectionSegment],
    strategy: FaceTriangulation,
    scratch: &mut BooleanScratch,
    diagnostics: &mut BooleanDiagnostics,
) -> IntersectionGraph {
    let mut graph = IntersectionGraph::default();
    let mut keys: HashMap<WeldKey, u32> = HashMap::new();
    // Exact-position merge table for vertex-anchored points: closes
    // vertex-vertex coincidences across the two meshes without tolerance.
    let mut exact_positions: HashMap<[u64; 3], u32> = HashMap::new();
    let mut edge_index: HashMap<[u32; 2], u32> = HashMap::new();

    let mut buffer = core::mem::take(&mut scratch.narrow_face_a);

    for segment in segments {
        let mut anchors = [(
            MeshAnchor::FaceInterior(FaceId::OUTSIDE),
            MeshAnchor::FaceInterior(FaceId::OUTSIDE),
        ); 2];
        let mut resolved = true;
        for (slot, endpoint) in anchors.iter_mut().zip([&segment.start, &segment.end]) {
            graph.stats.endpoints += 1;
            let anchor_a = resolve_anchor(
                mesh_a,
                segment.pair.a,
                endpoint.source_a,
                strategy,
                &mut buffer,
            );
            let anchor_b = resolve_anchor(
                mesh_b,
                segment.pair.b,
                endpoint.source_b,
                strategy,
                &mut buffer,
            );
            let (Some(anchor_a), Some(anchor_b)) = (anchor_a, anchor_b) else {
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::InternalInvariantViolation,
                    a: Some(segment.pair.a),
                    b: Some(segment.pair.b),
                    detail: "segment provenance no longer resolves against the meshes",
                });
                resolved = false;
                break;
            };
            *slot = (anchor_a, anchor_b);
        }
        if !resolved {
            continue;
        }

        if segment.kind == SegmentKind::Touch {
            // A touch is one geometric point that the interval overlap may
            // report with different provenance per endpoint (each bound
            // comes from a different triangle's cut). Weld it exactly once
            // with the sharper anchor per mesh — welding both endpoints
            // separately would create an orphan duplicate vertex that the
            // split stage then materializes as a duplicate mesh vertex.
            let [(start_a, start_b), (end_a, end_b)] = anchors;
            let anchor_a = sharper(start_a, end_a);
            let anchor_b = sharper(start_b, end_b);
            // The sharper endpoint's position wins (vertex-anchored
            // positions are exact); ties keep the start deterministically.
            let position = if anchor_rank(&end_a) + anchor_rank(&end_b)
                > anchor_rank(&start_a) + anchor_rank(&start_b)
            {
                segment.end.position
            } else {
                segment.start.position
            };
            let vertex = weld_endpoint(
                &mut graph,
                &mut keys,
                &mut exact_positions,
                position,
                anchor_a,
                anchor_b,
                segment.pair.a.face,
                segment.pair.b.face,
            );
            if !graph.touch_points.contains(&vertex) {
                graph.touch_points.push(vertex);
            }
            continue;
        }

        let mut endpoint_indices = [0_u32; 2];
        for (slot, ((anchor_a, anchor_b), endpoint)) in endpoint_indices
            .iter_mut()
            .zip(anchors.into_iter().zip([&segment.start, &segment.end]))
        {
            *slot = weld_endpoint(
                &mut graph,
                &mut keys,
                &mut exact_positions,
                endpoint.position,
                anchor_a,
                anchor_b,
                segment.pair.a.face,
                segment.pair.b.face,
            );
        }

        let [start, end] = endpoint_indices;
        if start == end {
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::NumericalInstability,
                a: Some(segment.pair.a),
                b: Some(segment.pair.b),
                detail: "transversal segment welded to a single point",
            });
            if !graph.touch_points.contains(&start) {
                graph.touch_points.push(start);
            }
            continue;
        }

        let key = [start.min(end), start.max(end)];
        let crossing = (segment.pair.a.face, segment.pair.b.face);
        if let Some(&edge) = edge_index.get(&key) {
            let crossings = &mut graph.edges[edge as usize].crossings;
            if !crossings.contains(&crossing) {
                crossings.push(crossing);
            }
        } else {
            let index = u32::try_from(graph.edges.len()).unwrap_or(u32::MAX);
            edge_index.insert(key, index);
            graph.edges.push(GraphEdge {
                vertices: key,
                crossings: alloc::vec![crossing],
            });
        }
    }

    scratch.narrow_face_a = buffer;

    // Canonical ordering for the artifact surface.
    for edge in &mut graph.edges {
        edge.crossings
            .sort_unstable_by_key(|(a, b)| (a.index(), b.index()));
    }
    graph.edges.sort_unstable_by_key(|edge| edge.vertices);
    for vertex in &mut graph.vertices {
        vertex.faces_a.sort_unstable_by_key(|f| f.index());
        vertex.faces_a.dedup();
        vertex.faces_b.sort_unstable_by_key(|f| f.index());
        vertex.faces_b.dedup();
    }
    graph.touch_points.sort_unstable();

    graph.stats.vertices = graph.vertices.len() as u64;
    graph.stats.edges = graph.edges.len() as u64;
    graph.stats.touch_points = graph.touch_points.len() as u64;

    trace_polylines(&mut graph);
    graph
}

/// Resolves triangle-level endpoint provenance to a mesh-level anchor.
fn resolve_anchor(
    mesh: &Mesh,
    triangle: BooleanTriangleRef,
    source: EndpointSource,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<MeshAnchor> {
    match source {
        EndpointSource::Interior => Some(MeshAnchor::FaceInterior(triangle.face)),
        EndpointSource::Vertex(k) => {
            let corners = triangle_corners(mesh, triangle, strategy, buffer)?;
            let vertex = mesh.to_vertex(corners[k as usize])?;
            Some(MeshAnchor::Vertex(vertex))
        }
        EndpointSource::Edge(k) => {
            let corners = triangle_corners(mesh, triangle, strategy, buffer)?;
            let i = k as usize;
            let j = (i + 1) % 3;
            let a = mesh.to_vertex(corners[i])?;
            let b = mesh.to_vertex(corners[j])?;
            let (lo, hi) = if a.index() <= b.index() {
                (a, b)
            } else {
                (b, a)
            };
            Some(MeshAnchor::EdgeSpan(lo, hi))
        }
    }
}

fn triangle_corners(
    mesh: &Mesh,
    triangle: BooleanTriangleRef,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<[crate::CornerId; 3]> {
    let _ = mesh.face_triangles_into(triangle.face, strategy, buffer);
    buffer.get(triangle.triangle_index as usize).copied()
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal welder threading fixed construction context"
)]
fn weld_endpoint(
    graph: &mut IntersectionGraph,
    keys: &mut HashMap<WeldKey, u32>,
    exact_positions: &mut HashMap<[u64; 3], u32>,
    position: [f64; 3],
    anchor_a: MeshAnchor,
    anchor_b: MeshAnchor,
    face_a: FaceId,
    face_b: FaceId,
) -> u32 {
    let key = weld_key(anchor_a, anchor_b);
    let position_bits = [
        position[0].to_bits(),
        position[1].to_bits(),
        position[2].to_bits(),
    ];
    let vertex_anchored = matches!(key, WeldKey::VertexA(_) | WeldKey::VertexB(_));

    let index = if let Some(&index) = keys.get(&key) {
        graph.stats.welded_endpoints += 1;
        index
    } else if vertex_anchored && exact_positions.contains_key(&position_bits) {
        // Vertex-vertex coincidence across meshes: exact positions, no
        // tolerance involved.
        let index = exact_positions[&position_bits];
        keys.insert(key, index);
        graph.stats.welded_endpoints += 1;
        index
    } else {
        let index = u32::try_from(graph.vertices.len()).unwrap_or(u32::MAX);
        keys.insert(key, index);
        graph.vertices.push(GraphVertex {
            position,
            anchor_a,
            anchor_b,
            faces_a: Vec::new(),
            faces_b: Vec::new(),
        });
        if vertex_anchored {
            exact_positions.entry(position_bits).or_insert(index);
        }
        index
    };

    let vertex = &mut graph.vertices[index as usize];
    // Prefer the sharpest anchors seen for this vertex (Vertex beats
    // EdgeSpan beats FaceInterior); attribution accumulates.
    vertex.anchor_a = sharper(vertex.anchor_a, anchor_a);
    vertex.anchor_b = sharper(vertex.anchor_b, anchor_b);
    vertex.faces_a.push(face_a);
    vertex.faces_b.push(face_b);
    index
}

fn weld_key(anchor_a: MeshAnchor, anchor_b: MeshAnchor) -> WeldKey {
    if let MeshAnchor::Vertex(v) = anchor_a {
        return WeldKey::VertexA(v);
    }
    if let MeshAnchor::Vertex(v) = anchor_b {
        return WeldKey::VertexB(v);
    }
    WeldKey::Span(anchor_key(anchor_a), anchor_key(anchor_b))
}

fn anchor_key(anchor: MeshAnchor) -> AnchorKey {
    match anchor {
        MeshAnchor::Vertex(_) => unreachable!("vertex anchors take the vertex key path"),
        MeshAnchor::EdgeSpan(a, b) => AnchorKey::Edge(a, b),
        MeshAnchor::FaceInterior(face) => AnchorKey::Face(face),
    }
}

fn anchor_rank(anchor: &MeshAnchor) -> u8 {
    match anchor {
        MeshAnchor::Vertex(_) => 2,
        MeshAnchor::EdgeSpan(..) => 1,
        MeshAnchor::FaceInterior(_) => 0,
    }
}

fn sharper(current: MeshAnchor, candidate: MeshAnchor) -> MeshAnchor {
    if anchor_rank(&candidate) > anchor_rank(&current) {
        candidate
    } else {
        current
    }
}

/// Traces polylines: open chains from terminals (degree 1 or >= 3), then
/// pure closed loops, all with deterministic starts and orientations.
fn trace_polylines(graph: &mut IntersectionGraph) {
    let vertex_count = graph.vertices.len();
    let mut adjacency: Vec<Vec<(u32, u32)>> = alloc::vec![Vec::new(); vertex_count];
    for (edge_index, edge) in graph.edges.iter().enumerate() {
        let e = u32::try_from(edge_index).unwrap_or(u32::MAX);
        let [a, b] = edge.vertices;
        adjacency[a as usize].push((b, e));
        adjacency[b as usize].push((a, e));
    }
    for neighbors in &mut adjacency {
        neighbors.sort_unstable();
    }

    let degree = |v: usize| adjacency[v].len();
    let is_terminal = |v: usize| degree(v) == 1 || degree(v) >= 3;
    graph.stats.junction_vertices = (0..vertex_count).filter(|&v| degree(v) >= 3).count() as u64;

    let mut edge_visited = alloc::vec![false; graph.edges.len()];
    let mut polylines = Vec::new();

    // Open chains from terminals.
    for start in 0..vertex_count {
        if !is_terminal(start) {
            continue;
        }
        for &(first_next, first_edge) in &adjacency[start] {
            if edge_visited[first_edge as usize] {
                continue;
            }
            let mut chain = alloc::vec![u32::try_from(start).unwrap_or(u32::MAX)];
            let mut previous = start;
            let mut current = first_next as usize;
            edge_visited[first_edge as usize] = true;
            chain.push(first_next);
            while !is_terminal(current) && degree(current) == 2 {
                let step = adjacency[current]
                    .iter()
                    .find(|(next, edge)| {
                        !edge_visited[*edge as usize] && *next as usize != previous
                    })
                    .or_else(|| {
                        adjacency[current]
                            .iter()
                            .find(|(_, edge)| !edge_visited[*edge as usize])
                    })
                    .copied();
                let Some((next, edge)) = step else {
                    break;
                };
                edge_visited[edge as usize] = true;
                chain.push(next);
                previous = current;
                current = next as usize;
            }
            polylines.push(Polyline {
                vertices: chain,
                closed: false,
            });
        }
    }

    // Remaining edges form pure loops (every vertex degree 2).
    for start in 0..vertex_count {
        let Some(&(first_next, first_edge)) = adjacency[start]
            .iter()
            .find(|(_, edge)| !edge_visited[*edge as usize])
        else {
            continue;
        };
        let mut chain = alloc::vec![u32::try_from(start).unwrap_or(u32::MAX)];
        edge_visited[first_edge as usize] = true;
        let mut previous = start;
        let mut current = first_next as usize;
        while current != start {
            chain.push(u32::try_from(current).unwrap_or(u32::MAX));
            let step = adjacency[current]
                .iter()
                .find(|(next, edge)| !edge_visited[*edge as usize] && *next as usize != previous)
                .or_else(|| {
                    adjacency[current]
                        .iter()
                        .find(|(_, edge)| !edge_visited[*edge as usize])
                })
                .copied();
            let Some((next, edge)) = step else {
                break;
            };
            edge_visited[edge as usize] = true;
            previous = current;
            current = next as usize;
        }
        polylines.push(Polyline {
            vertices: chain,
            closed: current == start,
        });
    }

    graph.stats.open_polylines = polylines.iter().filter(|p| !p.closed).count() as u64;
    graph.stats.closed_loops = polylines.iter().filter(|p| p.closed).count() as u64;
    graph.polylines = polylines;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshBuilder;
    use crate::boolean::{BooleanBvh, BooleanScratch, narrow_phase};

    /// Builds a unit cube mesh with its minimum corner at `origin`.
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

    fn build_graph(mesh_a: &Mesh, mesh_b: &Mesh) -> (IntersectionGraph, BooleanDiagnostics) {
        let mut scratch = BooleanScratch::new();
        let strategy = FaceTriangulation::Fan;
        let bvh_a = BooleanBvh::build(mesh_a, strategy, &mut scratch);
        let bvh_b = BooleanBvh::build(mesh_b, strategy, &mut scratch);
        let mut pairs = Vec::new();
        bvh_a.query_overlaps(&bvh_b, &mut scratch, &mut pairs);
        let mut segments = Vec::new();
        let mut diagnostics = BooleanDiagnostics::default();
        narrow_phase(
            mesh_a,
            mesh_b,
            &pairs,
            strategy,
            &mut scratch,
            &mut segments,
            &mut diagnostics,
        );
        let graph = build_intersection_graph(
            mesh_a,
            mesh_b,
            &segments,
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        (graph, diagnostics)
    }

    #[test]
    fn two_cube_overlap_traces_one_closed_loop() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, 0.5, 0.5]);
        let (graph, diagnostics) = build_graph(&mesh_a, &mesh_b);
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());

        assert_eq!(graph.stats.closed_loops, 1, "one hexagonal cut loop");
        assert_eq!(graph.stats.open_polylines, 0);
        assert_eq!(graph.stats.junction_vertices, 0);
        let hexagon = &graph.polylines[0];
        assert!(hexagon.closed);
        assert!(hexagon.vertices.len() >= 6, "at least the six loop corners");

        // The six hand-computed loop corners all appear as graph vertices.
        let corners = [
            [0.5, 0.5, 1.0],
            [0.5, 1.0, 1.0],
            [1.0, 0.5, 1.0],
            [1.0, 0.5, 0.5],
            [1.0, 1.0, 0.5],
            [0.5, 1.0, 0.5],
        ];
        for corner in corners {
            assert!(
                graph.vertices.iter().any(|v| v.position == corner),
                "missing loop corner {corner:?}"
            );
        }
        // Every loop vertex is inside the shared overlap region.
        for &index in &hexagon.vertices {
            let p = graph.vertices[index as usize].position;
            for &coordinate in &p {
                assert!((0.5 - 1e-9..=1.0 + 1e-9).contains(&coordinate));
            }
        }
        // Every vertex carries attribution on both meshes.
        for vertex in &graph.vertices {
            assert!(!vertex.faces_a.is_empty());
            assert!(!vertex.faces_b.is_empty());
        }
        // Every graph edge knows the face pair it crosses.
        for edge in &graph.edges {
            assert!(!edge.crossings.is_empty());
        }
    }

    #[test]
    fn graph_construction_is_deterministic() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, 0.5, 0.5]);
        let (first, _) = build_graph(&mesh_a, &mesh_b);
        let (second, _) = build_graph(&mesh_a, &mesh_b);
        assert_eq!(first, second, "graphs must be bit-deterministic");
    }

    #[test]
    fn corner_touching_cubes_classify_as_touch_not_cut() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([1.0, 1.0, 1.0]);
        let (graph, _diagnostics) = build_graph(&mesh_a, &mesh_b);
        assert_eq!(graph.stats.edges, 0, "no cut edges from a point touch");
        assert_eq!(graph.stats.closed_loops, 0);
        assert_eq!(graph.stats.open_polylines, 0);
        assert!(
            graph.stats.touch_points >= 1,
            "the shared corner registers as a touch point"
        );
        let touch = graph.vertices[graph.touch_points[0] as usize].position;
        assert_eq!(touch, [1.0, 1.0, 1.0]);
    }
}
