// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Intersection graph construction: narrow-phase segments connected into
//! polylines and loops with mesh-level attribution on both meshes.
//!
//! Welding is provenance-based, never tolerance-based. Endpoint provenance
//! from the narrow phase resolves to mesh-level anchors (a concrete vertex,
//! a vertex-pair span, or a face interior); endpoints weld when their
//! anchors agree. Endpoints whose coordinates narrow to the same f32 position
//! merge only when their anchors share a topological carrier on both meshes.
//! The coordinate match reflects the graph's eventual mesh representation;
//! the carrier check prevents unrelated seams in the same representable cell
//! from becoming one graph vertex.
//!
//! Adjacent triangle pairs can reconstruct one edge crossing through
//! edge/edge and edge/face plane solves whose dependent coordinates do not
//! narrow alike. In that case exact source-edge incidence establishes
//! identity without a spatial tolerance. Those proofs are accumulated as
//! equivalence classes and compacted after the complete segment stream, so
//! graph identity does not depend on which triangle pair reported first.
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

use exedra_math::{cross, dot, lerp, narrow, promote, sub};
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
    /// Canonicalized position (f64, from the first contributing endpoint in
    /// deterministic order; vertex anchors and recoverable carrier
    /// intersections use their exact stored inputs).
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

#[derive(Copy, Clone, Eq, PartialEq)]
struct EdgeIncidence {
    vertex: u32,
    opposite: MeshAnchor,
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct FacePairIncidence {
    vertex: u32,
    anchor_a: MeshAnchor,
    anchor_b: MeshAnchor,
}

#[derive(Default)]
struct WeldIndex {
    // Exact endpoint provenance is the primary identity.
    keys: HashMap<WeldKey, u32>,
    // Stored-position identity is a fallback guarded by carrier agreement.
    narrowed_positions: HashMap<[u32; 3], Vec<u32>>,
    // An edge/edge crossing may later be reported as edge/face by an
    // adjacent triangle pair. Keep the opposite anchor from every observed
    // edge incidence so that topology can recover that endpoint even when
    // its independently reconstructed dependent coordinate rounds
    // differently.
    edges_a: HashMap<(VertexId, VertexId), Vec<EdgeIncidence>>,
    edges_b: HashMap<(VertexId, VertexId), Vec<EdgeIncidence>>,
    // Reciprocal edge/face reports do not share an edge-key lookup. Keep
    // their originating mesh-face pair so the exact source spans can prove
    // whether the two reports are the same crossing or the distinct ends of
    // a real intersection segment.
    face_pairs: HashMap<(FaceId, FaceId), Vec<FacePairIncidence>>,
    // Incidence evidence can arrive after both of the vertices it proves
    // equivalent. Keep equivalence classes until every segment has been
    // observed, then compact the public graph once. Always choosing the
    // lowest root preserves first-contact ordering.
    parents: Vec<u32>,
}

impl WeldIndex {
    fn root(&self, mut index: u32) -> u32 {
        while self.parents[index as usize] != index {
            index = self.parents[index as usize];
        }
        index
    }

    fn join(&mut self, graph: &mut IntersectionGraph, left: u32, right: u32) -> u32 {
        let left = self.root(left);
        let right = self.root(right);
        if left == right {
            return left;
        }
        let (root, merged) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        self.parents[merged as usize] = root;

        // Make the representative useful to later carrier checks. The final
        // compaction discards `merged`, but welding continues while segments
        // are streamed and must already see the class's sharpest provenance.
        let merged_vertex = graph.vertices[merged as usize].clone();
        let root_vertex = &mut graph.vertices[root as usize];
        root_vertex.anchor_a = sharper(root_vertex.anchor_a, merged_vertex.anchor_a);
        root_vertex.anchor_b = sharper(root_vertex.anchor_b, merged_vertex.anchor_b);
        root_vertex.faces_a.extend(merged_vertex.faces_a);
        root_vertex.faces_b.extend(merged_vertex.faces_b);
        root
    }
}

#[derive(Copy, Clone)]
struct WeldedEndpoint {
    index: u32,
    recovered_from_incidence: bool,
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
    let mut weld_index = WeldIndex::default();
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
                endpoint.position,
                strategy,
                &mut buffer,
            );
            let anchor_b = resolve_anchor(
                mesh_b,
                segment.pair.b,
                endpoint.source_b,
                endpoint.position,
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
        let positions = [
            canonical_endpoint_position(
                mesh_a,
                segment.pair.a,
                anchors[0].0,
                segment.start.source_a,
                mesh_b,
                segment.pair.b,
                anchors[0].1,
                segment.start.source_b,
                segment.start.position,
                strategy,
                &mut buffer,
            ),
            canonical_endpoint_position(
                mesh_a,
                segment.pair.a,
                anchors[1].0,
                segment.end.source_a,
                mesh_b,
                segment.pair.b,
                anchors[1].1,
                segment.end.source_b,
                segment.end.position,
                strategy,
                &mut buffer,
            ),
        ];

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
                positions[1]
            } else {
                positions[0]
            };
            let vertex = weld_endpoint(
                mesh_a,
                mesh_b,
                &mut graph,
                &mut weld_index,
                position,
                anchor_a,
                anchor_b,
                segment.pair.a.face,
                segment.pair.b.face,
            )
            .index;
            if !graph.touch_points.contains(&vertex) {
                graph.touch_points.push(vertex);
            }
            continue;
        }

        let mut endpoint_indices = [WeldedEndpoint {
            index: 0,
            recovered_from_incidence: false,
        }; 2];
        for (slot, ((anchor_a, anchor_b), position)) in endpoint_indices
            .iter_mut()
            .zip(anchors.into_iter().zip(positions))
        {
            *slot = weld_endpoint(
                mesh_a,
                mesh_b,
                &mut graph,
                &mut weld_index,
                position,
                anchor_a,
                anchor_b,
                segment.pair.a.face,
                segment.pair.b.face,
            );
        }

        let [start, end] = endpoint_indices;
        if start.index == end.index {
            // The split mesh stores f32 positions. A positive-length f64
            // construction whose endpoints narrow to one stored point is a
            // representational touch, not a broken graph edge. A collapse
            // for any other reason remains an invariant-worthy diagnostic.
            if narrow(positions[0]) != narrow(positions[1])
                && !start.recovered_from_incidence
                && !end.recovered_from_incidence
            {
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::NumericalInstability,
                    a: Some(segment.pair.a),
                    b: Some(segment.pair.b),
                    detail: "transversal segment welded to a single point",
                });
            }
            if !graph.touch_points.contains(&start.index) {
                graph.touch_points.push(start.index);
            }
            continue;
        }

        let key = [start.index.min(end.index), start.index.max(end.index)];
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

    reconcile_incidence_classes(mesh_a, mesh_b, &mut graph, &mut weld_index);
    compact_welded_graph(&mut graph, &weld_index);

    // Canonical ordering for the artifact surface.
    for edge in &mut graph.edges {
        edge.crossings
            .sort_unstable_by_key(|(a, b)| (a.index(), b.index()));
        edge.crossings.dedup();
    }
    graph.edges.sort_unstable_by_key(|edge| edge.vertices);
    for vertex in &mut graph.vertices {
        vertex.faces_a.sort_unstable_by_key(|f| f.index());
        vertex.faces_a.dedup();
        vertex.faces_b.sort_unstable_by_key(|f| f.index());
        vertex.faces_b.dedup();
    }
    graph.touch_points.sort_unstable();
    graph.touch_points.dedup();

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
    position: [f64; 3],
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<MeshAnchor> {
    match source {
        EndpointSource::Interior => {
            sharpen_interior_anchor(mesh, triangle, position, strategy, buffer)
        }
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

/// Recovers mesh-vertex provenance lost by interval clipping.
///
/// A segment bound taken from the other triangle is labelled `Interior` on
/// this triangle even when it lands on one of this triangle's vertices. The
/// f64 construction can also drift along the triangle normal, so comparing all
/// three coordinates would miss that vertex. Exact equality after f32
/// narrowing in the triangle's dominant 2D projection is the
/// representation-level test: it snaps no in-plane coordinate and makes the
/// graph endpoint materialize on the existing corner. Interior points that
/// merely land on an edge remain face-interior—the narrow phase's original
/// edge provenance is the authority for edge/plane reconstruction below.
fn sharpen_interior_anchor(
    mesh: &Mesh,
    triangle: BooleanTriangleRef,
    position: [f64; 3],
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<MeshAnchor> {
    let corners = triangle_corners(mesh, triangle, strategy, buffer)?;
    let vertices = [
        mesh.to_vertex(corners[0])?,
        mesh.to_vertex(corners[1])?,
        mesh.to_vertex(corners[2])?,
    ];
    let points = [
        *mesh.vertex_position(vertices[0])?,
        *mesh.vertex_position(vertices[1])?,
        *mesh.vertex_position(vertices[2])?,
    ];
    let normal = cross(
        sub(promote(points[1]), promote(points[0])),
        sub(promote(points[2]), promote(points[0])),
    );
    if normal == [0.0; 3] {
        return Some(MeshAnchor::FaceInterior(triangle.face));
    }
    let dropped_axis = dominant_axis(normal);
    let position = narrow(position);
    if let Some(index) = points
        .iter()
        .position(|point| projected_equal(*point, position, dropped_axis))
    {
        return Some(MeshAnchor::Vertex(vertices[index]));
    }
    Some(MeshAnchor::FaceInterior(triangle.face))
}

fn dominant_axis(vector: [f64; 3]) -> usize {
    let absolute = vector.map(f64::abs);
    if absolute[0] >= absolute[1] && absolute[0] >= absolute[2] {
        0
    } else if absolute[1] >= absolute[2] {
        1
    } else {
        2
    }
}

fn projected_equal(left: [f32; 3], right: [f32; 3], dropped_axis: usize) -> bool {
    (0..3).all(|axis| axis == dropped_axis || left[axis] == right[axis])
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

/// Canonicalizes a constructed endpoint against its sharpest source carrier.
///
/// Triangle intersection positions are f64 constructions from exact f32
/// inputs. Their last few bits can drift off an axis-aligned source plane;
/// narrowing that drift into the output mesh makes a later coplanar boolean
/// see cracks that are not present topologically. Existing vertices win
/// exactly. Otherwise an edge endpoint is recomputed against the opposite
/// triangle plane and the plane's dominant coordinate is solved last. This
/// changes no in-plane classification and gives both split meshes one shared
/// stored position.
#[expect(
    clippy::too_many_arguments,
    reason = "canonicalization needs both fixed provenance carriers"
)]
fn canonical_endpoint_position(
    mesh_a: &Mesh,
    triangle_a: BooleanTriangleRef,
    anchor_a: MeshAnchor,
    source_a: EndpointSource,
    mesh_b: &Mesh,
    triangle_b: BooleanTriangleRef,
    anchor_b: MeshAnchor,
    source_b: EndpointSource,
    fallback: [f64; 3],
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> [f64; 3] {
    if let MeshAnchor::Vertex(vertex) = anchor_a {
        return mesh_a
            .vertex_position(vertex)
            .copied()
            .map(promote)
            .unwrap_or(fallback);
    }
    if let MeshAnchor::Vertex(vertex) = anchor_b {
        return mesh_b
            .vertex_position(vertex)
            .copied()
            .map(promote)
            .unwrap_or(fallback);
    }
    match (source_a, source_b, anchor_a, anchor_b) {
        (
            EndpointSource::Edge(_),
            EndpointSource::Edge(_),
            MeshAnchor::EdgeSpan(a, b),
            MeshAnchor::EdgeSpan(c, d),
        ) => edge_edge_intersection(mesh_a, [a, b], mesh_b, [c, d]).unwrap_or(fallback),
        (EndpointSource::Edge(_), EndpointSource::Interior, MeshAnchor::EdgeSpan(a, b), _) => {
            edge_plane_intersection(mesh_a, [a, b], mesh_b, triangle_b, strategy, buffer)
                .unwrap_or(fallback)
        }
        (EndpointSource::Interior, EndpointSource::Edge(_), _, MeshAnchor::EdgeSpan(a, b)) => {
            edge_plane_intersection(mesh_b, [a, b], mesh_a, triangle_a, strategy, buffer)
                .unwrap_or(fallback)
        }
        _ => fallback,
    }
}

/// Intersects two non-parallel exact promoted-f32 spans in the 2D projection
/// where their direction cross product is largest. Endpoint provenance says
/// the lines meet; reconstructing that meeting from the stored carriers
/// removes evaluation-order drift between adjacent triangle pairs.
fn edge_edge_intersection(
    mesh_a: &Mesh,
    edge_a: [VertexId; 2],
    mesh_b: &Mesh,
    edge_b: [VertexId; 2],
) -> Option<[f64; 3]> {
    let a = promote(*mesh_a.vertex_position(edge_a[0])?);
    let b = promote(*mesh_a.vertex_position(edge_a[1])?);
    let c = promote(*mesh_b.vertex_position(edge_b[0])?);
    let d = promote(*mesh_b.vertex_position(edge_b[1])?);
    let left = sub(b, a);
    let right = sub(d, c);
    let dropped_axis = dominant_axis(cross(left, right));
    let [u, v] = match dropped_axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let denominator = left[u] * right[v] - left[v] * right[u];
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let offset = sub(c, a);
    let parameter = (offset[u] * right[v] - offset[v] * right[u]) / denominator;
    let other_parameter = (offset[u] * left[v] - offset[v] * left[u]) / denominator;
    if !parameter.is_finite()
        || !other_parameter.is_finite()
        || !(0.0..=1.0).contains(&parameter)
        || !(0.0..=1.0).contains(&other_parameter)
    {
        return None;
    }
    let point = lerp(a, b, parameter);
    let other_point = lerp(c, d, other_parameter);
    (narrow(point) == narrow(other_point)).then_some(point)
}

/// Intersects an exact promoted-f32 edge with an exact promoted-f32 triangle
/// plane, then solves the plane's dominant coordinate last. The final solve
/// is what preserves exactly constant coordinates for axis-aligned inputs.
fn edge_plane_intersection(
    edge_mesh: &Mesh,
    edge: [VertexId; 2],
    plane_mesh: &Mesh,
    plane_triangle: BooleanTriangleRef,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<[f64; 3]> {
    let edge_a = promote(*edge_mesh.vertex_position(edge[0])?);
    let edge_b = promote(*edge_mesh.vertex_position(edge[1])?);
    let corners = triangle_corners(plane_mesh, plane_triangle, strategy, buffer)?;
    let plane = [
        promote(*plane_mesh.vertex_position(plane_mesh.to_vertex(corners[0])?)?),
        promote(*plane_mesh.vertex_position(plane_mesh.to_vertex(corners[1])?)?),
        promote(*plane_mesh.vertex_position(plane_mesh.to_vertex(corners[2])?)?),
    ];
    let normal = cross(sub(plane[1], plane[0]), sub(plane[2], plane[0]));
    let direction = sub(edge_b, edge_a);
    let denominator = dot(normal, direction);
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let parameter = dot(normal, sub(plane[0], edge_a)) / denominator;
    if !parameter.is_finite() || !(0.0..=1.0).contains(&parameter) {
        return None;
    }
    let mut point = lerp(edge_a, edge_b, parameter);
    let axis = dominant_axis(normal);
    if normal[axis] == 0.0 {
        return None;
    }
    let residual: f64 = (0..3)
        .filter(|&candidate| candidate != axis)
        .map(|candidate| normal[candidate] * (point[candidate] - plane[0][candidate]))
        .sum();
    point[axis] = plane[0][axis] - residual / normal[axis];
    Some(point)
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal welder threading fixed construction context"
)]
fn weld_endpoint(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &mut IntersectionGraph,
    weld_index: &mut WeldIndex,
    position: [f64; 3],
    anchor_a: MeshAnchor,
    anchor_b: MeshAnchor,
    face_a: FaceId,
    face_b: FaceId,
) -> WeldedEndpoint {
    let key = weld_key(anchor_a, anchor_b);
    let narrowed = narrow(position);
    // Signed zero is one geometric mesh coordinate. Canonicalize it in the
    // key so arithmetic that happens to produce -0.0 cannot split a seam
    // vertex from an otherwise identical +0.0 construction.
    let position_bits = narrowed.map(|coordinate| {
        if coordinate == 0.0 {
            0.0_f32.to_bits()
        } else {
            coordinate.to_bits()
        }
    });

    let exact = weld_index
        .keys
        .get(&key)
        .map(|&index| weld_index.root(index));
    let same_position = || {
        weld_index
            .narrowed_positions
            .get(&position_bits)?
            .iter()
            .copied()
            .map(|candidate| weld_index.root(candidate))
            .find(|&candidate| {
                let vertex = &graph.vertices[candidate as usize];
                endpoint_carriers_establish_identity(
                    mesh_a,
                    vertex.anchor_a,
                    anchor_a,
                    mesh_b,
                    vertex.anchor_b,
                    anchor_b,
                    narrowed,
                )
            })
    };

    let matched = exact.or_else(same_position);
    let incidence = incidence_candidates(
        mesh_a, mesh_b, weld_index, anchor_a, anchor_b, face_a, face_b,
    );
    let reused = matched.is_some() || !incidence.is_empty();
    let mut recovered_from_incidence = false;
    let index = if let Some(mut root) = matched {
        // Provenance and stored-position matches choose the representative,
        // but must not discard other proofs carried by this observation.
        // A differently rounded incidence candidate may join when its exact
        // edge/edge crossing agrees with that representative, or when it is
        // no sharper than the representative. The latter closes reciprocal
        // edge/face observations without allowing a less-specific matched
        // point to swallow an already-established edge/edge branch.
        for candidate in incidence {
            let matched_root = weld_index.root(root);
            let candidate_root = weld_index.root(candidate.vertex);
            let matched_vertex = &graph.vertices[matched_root as usize];
            let candidate_vertex = &graph.vertices[candidate_root as usize];
            let matched_position = narrow(matched_vertex.position);
            let crossing_matches_representative = candidate
                .crossing
                .is_some_and(|crossing| narrow(crossing) == matched_position);
            let candidate_is_no_sharper =
                endpoint_anchor_rank(candidate_vertex) <= endpoint_anchor_rank(matched_vertex);
            if matched_root != candidate_root
                && (crossing_matches_representative || candidate_is_no_sharper)
            {
                root = weld_index.join(graph, matched_root, candidate_root);
                recovered_from_incidence = true;
            }
        }
        root
    } else if let Some((&first, rest)) = incidence.split_first() {
        // Every incidence candidate is proven to be the unique crossing of
        // the same source edges, so these proofs do form an equivalence class
        // even when traversal exposed several roots.
        let mut root = first.vertex;
        for &candidate in rest {
            root = weld_index.join(graph, root, candidate.vertex);
        }
        recovered_from_incidence = true;
        root
    } else {
        let index = u32::try_from(graph.vertices.len()).unwrap_or(u32::MAX);
        graph.vertices.push(GraphVertex {
            position,
            anchor_a,
            anchor_b,
            faces_a: Vec::new(),
            faces_b: Vec::new(),
        });
        weld_index.parents.push(index);
        weld_index
            .narrowed_positions
            .entry(position_bits)
            .or_default()
            .push(index);
        index
    };
    if reused {
        graph.stats.welded_endpoints += 1;
    }
    let index = weld_index.root(index);
    weld_index.keys.insert(key, index);

    let vertex = &mut graph.vertices[index as usize];
    // Prefer the sharpest anchors seen for this vertex (Vertex beats
    // EdgeSpan beats FaceInterior); attribution accumulates.
    vertex.anchor_a = sharper(vertex.anchor_a, anchor_a);
    vertex.anchor_b = sharper(vertex.anchor_b, anchor_b);
    vertex.faces_a.push(face_a);
    vertex.faces_b.push(face_b);
    if let MeshAnchor::EdgeSpan(a, b) = anchor_a {
        let entry = weld_index.edges_a.entry((a, b)).or_default();
        let incidence = EdgeIncidence {
            vertex: index,
            opposite: anchor_b,
        };
        if !entry.contains(&incidence) {
            entry.push(incidence);
        }
    }
    if let MeshAnchor::EdgeSpan(a, b) = anchor_b {
        let entry = weld_index.edges_b.entry((a, b)).or_default();
        let incidence = EdgeIncidence {
            vertex: index,
            opposite: anchor_a,
        };
        if !entry.contains(&incidence) {
            entry.push(incidence);
        }
    }
    let entry = weld_index.face_pairs.entry((face_a, face_b)).or_default();
    let incidence = FacePairIncidence {
        vertex: index,
        anchor_a,
        anchor_b,
    };
    if !entry.contains(&incidence) {
        entry.push(incidence);
    }
    WeldedEndpoint {
        index,
        recovered_from_incidence,
    }
}

fn incidence_candidates(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    weld_index: &WeldIndex,
    anchor_a: MeshAnchor,
    anchor_b: MeshAnchor,
    face_a: FaceId,
    face_b: FaceId,
) -> Vec<IncidenceCandidate> {
    let mut matches = Vec::new();
    if let MeshAnchor::EdgeSpan(a, b) = anchor_a
        && let Some(candidates) = weld_index.edges_a.get(&(a, b))
    {
        for &candidate in candidates {
            if let Some(edge_b) = edge_face_incidence(mesh_b, candidate.opposite, anchor_b)
                && edges_are_non_parallel(mesh_a, [a, b], mesh_b, edge_b)
            {
                matches.push(IncidenceCandidate {
                    vertex: weld_index.root(candidate.vertex),
                    crossing: edge_edge_intersection(mesh_a, [a, b], mesh_b, edge_b),
                });
            }
        }
    }
    if let MeshAnchor::EdgeSpan(a, b) = anchor_b
        && let Some(candidates) = weld_index.edges_b.get(&(a, b))
    {
        for &candidate in candidates {
            if let Some(edge_a) = edge_face_incidence(mesh_a, candidate.opposite, anchor_a)
                && edges_are_non_parallel(mesh_a, edge_a, mesh_b, [a, b])
            {
                matches.push(IncidenceCandidate {
                    vertex: weld_index.root(candidate.vertex),
                    crossing: edge_edge_intersection(mesh_a, edge_a, mesh_b, [a, b]),
                });
            }
        }
    }
    if let Some(candidates) = weld_index.face_pairs.get(&(face_a, face_b)) {
        for &candidate in candidates {
            if let Some(crossing) = reciprocal_edge_crossing(
                mesh_a,
                candidate.anchor_a,
                anchor_a,
                mesh_b,
                candidate.anchor_b,
                anchor_b,
            ) {
                matches.push(IncidenceCandidate {
                    vertex: weld_index.root(candidate.vertex),
                    crossing: Some(crossing),
                });
            }
        }
    }
    matches.sort_unstable_by_key(|candidate| candidate.vertex);
    matches
        .into_iter()
        .fold(Vec::new(), |mut unique, candidate| {
            if let Some(previous) = unique.last_mut()
                && previous.vertex == candidate.vertex
            {
                if previous.crossing.is_none() {
                    previous.crossing = candidate.crossing;
                }
            } else {
                unique.push(candidate);
            }
            unique
        })
}

#[derive(Clone, Copy)]
struct IncidenceCandidate {
    // The exact finite edge/edge point is present when the two stored spans
    // agree after narrowing. The topological incidence remains useful during
    // initial recovery even when independent edge/face arithmetic did not.
    vertex: u32,
    crossing: Option<[f64; 3]>,
}

fn edge_face_incidence(mesh: &Mesh, left: MeshAnchor, right: MeshAnchor) -> Option<[VertexId; 2]> {
    match (left, right) {
        (MeshAnchor::EdgeSpan(a, b), MeshAnchor::FaceInterior(face))
        | (MeshAnchor::FaceInterior(face), MeshAnchor::EdgeSpan(a, b)) => {
            (face_contains_vertex(mesh, face, a) && face_contains_vertex(mesh, face, b))
                .then_some([a, b])
        }
        _ => None,
    }
}

fn reciprocal_edge_crossing(
    mesh_a: &Mesh,
    left_a: MeshAnchor,
    right_a: MeshAnchor,
    mesh_b: &Mesh,
    left_b: MeshAnchor,
    right_b: MeshAnchor,
) -> Option<[f64; 3]> {
    let edge_a = edge_face_incidence(mesh_a, left_a, right_a)?;
    let edge_b = edge_face_incidence(mesh_b, left_b, right_b)?;
    edge_edge_intersection(mesh_a, edge_a, mesh_b, edge_b)
}

fn edges_are_non_parallel(
    mesh_a: &Mesh,
    edge_a: [VertexId; 2],
    mesh_b: &Mesh,
    edge_b: [VertexId; 2],
) -> bool {
    let [Some(a), Some(b), Some(c), Some(d)] = [
        mesh_a.vertex_position(edge_a[0]).copied().map(promote),
        mesh_a.vertex_position(edge_a[1]).copied().map(promote),
        mesh_b.vertex_position(edge_b[0]).copied().map(promote),
        mesh_b.vertex_position(edge_b[1]).copied().map(promote),
    ] else {
        return false;
    };
    cross(sub(b, a), sub(d, c)) != [0.0; 3]
}

/// Whether two endpoint descriptions that narrow to the same mesh position
/// have compatible topology on both operands.
///
/// Usually each operand can establish a shared carrier independently. Near a
/// crossing of two mesh edges, however, adjacent triangle pairs can report
/// the reciprocal descriptions `(edge A, face B)` and `(face A, edge B)`.
/// Their reconstructed point may be a few f64 bits off both edges. If each
/// named edge belongs to the other's named face and the edges are
/// non-parallel, the two descriptions still have exactly one common
/// topological crossing. This is an exact incidence proof, not a distance
/// tolerance.
fn endpoint_carriers_establish_identity(
    mesh_a: &Mesh,
    left_a: MeshAnchor,
    right_a: MeshAnchor,
    mesh_b: &Mesh,
    left_b: MeshAnchor,
    right_b: MeshAnchor,
    position: [f32; 3],
) -> bool {
    if anchors_share_carrier(mesh_a, left_a, right_a, position)
        && anchors_share_carrier(mesh_b, left_b, right_b, position)
    {
        return true;
    }

    reciprocal_edge_crossing(mesh_a, left_a, right_a, mesh_b, left_b, right_b).is_some()
}

/// Closes incidence equivalence after every endpoint has been observed.
///
/// Two reciprocal edge/face reports can first combine into an edge/edge
/// representative and only then prove identity with a third report on an
/// adjacent face. Streaming alone cannot revisit that earlier report. This
/// worklist revisits only classes that gain sharper topology. Late closure is
/// limited to classes already at the same eventual `f32` mesh position: the
/// explicit observations needed to justify different-position welding were
/// available during streaming, while propagating that relation afterward can
/// erase legitimate branches at exact multi-cut vertices. A successful join
/// requeues only its newly sharpened representative, so work grows with the
/// number of discovered equivalences instead of repeatedly scanning the
/// whole graph.
fn reconcile_incidence_classes(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &mut IntersectionGraph,
    weld_index: &mut WeldIndex,
) {
    let mut pending: Vec<u32> = (0..graph.vertices.len())
        .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
        .filter(|&index| weld_index.root(index) == index)
        .collect();
    let mut cursor = 0;
    while let Some(&queued) = pending.get(cursor) {
        cursor += 1;
        let mut root = weld_index.root(queued);
        if root != queued {
            continue;
        }
        let vertex = &graph.vertices[root as usize];
        let candidates = incidence_candidates(
            mesh_a,
            mesh_b,
            weld_index,
            vertex.anchor_a,
            vertex.anchor_b,
            FaceId::OUTSIDE,
            FaceId::OUTSIDE,
        );
        let mut changed = false;
        for candidate in candidates {
            let left = weld_index.root(root);
            let right = weld_index.root(candidate.vertex);
            if left == right
                || narrow(graph.vertices[left as usize].position)
                    != narrow(graph.vertices[right as usize].position)
            {
                continue;
            }
            root = weld_index.join(graph, left, right);
            changed = true;
        }
        if changed {
            pending.push(root);
        }
    }
}

/// Rewrites provisional equivalence classes into the public graph.
///
/// A late edge/edge observation may join vertices that already have graph
/// edges. Rebuilding once after the stream is both simpler and safer than
/// mutating adjacency incrementally: self-edges become touch points and
/// duplicate edges merge their face-pair attribution deterministically.
fn compact_welded_graph(graph: &mut IntersectionGraph, weld_index: &WeldIndex) {
    let mut remap = alloc::vec![u32::MAX; graph.vertices.len()];
    let mut vertices = Vec::new();
    for old in 0..graph.vertices.len() {
        let old = u32::try_from(old).unwrap_or(u32::MAX);
        let root = weld_index.root(old);
        if remap[root as usize] == u32::MAX {
            remap[root as usize] = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            vertices.push(graph.vertices[root as usize].clone());
        }
        remap[old as usize] = remap[root as usize];
    }

    let mut touch_points: Vec<u32> = core::mem::take(&mut graph.touch_points)
        .into_iter()
        .map(|vertex| remap[vertex as usize])
        .collect();
    let mut edges = Vec::<GraphEdge>::new();
    let mut edge_index = HashMap::<[u32; 2], u32>::new();
    for edge in core::mem::take(&mut graph.edges) {
        let [a, b] = edge.vertices.map(|vertex| remap[vertex as usize]);
        if a == b {
            touch_points.push(a);
            continue;
        }
        let key = [a.min(b), a.max(b)];
        if let Some(&index) = edge_index.get(&key) {
            edges[index as usize].crossings.extend(edge.crossings);
        } else {
            edge_index.insert(key, u32::try_from(edges.len()).unwrap_or(u32::MAX));
            edges.push(GraphEdge {
                vertices: key,
                crossings: edge.crossings,
            });
        }
    }
    graph.vertices = vertices;
    graph.edges = edges;
    graph.touch_points = touch_points;
}

/// Whether two anchors can name the same stored point without crossing a
/// topological boundary on `mesh`.
///
/// Narrowing can move two independently evaluated endpoints into one f32
/// cell. They are weld-compatible only when their provenance closures meet:
/// a vertex belongs to the other anchor, an edge span belongs to the face, or
/// two faces share the boundary carrying the stored point. This deliberately
/// keeps coincident points on disconnected or non-adjacent surface regions
/// distinct.
fn anchors_share_carrier(
    mesh: &Mesh,
    left: MeshAnchor,
    right: MeshAnchor,
    position: [f32; 3],
) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (MeshAnchor::Vertex(vertex), anchor) | (anchor, MeshAnchor::Vertex(vertex)) => {
            mesh.vertex_position(vertex) == Some(&position)
                && anchor_contains_vertex(mesh, anchor, vertex)
        }
        (MeshAnchor::EdgeSpan(a, b), MeshAnchor::EdgeSpan(c, d)) => {
            spans_share_carrier(mesh, [a, b], [c, d], position)
        }
        (MeshAnchor::EdgeSpan(a, b), MeshAnchor::FaceInterior(face))
        | (MeshAnchor::FaceInterior(face), MeshAnchor::EdgeSpan(a, b)) => {
            face_contains_vertex(mesh, face, a)
                && face_contains_vertex(mesh, face, b)
                && span_contains_position(mesh, [a, b], position)
        }
        (MeshAnchor::FaceInterior(a), MeshAnchor::FaceInterior(b)) => {
            shared_boundary_contains_position(mesh, a, b, position)
        }
    }
}

/// Whether `position` lies on the actual boundary shared by two mesh faces.
///
/// Adjacency alone is insufficient: distinct intersection endpoints can
/// round to the same f32 point near a common edge while remaining on opposite
/// face interiors. The point must materialize on that shared edge (or vertex)
/// before their provenance closures meet.
fn shared_boundary_contains_position(
    mesh: &Mesh,
    left: FaceId,
    right: FaceId,
    position: [f32; 3],
) -> bool {
    if mesh
        .face_loop(left)
        .filter_map(|half_edge| mesh.to_vertex(half_edge))
        .any(|vertex| {
            face_contains_vertex(mesh, right, vertex)
                && mesh.vertex_position(vertex) == Some(&position)
        })
    {
        return true;
    }
    mesh.face_loop(left).any(|half_edge| {
        let (Some(a), Some(b)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge)) else {
            return false;
        };
        face_contains_vertex(mesh, right, a)
            && face_contains_vertex(mesh, right, b)
            && span_contains_position(mesh, [a, b], position)
    })
}

fn span_contains_position(mesh: &Mesh, span: [VertexId; 2], position: [f32; 3]) -> bool {
    let [Some(a), Some(b)] = [
        mesh.vertex_position(span[0]).copied().map(promote),
        mesh.vertex_position(span[1]).copied().map(promote),
    ] else {
        return false;
    };
    let point = promote(position);
    cross(sub(b, a), sub(point, a)) == [0.0; 3] && span_bounds_contain(a, b, point)
}

/// Whether two edge or triangulation spans overlap at `position` as one
/// topological carrier.
///
/// Split faces can replace one original edge by nested spans such as
/// `(a, b)` and `(a, midpoint)`. Sharing the endpoint is not enough—two
/// ordinary edges can meet there—so points away from that endpoint weld only
/// when both stored spans are exactly collinear and contain the point.
fn spans_share_carrier(
    mesh: &Mesh,
    left: [VertexId; 2],
    right: [VertexId; 2],
    position: [f32; 3],
) -> bool {
    let shared = left
        .into_iter()
        .find(|vertex| *vertex == right[0] || *vertex == right[1]);
    let Some(shared) = shared else {
        return false;
    };
    if mesh.vertex_position(shared) == Some(&position) {
        return true;
    }
    let [Some(left_a), Some(left_b), Some(right_a), Some(right_b)] = [
        mesh.vertex_position(left[0]).copied().map(promote),
        mesh.vertex_position(left[1]).copied().map(promote),
        mesh.vertex_position(right[0]).copied().map(promote),
        mesh.vertex_position(right[1]).copied().map(promote),
    ] else {
        return false;
    };
    let left_direction = sub(left_b, left_a);
    let right_direction = sub(right_b, right_a);
    if cross(left_direction, right_direction) != [0.0; 3] {
        return false;
    }
    let position = promote(position);
    span_bounds_contain(left_a, left_b, position) && span_bounds_contain(right_a, right_b, position)
}

fn span_bounds_contain(a: [f64; 3], b: [f64; 3], point: [f64; 3]) -> bool {
    (0..3).all(|axis| {
        let (min, max) = if a[axis] <= b[axis] {
            (a[axis], b[axis])
        } else {
            (b[axis], a[axis])
        };
        (min..=max).contains(&point[axis])
    })
}

fn anchor_contains_vertex(mesh: &Mesh, anchor: MeshAnchor, vertex: VertexId) -> bool {
    match anchor {
        MeshAnchor::Vertex(candidate) => candidate == vertex,
        MeshAnchor::EdgeSpan(a, b) => a == vertex || b == vertex,
        MeshAnchor::FaceInterior(face) => face_contains_vertex(mesh, face, vertex),
    }
}

fn face_contains_vertex(mesh: &Mesh, face: FaceId, vertex: VertexId) -> bool {
    mesh.face_loop(face)
        .filter_map(|half_edge| mesh.to_vertex(half_edge))
        .any(|candidate| candidate == vertex)
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

fn endpoint_anchor_rank(vertex: &GraphVertex) -> u8 {
    anchor_rank(&vertex.anchor_a) + anchor_rank(&vertex.anchor_b)
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
    fn unrelated_endpoints_that_narrow_to_one_point_remain_distinct() {
        // Equal eventual f32 coordinates do not establish topological identity:
        // two unrelated face-pair crossings may be distinct seams that happen
        // to fall in the same representable cell.
        let mesh = cube([0.0, 0.0, 0.0]);
        let faces: Vec<_> = mesh.faces().take(2).collect();
        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let first = weld_endpoint(
            &mesh,
            &mesh,
            &mut graph,
            &mut weld_index,
            [1.0, 0.0, 0.0],
            MeshAnchor::FaceInterior(faces[0]),
            MeshAnchor::FaceInterior(faces[0]),
            faces[0],
            faces[0],
        );
        let second = weld_endpoint(
            &mesh,
            &mesh,
            &mut graph,
            &mut weld_index,
            [1.0 + f64::EPSILON, -0.0, 0.0],
            MeshAnchor::FaceInterior(faces[1]),
            MeshAnchor::FaceInterior(faces[1]),
            faces[1],
            faces[1],
        );

        assert_ne!(first.index, second.index);
        assert_eq!(graph.vertices.len(), 2);
        assert_eq!(graph.stats.welded_endpoints, 0);
    }

    #[test]
    fn compatible_endpoints_that_narrow_to_one_point_are_welded() {
        // One triangle pair can place an endpoint on a face diagonal while an
        // adjacent pair calls the same point face-interior. The shared face is
        // the topological evidence that lets the equal stored coordinates weld.
        let mesh = cube([0.0, 0.0, 0.0]);
        let faces: Vec<_> = mesh.faces().take(2).collect();
        let edge: Vec<_> = mesh
            .face_loop(faces[0])
            .filter_map(|half_edge| mesh.to_vertex(half_edge))
            .take(2)
            .collect();
        let a = *mesh.vertex_position(edge[0]).expect("live edge vertex");
        let b = *mesh.vertex_position(edge[1]).expect("live edge vertex");
        let midpoint = [
            f64::from((a[0] + b[0]) * 0.5),
            f64::from((a[1] + b[1]) * 0.5),
            f64::from((a[2] + b[2]) * 0.5),
        ];
        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let first = weld_endpoint(
            &mesh,
            &mesh,
            &mut graph,
            &mut weld_index,
            midpoint,
            MeshAnchor::EdgeSpan(edge[0], edge[1]),
            MeshAnchor::FaceInterior(faces[1]),
            faces[0],
            faces[1],
        );
        let second = weld_endpoint(
            &mesh,
            &mesh,
            &mut graph,
            &mut weld_index,
            [midpoint[0] + f64::EPSILON, midpoint[1], -0.0],
            MeshAnchor::FaceInterior(faces[0]),
            MeshAnchor::FaceInterior(faces[1]),
            faces[0],
            faces[1],
        );

        assert_eq!(first.index, second.index);
        assert_eq!(graph.vertices.len(), 1);
        assert_eq!(graph.stats.welded_endpoints, 1);
    }

    #[test]
    fn edge_incidence_recovers_plane_solve_drift_without_a_tolerance() {
        // Adjacent triangle pairs can describe one endpoint as edge/edge and
        // edge/face. The face-plane solve may drift in a dependent coordinate,
        // but a source edge crossing that face transversely has one possible
        // intersection, so incidence proves identity without a spatial epsilon.
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, -0.5, 0.0]);
        let vertex_at = |mesh: &Mesh, position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture vertex exists")
        };
        let edge_a = [
            vertex_at(&mesh_a, [0.0, 0.0, 0.0]),
            vertex_at(&mesh_a, [1.0, 0.0, 0.0]),
        ];
        let edge_b = [
            vertex_at(&mesh_b, [0.5, -0.5, 0.0]),
            vertex_at(&mesh_b, [0.5, 0.5, 0.0]),
        ];
        let face_b = mesh_b
            .faces()
            .find(|&face| {
                mesh_b
                    .face_loop(face)
                    .filter_map(|half_edge| mesh_b.to_vertex(half_edge))
                    .all(|vertex| mesh_b.vertex_position(vertex).is_some_and(|p| p[0] == 0.5))
            })
            .expect("crossing edge belongs to the x-min face");
        let face_a = mesh_a.faces().next().expect("cube has a face");
        let point = [0.5, 0.0, 0.0];
        let drifted = [0.5, 0.0, 1.0e-6];

        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let first = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            point,
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            face_a,
            face_b,
        );
        let second = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            drifted,
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::FaceInterior(face_b),
            face_a,
            face_b,
        );

        assert_eq!(first.index, second.index);
        assert!(second.recovered_from_incidence);
        assert_eq!(graph.vertices.len(), 1);
    }

    #[test]
    fn matched_endpoint_still_joins_reciprocal_incidence_classes() {
        // Two edge/face reports can exist at adjacent stored positions before
        // an edge/edge report supplies the bridge between them. The bridge
        // first matches one class by stored position, but must also consume
        // its incidence proof and join the other class; otherwise endpoint
        // streaming order leaves a dangling cut.
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, -0.5, 0.0]);
        let vertex_at = |mesh: &Mesh, position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture vertex exists")
        };
        let edge_a = [
            vertex_at(&mesh_a, [0.0, 0.0, 0.0]),
            vertex_at(&mesh_a, [1.0, 0.0, 0.0]),
        ];
        let edge_b = [
            vertex_at(&mesh_b, [0.5, -0.5, 0.0]),
            vertex_at(&mesh_b, [0.5, 0.5, 0.0]),
        ];
        let containing_faces = |mesh: &Mesh, edge: [VertexId; 2]| {
            mesh.faces()
                .filter(|&face| {
                    face_contains_vertex(mesh, face, edge[0])
                        && face_contains_vertex(mesh, face, edge[1])
                })
                .collect::<Vec<_>>()
        };
        let faces_a = containing_faces(&mesh_a, edge_a);
        let faces_b = containing_faces(&mesh_b, edge_b);
        assert_eq!(faces_a.len(), 2);
        assert_eq!(faces_b.len(), 2);

        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let first = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 0.0],
            MeshAnchor::FaceInterior(faces_a[0]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            faces_a[0],
            faces_b[0],
        );
        let second = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 1.0e-6],
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::FaceInterior(faces_b[1]),
            faces_a[1],
            faces_b[1],
        );
        assert_ne!(weld_index.root(first.index), weld_index.root(second.index));

        let bridge = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 0.0],
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            faces_a[0],
            faces_b[1],
        );

        assert!(bridge.recovered_from_incidence);
        assert_eq!(weld_index.root(first.index), weld_index.root(second.index));
        assert_eq!(weld_index.root(first.index), weld_index.root(bridge.index));
    }

    #[test]
    fn parallel_edge_incidence_does_not_claim_unique_identity() {
        // Collinear edge overlap has more than one possible point. Reusing
        // one edge/face incidence must not collapse distinct locations on
        // that overlap as though two non-parallel edges had crossed once.
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.0, 0.0, 0.0]);
        let vertex_at = |mesh: &Mesh, position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture vertex exists")
        };
        let edge_a = [
            vertex_at(&mesh_a, [0.0, 0.0, 0.0]),
            vertex_at(&mesh_a, [1.0, 0.0, 0.0]),
        ];
        let edge_b = [
            vertex_at(&mesh_b, [0.0, 0.0, 0.0]),
            vertex_at(&mesh_b, [1.0, 0.0, 0.0]),
        ];
        let face_b = mesh_b
            .faces()
            .find(|&face| {
                mesh_b
                    .face_loop(face)
                    .filter_map(|half_edge| mesh_b.to_vertex(half_edge))
                    .all(|vertex| mesh_b.vertex_position(vertex).is_some_and(|p| p[2] == 0.0))
            })
            .expect("crossing edge belongs to the z-min face");
        let face_a = mesh_a.faces().next().expect("cube has a face");

        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let first = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.25, 0.0, 0.0],
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            face_a,
            face_b,
        );
        let second = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.75, 0.0, 0.0],
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::FaceInterior(face_b),
            face_a,
            face_b,
        );

        assert_ne!(first.index, second.index);
        assert!(!second.recovered_from_incidence);
        assert_eq!(graph.vertices.len(), 2);
    }

    #[test]
    fn late_edge_evidence_reconciles_earlier_face_reports() {
        // A complete edge/edge identity can emerge only after two reciprocal
        // edge/face reports have welded. A still-earlier report on the same
        // edge must then join that class even though streaming order gave it
        // no usable edge evidence at the time.
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([0.5, -0.5, 0.0]);
        let vertex_at = |mesh: &Mesh, position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture vertex exists")
        };
        let edge_a = [
            vertex_at(&mesh_a, [0.0, 0.0, 0.0]),
            vertex_at(&mesh_a, [1.0, 0.0, 0.0]),
        ];
        let edge_b = [
            vertex_at(&mesh_b, [0.5, -0.5, 0.0]),
            vertex_at(&mesh_b, [0.5, 0.5, 0.0]),
        ];
        let containing_faces = |mesh: &Mesh, edge: [VertexId; 2]| {
            mesh.faces()
                .filter(|&face| {
                    face_contains_vertex(mesh, face, edge[0])
                        && face_contains_vertex(mesh, face, edge[1])
                })
                .collect::<Vec<_>>()
        };
        let faces_a = containing_faces(&mesh_a, edge_a);
        let faces_b = containing_faces(&mesh_b, edge_b);
        assert_eq!(faces_a.len(), 2);
        assert_eq!(faces_b.len(), 2);

        let mut graph = IntersectionGraph::default();
        let mut weld_index = WeldIndex::default();
        let earlier = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 2.0e-6],
            MeshAnchor::FaceInterior(faces_a[1]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            faces_a[1],
            faces_b[0],
        );
        let adjacent = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 2.0e-6],
            MeshAnchor::FaceInterior(faces_a[0]),
            MeshAnchor::EdgeSpan(edge_b[0], edge_b[1]),
            faces_a[0],
            faces_b[0],
        );
        let edge_report = weld_endpoint(
            &mesh_a,
            &mesh_b,
            &mut graph,
            &mut weld_index,
            [0.5, 0.0, 0.0],
            MeshAnchor::EdgeSpan(edge_a[0], edge_a[1]),
            MeshAnchor::FaceInterior(faces_b[0]),
            faces_a[0],
            faces_b[0],
        );

        assert_eq!(
            weld_index.root(edge_report.index),
            weld_index.root(adjacent.index)
        );
        assert_ne!(
            weld_index.root(earlier.index),
            weld_index.root(edge_report.index)
        );
        reconcile_incidence_classes(&mesh_a, &mesh_b, &mut graph, &mut weld_index);
        assert_eq!(
            weld_index.root(earlier.index),
            weld_index.root(edge_report.index)
        );
    }

    #[test]
    fn adjacent_face_anchors_only_weld_on_their_shared_boundary() {
        // Face adjacency names a possible carrier, not the whole carrier:
        // equal rounded points in the two face interiors remain distinct
        // unless the stored point lies on their common edge.
        let mesh = cube([0.0, 0.0, 0.0]);
        let faces: Vec<_> = mesh.faces().collect();
        let (left, right, left_only) = faces
            .iter()
            .enumerate()
            .find_map(|(index, &left)| {
                let left_vertices: Vec<_> = mesh
                    .face_loop(left)
                    .filter_map(|half_edge| mesh.to_vertex(half_edge))
                    .collect();
                faces[index + 1..].iter().find_map(|&right| {
                    let shared = left_vertices
                        .iter()
                        .filter(|&&vertex| face_contains_vertex(&mesh, right, vertex))
                        .count();
                    let left_only = left_vertices
                        .iter()
                        .copied()
                        .find(|&vertex| !face_contains_vertex(&mesh, right, vertex));
                    (shared == 2).then_some((left, right, left_only?))
                })
            })
            .expect("cube has adjacent faces and a non-shared corner");
        let position = *mesh
            .vertex_position(left_only)
            .expect("live non-shared corner");

        assert!(!anchors_share_carrier(
            &mesh,
            MeshAnchor::FaceInterior(left),
            MeshAnchor::FaceInterior(right),
            position,
        ));
    }

    #[test]
    fn edge_reconstruction_requires_one_shared_stored_point() {
        // Projected line crossings are not enough in 3D: genuinely crossing
        // source edges reconstruct their common f32 point, while a parallel
        // projection of two skew edges must retain the narrow-phase fallback.
        let horizontal = cube([0.0, 0.0, 0.0]);
        let crossing = cube([0.5, -0.5, 0.0]);
        let skew = cube([0.5, -0.5, 0.25]);
        let vertex_at = |mesh: &Mesh, position: [f32; 3]| {
            mesh.vertices()
                .find(|&vertex| mesh.vertex_position(vertex) == Some(&position))
                .expect("fixture vertex exists")
        };
        let horizontal_edge = [
            vertex_at(&horizontal, [0.0, 0.0, 0.0]),
            vertex_at(&horizontal, [1.0, 0.0, 0.0]),
        ];
        let crossing_edge = [
            vertex_at(&crossing, [0.5, -0.5, 0.0]),
            vertex_at(&crossing, [0.5, 0.5, 0.0]),
        ];
        let skew_edge = [
            vertex_at(&skew, [0.5, -0.5, 0.25]),
            vertex_at(&skew, [0.5, 0.5, 0.25]),
        ];

        assert_eq!(
            edge_edge_intersection(&horizontal, horizontal_edge, &crossing, crossing_edge)
                .map(narrow),
            Some([0.5, 0.0, 0.0])
        );
        assert_eq!(
            edge_edge_intersection(&horizontal, horizontal_edge, &skew, skew_edge),
            None
        );
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
