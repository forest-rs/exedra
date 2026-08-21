// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic rounding (fillet/chamfer) of sharp provenance edges.
//!
//! [`round_sharp_edges`] replaces every edge whose [`attr::EDGE_SHARPNESS`]
//! meets a policy threshold with a rounded strip: the flanking faces shrink
//! to the rolling-ball tangency lines, a quad strip fills the gap (a single
//! flat bevel in chamfer mode), and junctions where three rounded chains
//! meet at a convex trihedral corner receive a spherical corner patch.
//! Producers that tag feature edges — constructive tessellation tags crease
//! edges, the boolean pipeline tags seam rings — get mesh-level fillets
//! without re-authoring geometry.
//!
//! # Envelope (v1)
//!
//! Supported: open chains ending on a single transversal face or at a
//! convex trihedral corner of three rounded chains; closed rings (a drilled
//! rim); gently bent chains (per-vertex averaged frames); per-edge varying
//! flank faces (faceted walls). Everything outside that envelope is a typed
//! [`RoundError`] and the mesh is left byte-identical: concave edges,
//! junction valence other than one, two, or three, non-trihedral corners,
//! chain turns beyond [`RoundPolicy::max_tangent_turn`], non-planar
//! affected faces, and rewrites that would invert or degenerate a face.
//!
//! Two quality caveats are deliberate v1 scope: averaged per-vertex frames
//! leave faceted flank faces (a polygonal drill wall) slightly non-planar
//! after rewriting, and strips of chains curved tighter than the offset are
//! not detected as self-overlapping — callers keep the offset small against
//! the local curvature radius.
//!
//! # Determinism
//!
//! Chains, substitutions, and emitted faces derive from ascending stable
//! ids; band counts derive from integer ceilings over backend `acos`; all
//! constructions run in f64 and narrow to f32 exactly once per new vertex.
//! Output is deterministic for a fixed math backend, target, and input —
//! the same `std`/`libm` backend policy the primitive generators document.

mod geom;
#[cfg(test)]
mod tests;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

use crate::math::FloatExt;
use crate::op::{
    add_face, add_vertex, delete_faces, delete_vertices, set_edge_seam, set_edge_sharpness,
    set_face_region,
};
use crate::{DeletePolicy, FaceId, HalfEdgeId, Mesh, VertexId, attr};

use geom::{
    Plane, add, arc_points, cross, dot, line_intersection, narrow, newell, normalize, promote,
    scale, solve3, sub,
};

/// The rounding profile applied along each sharp chain.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum RoundKind {
    /// A circular fillet of the given rolling-ball radius.
    Fillet {
        /// Rolling-ball radius; positive and finite.
        radius: f64,
    },
    /// A single flat bevel offset by the given in-plane setback.
    Chamfer {
        /// In-plane setback on each flanking face; positive and finite.
        setback: f64,
    },
}

/// Policy for one rounding pass.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RoundPolicy {
    /// Fillet or chamfer profile.
    pub kind: RoundKind,
    /// Explicit fillet band count; `None` derives one from
    /// [`RoundPolicy::chord_tolerance`]. Chamfers always use one band.
    pub segments: Option<u32>,
    /// Maximum chord deviation used to derive fillet band counts.
    pub chord_tolerance: f64,
    /// Edges with [`attr::EDGE_SHARPNESS`] at or above this value round.
    pub sharpness_threshold: f32,
    /// [`attr::FACE_REGION`] assigned to new strip and patch faces.
    pub region: Option<u32>,
    /// Maximum absolute deviation for affected-face planarity and end-face
    /// containment checks.
    pub max_planar_deviation: f64,
    /// Maximum turn angle (radians) between consecutive chain edges.
    pub max_tangent_turn: f64,
}

impl RoundPolicy {
    /// A fillet policy with default selection and tolerance settings.
    #[must_use]
    pub fn fillet(radius: f64) -> Self {
        Self {
            kind: RoundKind::Fillet { radius },
            segments: None,
            chord_tolerance: radius / 20.0,
            sharpness_threshold: 0.5,
            region: None,
            max_planar_deviation: 1e-3,
            max_tangent_turn: 0.7,
        }
    }

    /// A chamfer policy with default selection and tolerance settings.
    #[must_use]
    pub fn chamfer(setback: f64) -> Self {
        Self {
            kind: RoundKind::Chamfer { setback },
            segments: None,
            chord_tolerance: setback / 20.0,
            sharpness_threshold: 0.5,
            region: None,
            max_planar_deviation: 1e-3,
            max_tangent_turn: 0.7,
        }
    }

    fn offset(&self) -> f64 {
        match self.kind {
            RoundKind::Fillet { radius } => radius,
            RoundKind::Chamfer { setback } => setback,
        }
    }
}

/// Structured rounding failure.
///
/// Every failure is detected before the staged rewrite is committed, so a
/// failed pass leaves the mesh byte-identical.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoundError {
    /// The policy carries a non-positive or non-finite parameter.
    InvalidPolicy {
        /// Which parameter was rejected.
        detail: &'static str,
    },
    /// A selected edge borders the outside (no second interior face).
    BoundaryEdge {
        /// Smaller endpoint index.
        a: u32,
        /// Larger endpoint index.
        b: u32,
    },
    /// A face taking part in the rewrite deviates from its fitted plane by
    /// more than [`RoundPolicy::max_planar_deviation`].
    NonPlanarFace {
        /// The offending face index.
        face: u32,
    },
    /// A selected edge is concave (material dihedral above a flat angle);
    /// v1 rounds convex edges only.
    ConcaveEdge {
        /// Smaller endpoint index.
        a: u32,
        /// Larger endpoint index.
        b: u32,
    },
    /// A selected edge's geometry degenerates (parallel or anti-parallel
    /// flank planes, zero-length direction, failed arc construction).
    DegenerateEdge {
        /// Smaller endpoint index.
        a: u32,
        /// Larger endpoint index.
        b: u32,
    },
    /// The offset would invert or degenerate a rewritten face.
    ClearanceExceeded {
        /// The offending face index.
        face: u32,
    },
    /// A junction outside the v1 envelope: sharp valence of four or more,
    /// a three-chain corner that is not a convex trihedral corner, or a
    /// chain turning more than [`RoundPolicy::max_tangent_turn`].
    UnsupportedJunction {
        /// The offending vertex index.
        vertex: u32,
    },
    /// An open chain end without exactly one containing end face.
    UnsupportedEnd {
        /// The offending vertex index.
        vertex: u32,
    },
    /// The neighborhood cannot be rewritten consistently (conflicting
    /// substitutions, ambiguous side classification, or a rewrite that
    /// would break manifoldness).
    UnsupportedTopology {
        /// What was rejected.
        detail: &'static str,
    },
    /// An application-stage edit failed after planning validated it.
    ///
    /// This indicates a kernel bug. The staged rewrite is discarded, leaving
    /// the caller's mesh byte-identical.
    Internal {
        /// The failing stage.
        detail: &'static str,
    },
}

impl fmt::Display for RoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy { detail } => write!(f, "invalid rounding policy: {detail}"),
            Self::BoundaryEdge { a, b } => {
                write!(f, "sharp edge ({a}, {b}) borders the outside")
            }
            Self::NonPlanarFace { face } => {
                write!(f, "face {face} deviates from its fitted plane")
            }
            Self::ConcaveEdge { a, b } => write!(f, "sharp edge ({a}, {b}) is concave"),
            Self::DegenerateEdge { a, b } => {
                write!(f, "sharp edge ({a}, {b}) has degenerate geometry")
            }
            Self::ClearanceExceeded { face } => {
                write!(f, "offset exceeds clearance on face {face}")
            }
            Self::UnsupportedJunction { vertex } => {
                write!(f, "unsupported sharp-edge junction at vertex {vertex}")
            }
            Self::UnsupportedEnd { vertex } => {
                write!(f, "unsupported open chain end at vertex {vertex}")
            }
            Self::UnsupportedTopology { detail } => {
                write!(f, "unsupported rounding topology: {detail}")
            }
            Self::Internal { detail } => write!(f, "internal rounding failure: {detail}"),
        }
    }
}

impl core::error::Error for RoundError {}

/// Work counters for one rounding pass.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct RoundStats {
    /// Chains rounded (open and closed).
    pub chains: u32,
    /// Closed-ring chains among them.
    pub closed_chains: u32,
    /// Trihedral corner patches emitted.
    pub corners: u32,
    /// Strip quads emitted.
    pub strip_faces: u32,
    /// Corner patch faces emitted.
    pub patch_faces: u32,
    /// Pre-existing faces rewritten.
    pub rewritten_faces: u32,
    /// Exactly-degenerate sliver faces that vanished with the rewrite
    /// (their vertices were coincident seam duplicates).
    pub vanished_faces: u32,
    /// Consumed chain vertices removed.
    pub removed_vertices: u32,
    /// New vertices added.
    pub added_vertices: u32,
    /// Largest band count used by any chain.
    pub max_segments: u32,
}

/// Rounds every edge whose sharpness meets the policy threshold.
///
/// Returns work counters; a pass that selects no edges is a successful
/// no-op with zeroed counters.
///
/// # Errors
///
/// Any configuration outside the documented v1 envelope fails typed with
/// the mesh left byte-identical; see [`RoundError`].
pub fn round_sharp_edges(mesh: &mut Mesh, policy: &RoundPolicy) -> Result<RoundStats, RoundError> {
    validate_policy(policy)?;
    let Some(plan) = plan(mesh, policy)? else {
        return Ok(RoundStats::default());
    };
    apply(mesh, plan)
}

fn validate_policy(policy: &RoundPolicy) -> Result<(), RoundError> {
    let offset = policy.offset();
    if !(offset.is_finite() && offset > 0.0) {
        return Err(RoundError::InvalidPolicy {
            detail: "radius/setback must be positive and finite",
        });
    }
    if !(policy.chord_tolerance.is_finite() && policy.chord_tolerance > 0.0) {
        return Err(RoundError::InvalidPolicy {
            detail: "chord_tolerance must be positive and finite",
        });
    }
    if !(policy.max_planar_deviation.is_finite() && policy.max_planar_deviation > 0.0) {
        return Err(RoundError::InvalidPolicy {
            detail: "max_planar_deviation must be positive and finite",
        });
    }
    if !(policy.max_tangent_turn.is_finite() && policy.max_tangent_turn > 0.0) {
        return Err(RoundError::InvalidPolicy {
            detail: "max_tangent_turn must be positive and finite",
        });
    }
    if policy.segments == Some(0) {
        return Err(RoundError::InvalidPolicy {
            detail: "segments must be at least one",
        });
    }
    Ok(())
}

/// A loop entry of a planned face: a surviving vertex or a new point.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Tok {
    Old(VertexId),
    New(u32),
}

/// One selected sharp edge in canonical orientation.
#[derive(Copy, Clone, Debug)]
struct SelEdge {
    a: VertexId,
    b: VertexId,
    /// Face left of the directed edge `a -> b`.
    left: FaceId,
    /// Face right of the directed edge `a -> b`.
    right: FaceId,
}

/// One chain edge in traversal orientation.
#[derive(Copy, Clone, Debug)]
struct DirEdge {
    a: VertexId,
    b: VertexId,
    left: FaceId,
    right: FaceId,
    /// Sweep angle between the flank normals.
    sweep: f64,
}

#[derive(Clone, Debug)]
struct Chain {
    verts: Vec<VertexId>,
    /// Coincident mesh vertices merged into each chain position: boolean
    /// seams carry distinct vertices at identical narrowed positions, whose
    /// zero-length connecting edges collapse into one cross-section.
    aliases: Vec<Vec<VertexId>>,
    edges: Vec<DirEdge>,
    closed: bool,
    segments: u32,
    /// Cross-section point ids per chain vertex, ordered left to right.
    sections: Vec<Vec<u32>>,
    /// Averaged (left, right) flank normals per chain vertex, where the
    /// vertex owns a per-vertex frame (interior and open-end vertices).
    frames: Vec<Option<([f64; 3], [f64; 3])>>,
}

impl Chain {
    fn vertex_count(&self) -> usize {
        self.verts.len()
    }

    fn edges_around(&self, index: usize) -> (Option<usize>, Option<usize>) {
        let count = self.edges.len();
        if self.closed {
            (Some((index + count - 1) % count), Some(index % count))
        } else {
            (index.checked_sub(1), (index < count).then_some(index))
        }
    }

    fn end_edge(&self, at_end: bool) -> DirEdge {
        if at_end {
            self.edges[self.edges.len() - 1]
        } else {
            self.edges[0]
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum VertexKind {
    Interior,
    OpenEnd,
    Corner,
}

#[derive(Clone, Debug)]
struct NewFace {
    entries: Vec<Tok>,
    region: Option<u32>,
}

#[derive(Clone, Debug)]
struct Plan {
    points: Vec<[f64; 3]>,
    faces: Vec<NewFace>,
    affected: Vec<FaceId>,
    consumed: Vec<VertexId>,
    edge_attrs: Vec<(Tok, Tok, Option<f32>, Option<bool>)>,
    stats: RoundStats,
}

/// One planned trihedral corner: its vertex, the fillet ball center, and
/// the adjoining `(chain index, at-end)` pairs for the patch ring walk.
type CornerPlan = (VertexId, Option<[f64; 3]>, Vec<(usize, bool)>);

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Subst {
    Point(u32),
    /// End-face splice: `(chain index, vertex index)`, oriented at loop
    /// build via the incoming-twin rule.
    Splice(u32, u32),
}

struct Planner<'a> {
    mesh: &'a Mesh,
    policy: &'a RoundPolicy,
    half_edge_of: BTreeMap<(VertexId, VertexId), HalfEdgeId>,
    planes: BTreeMap<FaceId, Plane>,
    points: Vec<[f64; 3]>,
    subst: BTreeMap<(FaceId, VertexId), Subst>,
    faces: Vec<NewFace>,
    stats: RoundStats,
}

/// Newell reliability floor: a face whose Newell norm falls below this
/// fraction of its squared perimeter is a sliver whose own normal is noise
/// (boolean cut seams reinsert exactly-collinear vertices, so rim flanks
/// are full of them); such faces borrow a verified coplanar neighbor's
/// plane instead.
const SLIVER_NEWELL_FLOOR: f64 = 1e-4;

fn plan(mesh: &Mesh, policy: &RoundPolicy) -> Result<Option<Plan>, RoundError> {
    // Directed half-edge lookup over interior faces.
    let mut half_edge_of = BTreeMap::new();
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
            else {
                continue;
            };
            half_edge_of.insert((from, to), half_edge);
        }
    }

    // Selection: canonical sharp edges at or above the threshold.
    let mut selected = Vec::<SelEdge>::new();
    let mut seen = BTreeSet::new();
    for face in mesh.faces() {
        for half_edge in mesh.face_loop(face) {
            let Some(canonical) = mesh.canonical_edge(half_edge) else {
                continue;
            };
            if !seen.insert(canonical) {
                continue;
            }
            let sharpness = mesh.edge_sharpness(canonical).unwrap_or(0.0);
            if sharpness < policy.sharpness_threshold {
                continue;
            }
            let (Some(a), Some(b)) = (mesh.from_vertex(canonical), mesh.to_vertex(canonical))
            else {
                continue;
            };
            let twin = mesh.twin(canonical).ok_or(RoundError::Internal {
                detail: "canonical edge without twin",
            })?;
            let left = mesh.face(canonical).unwrap_or(FaceId::OUTSIDE);
            let right = mesh.face(twin).unwrap_or(FaceId::OUTSIDE);
            if left == FaceId::OUTSIDE || right == FaceId::OUTSIDE {
                return Err(RoundError::BoundaryEdge {
                    a: a.index().min(b.index()),
                    b: a.index().max(b.index()),
                });
            }
            if left == right {
                return Err(RoundError::UnsupportedTopology {
                    detail: "sharp edge with one face on both sides",
                });
            }
            selected.push(SelEdge { a, b, left, right });
        }
    }
    if selected.is_empty() {
        return Ok(None);
    }

    let mut planner = Planner {
        mesh,
        policy,
        half_edge_of,
        planes: BTreeMap::new(),
        points: Vec::new(),
        subst: BTreeMap::new(),
        faces: Vec::new(),
        stats: RoundStats::default(),
    };
    planner.run(&selected).map(Some)
}

impl Planner<'_> {
    fn run(&mut self, selected: &[SelEdge]) -> Result<Plan, RoundError> {
        // Sharp-graph adjacency and degrees.
        let mut adjacency = BTreeMap::<VertexId, Vec<usize>>::new();
        for (index, edge) in selected.iter().enumerate() {
            adjacency.entry(edge.a).or_default().push(index);
            adjacency.entry(edge.b).or_default().push(index);
        }
        for (&vertex, edges) in &adjacency {
            if edges.len() > 3 {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            }
        }
        let kind_of = |vertex: VertexId, adjacency: &BTreeMap<VertexId, Vec<usize>>| match adjacency
            .get(&vertex)
            .map_or(0, Vec::len)
        {
            1 => VertexKind::OpenEnd,
            2 => VertexKind::Interior,
            _ => VertexKind::Corner,
        };

        // Per-edge geometry checks. Zero-length edges (distinct seam
        // vertices at identical narrowed positions) skip them: they merge
        // into one chain position during tracing.
        let mut sweeps = Vec::with_capacity(selected.len());
        for edge in selected {
            if self.position(edge.a) == self.position(edge.b) {
                sweeps.push(None);
            } else {
                sweeps.push(Some(self.edge_sweep(edge)?));
            }
        }

        // Chains.
        let mut chains = self.trace_chains(selected, &adjacency, &sweeps)?;

        // Faces incident to any chain vertex.
        let chain_vertices: BTreeSet<VertexId> = adjacency.keys().copied().collect();
        let mut vertex_faces = BTreeMap::<VertexId, Vec<FaceId>>::new();
        for face in self.mesh.faces() {
            for half_edge in self.mesh.face_loop(face) {
                if let Some(from) = self
                    .mesh
                    .from_vertex(half_edge)
                    .filter(|from| chain_vertices.contains(from))
                {
                    vertex_faces.entry(from).or_default().push(face);
                }
            }
        }

        // Interior and open-end cross-sections.
        for chain_index in 0..chains.len() {
            self.build_sections(&mut chains, chain_index, &adjacency, &kind_of)?;
        }

        // Trihedral corners (also builds corner-end sections).
        let corners = self.build_corners(&mut chains, &adjacency, selected, &vertex_faces)?;

        // Flank substitutions (reps and coincident aliases alike).
        for chain in &chains {
            for (index, edge) in chain.edges.iter().enumerate() {
                let next = (index + 1) % chain.vertex_count();
                let section_a = &chain.sections[index];
                let section_b = &chain.sections[next];
                let subs = [
                    (edge.left, index, section_a[0]),
                    (edge.left, next, section_b[0]),
                    (edge.right, index, *section_a.last().expect("non-empty")),
                    (edge.right, next, *section_b.last().expect("non-empty")),
                ];
                for (face, position, point) in subs {
                    self.register(face, chain.verts[position], Subst::Point(point))?;
                    for alias_index in 0..chain.aliases[position].len() {
                        let alias = chain.aliases[position][alias_index];
                        self.register(face, alias, Subst::Point(point))?;
                    }
                }
            }
        }

        // Open-end splices.
        for (chain_index, chain) in chains.iter().enumerate() {
            if chain.closed {
                continue;
            }
            for at_end in [false, true] {
                let position = if at_end { chain.verts.len() - 1 } else { 0 };
                let vertex = chain.verts[position];
                if kind_of(vertex, &adjacency) != VertexKind::OpenEnd {
                    continue;
                }
                let edge = chain.end_edge(at_end);
                let mut others: Vec<FaceId> = vertex_faces
                    .get(&vertex)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .filter(|&f| f != edge.left && f != edge.right)
                    .collect();
                others.sort_unstable();
                others.dedup();
                let [end_face] = others.as_slice() else {
                    return Err(RoundError::UnsupportedEnd {
                        vertex: vertex.index(),
                    });
                };
                let end_face = *end_face;
                // The end section must lie in the end face's plane.
                let plane = self.plane(end_face)?;
                let anchor = self.position(vertex);
                for &point in &chain.sections[position] {
                    let deviation = dot(sub(self.points[point as usize], anchor), plane.normal);
                    if deviation.abs() > self.policy.max_planar_deviation {
                        return Err(RoundError::UnsupportedEnd {
                            vertex: vertex.index(),
                        });
                    }
                }
                self.register(
                    end_face,
                    vertex,
                    Subst::Splice(
                        u32::try_from(chain_index).expect("chain count fits u32"),
                        u32::try_from(position).expect("vertex index fits u32"),
                    ),
                )?;
            }
        }

        // Vertex-only faces: classify to a side and substitute.
        for (&vertex, faces) in &vertex_faces {
            if kind_of(vertex, &adjacency) == VertexKind::Corner {
                continue;
            }
            let Some((left_normal, right_normal, left_point, right_point)) =
                vertex_side_points(&chains, vertex)
            else {
                continue;
            };
            for &face in faces {
                if self.subst.contains_key(&(face, vertex)) {
                    continue;
                }
                let plane = self.plane(face)?;
                let left_dot = dot(plane.normal, left_normal);
                let right_dot = dot(plane.normal, right_normal);
                if (left_dot - right_dot).abs() <= 1e-9 {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "vertex-only face side is ambiguous",
                    });
                }
                let point = if left_dot > right_dot {
                    left_point
                } else {
                    right_point
                };
                self.register(face, vertex, Subst::Point(point))?;
            }
        }

        // Rewrite affected faces.
        let affected: Vec<FaceId> = {
            let mut faces: Vec<FaceId> = self.subst.keys().map(|&(face, _)| face).collect();
            faces.sort_unstable();
            faces.dedup();
            faces
        };
        let affected_set: BTreeSet<FaceId> = affected.iter().copied().collect();
        let selected_pairs: BTreeSet<(VertexId, VertexId)> = selected
            .iter()
            .map(|e| (e.a.min(e.b), e.a.max(e.b)))
            .collect();
        let mut attr_keys = BTreeMap::<(VertexId, VertexId), (Tok, Tok)>::new();
        for &face in &affected {
            // Affected faces must be planar: their geometry feeds offset
            // and classification decisions.
            let plane = self.plane(face)?;
            if plane.max_deviation > self.policy.max_planar_deviation {
                return Err(RoundError::NonPlanarFace { face: face.index() });
            }
            self.rewrite_face(
                face,
                &chains,
                &chain_vertices,
                &selected_pairs,
                &mut attr_keys,
            )?;
        }
        self.stats.rewritten_faces = u32::try_from(affected.len()).expect("count fits u32");

        // Strips.
        for chain in &chains {
            for index in 0..chain.edges.len() {
                let section_a = &chain.sections[index];
                let section_b = &chain.sections[(index + 1) % chain.vertex_count()];
                for band in 0..chain.segments as usize {
                    self.faces.push(NewFace {
                        entries: alloc::vec![
                            Tok::New(section_b[band]),
                            Tok::New(section_a[band]),
                            Tok::New(section_a[band + 1]),
                            Tok::New(section_b[band + 1]),
                        ],
                        region: self.policy.region,
                    });
                    self.stats.strip_faces += 1;
                }
            }
            self.stats.chains += 1;
            if chain.closed {
                self.stats.closed_chains += 1;
            }
            self.stats.max_segments = self.stats.max_segments.max(chain.segments);
        }

        // Corner patches.
        self.emit_corner_patches(&chains, &corners)?;

        // Capture edge attributes worth re-keying. The chain edges
        // themselves are consumed — their sharpness must NOT transfer onto
        // the new tangency lines (rounding exists to remove it).
        let mut edge_attrs = Vec::new();
        let mut captured = BTreeSet::new();
        for &face in &affected {
            for half_edge in self.mesh.face_loop(face) {
                let Some(canonical) = self.mesh.canonical_edge(half_edge) else {
                    continue;
                };
                if !captured.insert(canonical) {
                    continue;
                }
                let (Some(from), Some(to)) = (
                    self.mesh.from_vertex(canonical),
                    self.mesh.to_vertex(canonical),
                ) else {
                    continue;
                };
                let key = (from.min(to), from.max(to));
                if selected_pairs.contains(&key) {
                    continue;
                }
                let Some(&(new_from, new_to)) = attr_keys.get(&key) else {
                    continue;
                };
                let sharpness = self.mesh.edge_sharpness(canonical).filter(|s| *s != 0.0);
                let seam = self.mesh.edge_seam(canonical).filter(|s| *s);
                if sharpness.is_some() || seam.is_some() {
                    edge_attrs.push((new_from, new_to, sharpness, seam));
                }
            }
        }

        // Manifold pre-check over the planned complex.
        self.precheck(&affected_set, &chain_vertices)?;

        let consumed: Vec<VertexId> = chain_vertices.iter().copied().collect();
        self.stats.removed_vertices = u32::try_from(consumed.len()).expect("count fits u32");
        self.stats.added_vertices = u32::try_from(self.points.len()).expect("count fits u32");

        Ok(Plan {
            points: core::mem::take(&mut self.points),
            faces: core::mem::take(&mut self.faces),
            affected,
            consumed,
            edge_attrs,
            stats: self.stats,
        })
    }

    // --- Geometry helpers -------------------------------------------------

    fn position(&self, vertex: VertexId) -> [f64; 3] {
        self.mesh
            .vertex_position(vertex)
            .map(|p| promote(*p))
            .unwrap_or_default()
    }

    fn face_points(&self, face: FaceId) -> Vec<[f64; 3]> {
        self.mesh
            .face_loop(face)
            .filter_map(|h| self.mesh.to_vertex(h))
            .map(|v| self.position(v))
            .collect()
    }

    fn reliable_newell(points: &[[f64; 3]]) -> Option<[f64; 3]> {
        let raw = newell(points);
        let mut perimeter = 0.0;
        for i in 0..points.len() {
            perimeter += geom::norm(sub(points[(i + 1) % points.len()], points[i]));
        }
        (geom::norm(raw) > SLIVER_NEWELL_FLOOR * perimeter * perimeter)
            .then(|| normalize(raw))
            .flatten()
    }

    fn plane(&mut self, face: FaceId) -> Result<Plane, RoundError> {
        if let Some(plane) = self.planes.get(&face) {
            return Ok(*plane);
        }
        let points = self.face_points(face);
        let normal = match Self::reliable_newell(&points) {
            Some(normal) => normal,
            None => self.borrowed_normal(face, &points)?,
        };
        let inv = 1.0 / points.len() as f64;
        let mut centroid = [0.0_f64; 3];
        for p in &points {
            centroid = add(centroid, scale(*p, inv));
        }
        let mut max_deviation = 0.0_f64;
        for p in &points {
            max_deviation = max_deviation.max(dot(sub(*p, centroid), normal).abs());
        }
        let plane = Plane {
            normal,
            max_deviation,
        };
        self.planes.insert(face, plane);
        Ok(plane)
    }

    /// Recovers a sliver face's plane from a well-conditioned coplanar
    /// neighbor: breadth-first over edge neighbors (never crossing a sharp
    /// edge, so the search stays on one side of every crease and cut),
    /// accepting the first neighbor whose plane contains the sliver.
    fn borrowed_normal(&self, face: FaceId, points: &[[f64; 3]]) -> Result<[f64; 3], RoundError> {
        let mut visited = BTreeSet::new();
        visited.insert(face);
        let mut queue = alloc::collections::VecDeque::new();
        queue.push_back(face);
        let mut expansions = 0;
        while let Some(current) = queue.pop_front() {
            expansions += 1;
            if expansions > 64 {
                break;
            }
            let mut neighbors = Vec::new();
            for half_edge in self.mesh.face_loop(current) {
                if self.mesh.edge_sharpness(half_edge).unwrap_or(0.0)
                    >= self.policy.sharpness_threshold
                {
                    continue;
                }
                let Some(neighbor) = self.mesh.twin(half_edge).and_then(|t| self.mesh.face(t))
                else {
                    continue;
                };
                if neighbor == FaceId::OUTSIDE || visited.contains(&neighbor) {
                    continue;
                }
                neighbors.push(neighbor);
            }
            neighbors.sort_unstable();
            neighbors.dedup();
            for neighbor in neighbors {
                visited.insert(neighbor);
                let neighbor_points = self.face_points(neighbor);
                if let Some(normal) = Self::reliable_newell(&neighbor_points) {
                    // The sliver must lie in the neighbor's plane.
                    let inv = 1.0 / neighbor_points.len() as f64;
                    let mut centroid = [0.0_f64; 3];
                    for p in &neighbor_points {
                        centroid = add(centroid, scale(*p, inv));
                    }
                    let contained = points.iter().all(|p| {
                        dot(sub(*p, centroid), normal).abs() <= self.policy.max_planar_deviation
                    });
                    if contained {
                        return Ok(normal);
                    }
                }
                queue.push_back(neighbor);
            }
        }
        Err(RoundError::NonPlanarFace { face: face.index() })
    }

    fn edge_sweep(&mut self, edge: &SelEdge) -> Result<f64, RoundError> {
        let (small, large) = (
            edge.a.index().min(edge.b.index()),
            edge.a.index().max(edge.b.index()),
        );
        let direction = normalize(sub(self.position(edge.b), self.position(edge.a)))
            .ok_or(RoundError::DegenerateEdge { a: small, b: large })?;
        let left = self.plane(edge.left)?.normal;
        let right = self.plane(edge.right)?.normal;
        let side = dot(cross(left, right), direction);
        if side.abs() <= 1e-12 {
            return Err(RoundError::DegenerateEdge { a: small, b: large });
        }
        if side < 0.0 {
            return Err(RoundError::ConcaveEdge { a: small, b: large });
        }
        let sweep = dot(left, right).clamp(-1.0, 1.0).acos_ext();
        if !(1e-6..=core::f64::consts::PI - 1e-6).contains(&sweep) {
            return Err(RoundError::DegenerateEdge { a: small, b: large });
        }
        Ok(sweep)
    }

    fn trace_chains(
        &self,
        selected: &[SelEdge],
        adjacency: &BTreeMap<VertexId, Vec<usize>>,
        sweeps: &[Option<f64>],
    ) -> Result<Vec<Chain>, RoundError> {
        let mut visited = alloc::vec![false; selected.len()];
        let mut raw = Vec::new();

        // Each walked edge pairs with its sweep; `None` marks a zero-length
        // edge that merges away below.
        let walk = |first_vertex: VertexId, first_edge: usize, visited: &mut Vec<bool>| {
            let mut verts = alloc::vec![first_vertex];
            let mut edges = Vec::new();
            let mut current = first_vertex;
            let mut edge_index = first_edge;
            loop {
                visited[edge_index] = true;
                let edge = &selected[edge_index];
                let forward = edge.a == current;
                let (a, b, left, right) = if forward {
                    (edge.a, edge.b, edge.left, edge.right)
                } else {
                    (edge.b, edge.a, edge.right, edge.left)
                };
                edges.push((
                    DirEdge {
                        a,
                        b,
                        left,
                        right,
                        sweep: sweeps[edge_index].unwrap_or(0.0),
                    },
                    sweeps[edge_index].is_none(),
                ));
                current = b;
                verts.push(b);
                let incident = &adjacency[&current];
                if incident.len() != 2 {
                    break;
                }
                let next = incident.iter().copied().find(|&c| c != edge_index);
                match next {
                    Some(candidate) if !visited[candidate] => edge_index = candidate,
                    _ => break,
                }
            }
            (verts, edges)
        };

        // Open chains: spawn from junction/end vertices in ascending order.
        for (&vertex, incident) in adjacency {
            if incident.len() == 2 {
                continue;
            }
            for &edge_index in incident {
                if visited[edge_index] {
                    continue;
                }
                let (verts, edges) = walk(vertex, edge_index, &mut visited);
                raw.push((verts, edges, false));
            }
        }
        // Rings: everything left is a closed loop of interior vertices.
        for edge_index in 0..selected.len() {
            if visited[edge_index] {
                continue;
            }
            let edge = &selected[edge_index];
            let start = edge.a.min(edge.b);
            let (mut verts, edges) = walk(start, edge_index, &mut visited);
            if verts.first() != verts.last() {
                return Err(RoundError::UnsupportedTopology {
                    detail: "ring chain did not close",
                });
            }
            verts.pop();
            raw.push((verts, edges, true));
        }

        let mut chains = Vec::with_capacity(raw.len());
        for (verts, edges, closed) in raw {
            let (verts, aliases, edges) = merge_coincident(verts, edges, closed)?;
            let segments = self.chain_segments(&edges)?;
            let count = verts.len();
            chains.push(Chain {
                verts,
                aliases,
                edges,
                closed,
                segments,
                sections: alloc::vec![Vec::new(); count],
                frames: alloc::vec![None; count],
            });
        }
        Ok(chains)
    }

    fn chain_segments(&self, edges: &[DirEdge]) -> Result<u32, RoundError> {
        match self.policy.kind {
            RoundKind::Chamfer { .. } => Ok(1),
            RoundKind::Fillet { radius } => {
                if let Some(explicit) = self.policy.segments {
                    return Ok(explicit.clamp(1, 256));
                }
                let ratio = (1.0 - self.policy.chord_tolerance / radius).clamp(-1.0, 1.0);
                let theta = (2.0 * ratio.acos_ext()).max(1e-3);
                let max_sweep = edges.iter().fold(0.0_f64, |acc, e| acc.max(e.sweep));
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "ceiling of a small positive ratio"
                )]
                let bands = (max_sweep / theta).ceil_ext() as u32;
                Ok(bands.clamp(1, 256))
            }
        }
    }

    fn push_point(&mut self, point: [f64; 3]) -> u32 {
        let id = u32::try_from(self.points.len()).expect("point count fits u32");
        self.points.push(point);
        id
    }

    /// Builds interior and open-end cross-sections for one chain.
    fn build_sections(
        &mut self,
        chains: &mut [Chain],
        chain_index: usize,
        adjacency: &BTreeMap<VertexId, Vec<usize>>,
        kind_of: &impl Fn(VertexId, &BTreeMap<VertexId, Vec<usize>>) -> VertexKind,
    ) -> Result<(), RoundError> {
        let chain = chains[chain_index].clone();
        let mut sections = alloc::vec![Vec::new(); chain.vertex_count()];
        let mut frames = alloc::vec![None; chain.vertex_count()];
        for index in 0..chain.vertex_count() {
            let vertex = chain.verts[index];
            if kind_of(vertex, adjacency) == VertexKind::Corner {
                continue; // Corner sections are built with the corner.
            }
            let degenerate = RoundError::DegenerateEdge {
                a: vertex.index(),
                b: vertex.index(),
            };
            let (prev, next) = chain.edges_around(index);
            let mut tangent = [0.0_f64; 3];
            let mut left_normal = [0.0_f64; 3];
            let mut right_normal = [0.0_f64; 3];
            let mut directions = Vec::new();
            for edge_index in [prev, next].into_iter().flatten() {
                let edge = chain.edges[edge_index];
                let direction = normalize(sub(self.position(edge.b), self.position(edge.a)))
                    .ok_or(degenerate)?;
                directions.push(direction);
                tangent = add(tangent, direction);
                left_normal = add(left_normal, self.plane(edge.left)?.normal);
                right_normal = add(right_normal, self.plane(edge.right)?.normal);
            }
            if directions.len() == 2
                && dot(directions[0], directions[1]) < self.policy.max_tangent_turn.cos_ext()
            {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            }
            let tangent = normalize(tangent).ok_or(degenerate)?;
            let left_normal = normalize(left_normal).ok_or(degenerate)?;
            let right_normal = normalize(right_normal).ok_or(degenerate)?;
            let cos_sweep = dot(left_normal, right_normal).clamp(-1.0, 1.0);
            let sweep = cos_sweep.acos_ext();
            if !(1e-6..=core::f64::consts::PI - 1e-6).contains(&sweep) {
                return Err(degenerate);
            }
            let offset = match self.policy.kind {
                // t = r * tan(sweep / 2), via the half-angle identity.
                RoundKind::Fillet { radius } => {
                    radius * (1.0 - cos_sweep * cos_sweep).max(0.0).sqrt_ext() / (1.0 + cos_sweep)
                }
                RoundKind::Chamfer { setback } => setback,
            };
            let left_dir = normalize(cross(left_normal, tangent)).ok_or(degenerate)?;
            let right_dir = normalize(cross(tangent, right_normal)).ok_or(degenerate)?;
            let anchor = self.position(vertex);
            let left_point = add(anchor, scale(left_dir, offset));
            let right_point = add(anchor, scale(right_dir, offset));
            let section_points = match self.policy.kind {
                RoundKind::Chamfer { .. } => alloc::vec![left_point, right_point],
                RoundKind::Fillet { radius } => {
                    let center = scale(
                        add(
                            sub(left_point, scale(left_normal, radius)),
                            sub(right_point, scale(right_normal, radius)),
                        ),
                        0.5,
                    );
                    arc_points(center, left_point, right_point, chain.segments).ok_or(degenerate)?
                }
            };
            sections[index] = section_points
                .into_iter()
                .map(|p| self.push_point(p))
                .collect();
            frames[index] = Some((left_normal, right_normal));
        }
        chains[chain_index].sections = sections;
        chains[chain_index].frames = frames;
        Ok(())
    }

    /// Builds corner data and corner-end sections; returns per corner the
    /// fillet center and the adjoining chain ends for the ring walk.
    fn build_corners(
        &mut self,
        chains: &mut [Chain],
        adjacency: &BTreeMap<VertexId, Vec<usize>>,
        selected: &[SelEdge],
        vertex_faces: &BTreeMap<VertexId, Vec<FaceId>>,
    ) -> Result<Vec<CornerPlan>, RoundError> {
        let mut corners = Vec::new();
        for (&vertex, incident) in adjacency {
            if incident.len() != 3 {
                continue;
            }
            // The three flank faces must be exactly the vertex's faces.
            let mut faces: Vec<FaceId> = incident
                .iter()
                .flat_map(|&e| [selected[e].left, selected[e].right])
                .collect();
            faces.sort_unstable();
            faces.dedup();
            let mut incident_faces = vertex_faces.get(&vertex).cloned().unwrap_or_default();
            incident_faces.sort_unstable();
            incident_faces.dedup();
            if faces.len() != 3 || incident_faces != faces {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            }

            let anchor = self.position(vertex);
            let normals = [
                self.plane(faces[0])?.normal,
                self.plane(faces[1])?.normal,
                self.plane(faces[2])?.normal,
            ];

            // One tangency point per corner face.
            let mut q = BTreeMap::<FaceId, u32>::new();
            let center = match self.policy.kind {
                RoundKind::Fillet { radius } => {
                    let rhs = [
                        dot(normals[0], anchor) - radius,
                        dot(normals[1], anchor) - radius,
                        dot(normals[2], anchor) - radius,
                    ];
                    let center = solve3([normals[0], normals[1], normals[2]], rhs).ok_or(
                        RoundError::UnsupportedJunction {
                            vertex: vertex.index(),
                        },
                    )?;
                    for (face, normal) in faces.iter().zip(normals) {
                        let point = add(center, scale(normal, radius));
                        let id = self.push_point(point);
                        q.insert(*face, id);
                    }
                    Some(center)
                }
                RoundKind::Chamfer { setback } => {
                    for (face, normal) in faces.iter().zip(normals) {
                        let point = self.chamfer_corner_point(
                            vertex, *face, normal, setback, selected, incident,
                        )?;
                        let id = self.push_point(point);
                        q.insert(*face, id);
                    }
                    None
                }
            };

            // Corner-end sections: locate each chain end at this vertex.
            let mut ends = Vec::new();
            for (chain_index, chain) in chains.iter_mut().enumerate() {
                if chain.closed {
                    continue;
                }
                for at_end in [false, true] {
                    let position = if at_end { chain.verts.len() - 1 } else { 0 };
                    if chain.verts[position] != vertex {
                        continue;
                    }
                    let edge = chain.end_edge(at_end);
                    let left_q = q[&edge.left];
                    let right_q = q[&edge.right];
                    let section = if let Some(center) = center {
                        let from = self.points[left_q as usize];
                        let to = self.points[right_q as usize];
                        let arc = arc_points(center, from, to, chain.segments).ok_or(
                            RoundError::UnsupportedJunction {
                                vertex: vertex.index(),
                            },
                        )?;
                        let mut ids = Vec::with_capacity(arc.len());
                        ids.push(left_q);
                        for point in &arc[1..arc.len() - 1] {
                            ids.push(self.push_point(*point));
                        }
                        ids.push(right_q);
                        ids
                    } else {
                        alloc::vec![left_q, right_q]
                    };
                    chain.sections[position] = section;
                    ends.push((chain_index, at_end));
                }
            }
            if ends.len() != 3 {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            }

            // The corner vertex maps to the face's tangency point on each
            // corner face.
            for face in faces {
                self.register(face, vertex, Subst::Point(q[&face]))?;
            }
            corners.push((vertex, center, ends));
            self.stats.corners += 1;
        }
        Ok(corners)
    }

    /// Chamfer corner tangency point on one face: the in-plane intersection
    /// of the two offset lines of the face's chamfered edges.
    fn chamfer_corner_point(
        &mut self,
        vertex: VertexId,
        face: FaceId,
        normal: [f64; 3],
        setback: f64,
        selected: &[SelEdge],
        incident: &[usize],
    ) -> Result<[f64; 3], RoundError> {
        let junction = RoundError::UnsupportedJunction {
            vertex: vertex.index(),
        };
        let mut lines = Vec::new();
        for &edge_index in incident {
            let edge = &selected[edge_index];
            if edge.left != face && edge.right != face {
                continue;
            }
            let direction =
                normalize(sub(self.position(edge.b), self.position(edge.a))).ok_or(junction)?;
            let inward = if edge.left == face {
                cross(normal, direction)
            } else {
                cross(direction, normal)
            };
            let inward = normalize(inward).ok_or(junction)?;
            let anchor = self.position(vertex);
            lines.push((add(anchor, scale(inward, setback)), direction));
        }
        if lines.len() != 2 {
            return Err(junction);
        }
        let (point, gap) =
            line_intersection(lines[0].0, lines[0].1, lines[1].0, lines[1].1).ok_or(junction)?;
        if gap > self.policy.max_planar_deviation {
            return Err(junction);
        }
        Ok(point)
    }

    fn register(&mut self, face: FaceId, vertex: VertexId, subst: Subst) -> Result<(), RoundError> {
        match self.subst.insert((face, vertex), subst) {
            None => Ok(()),
            Some(previous) if previous == subst => Ok(()),
            Some(_) => Err(RoundError::UnsupportedTopology {
                detail: "conflicting substitutions for one face vertex",
            }),
        }
    }

    /// Rewrites one affected face's loop through its substitutions.
    fn rewrite_face(
        &mut self,
        face: FaceId,
        chains: &[Chain],
        chain_vertices: &BTreeSet<VertexId>,
        selected_pairs: &BTreeSet<(VertexId, VertexId)>,
        attr_keys: &mut BTreeMap<(VertexId, VertexId), (Tok, Tok)>,
    ) -> Result<(), RoundError> {
        let old_loop: Vec<VertexId> = self
            .mesh
            .face_loop(face)
            .filter_map(|h| self.mesh.to_vertex(h))
            .collect();
        // Image of each original vertex: one token or a spliced run.
        let mut images: Vec<Vec<Tok>> = Vec::with_capacity(old_loop.len());
        for (position, &vertex) in old_loop.iter().enumerate() {
            match self.subst.get(&(face, vertex)) {
                None => {
                    if chain_vertices.contains(&vertex) {
                        return Err(RoundError::UnsupportedTopology {
                            detail: "chain vertex without a substitution",
                        });
                    }
                    images.push(alloc::vec![Tok::Old(vertex)]);
                }
                Some(Subst::Point(point)) => images.push(alloc::vec![Tok::New(*point)]),
                Some(Subst::Splice(chain_index, vertex_index)) => {
                    let chain = &chains[*chain_index as usize];
                    let section = &chain.sections[*vertex_index as usize];
                    let end_edge = chain.end_edge(*vertex_index != 0);
                    // Orientation from the incoming original edge's twin.
                    let previous = old_loop[(position + old_loop.len() - 1) % old_loop.len()];
                    let twin_face = self
                        .half_edge_of
                        .get(&(vertex, previous))
                        .copied()
                        .and_then(|h| self.mesh.face(h));
                    let run: Vec<Tok> = if twin_face == Some(end_edge.left) {
                        section.iter().map(|&p| Tok::New(p)).collect()
                    } else if twin_face == Some(end_edge.right) {
                        section.iter().rev().map(|&p| Tok::New(p)).collect()
                    } else {
                        return Err(RoundError::UnsupportedEnd {
                            vertex: vertex.index(),
                        });
                    };
                    images.push(run);
                }
            }
        }

        // Coincident seam vertices map to one point: zero-length old edges
        // collapse, and a face made entirely of them vanishes.
        let mut entries: Vec<Tok> = images.iter().flatten().copied().collect();
        entries.dedup();
        while entries.len() > 1 && entries.first() == entries.last() {
            entries.pop();
        }
        if entries.len() < 3 {
            self.stats.vanished_faces += 1;
            return Ok(());
        }
        let mut sorted = entries.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != entries.len() {
            return Err(RoundError::ClearanceExceeded { face: face.index() });
        }

        // Orientation must survive: the rewritten loop's Newell normal must
        // agree with the original.
        let old_points: Vec<[f64; 3]> = old_loop.iter().map(|&v| self.position(v)).collect();
        let new_points: Vec<[f64; 3]> = entries
            .iter()
            .map(|tok| match tok {
                Tok::Old(v) => self.position(*v),
                Tok::New(p) => self.points[*p as usize],
            })
            .collect();
        let old_normal = newell(&old_points);
        let new_normal = newell(&new_points);
        if let (Some(old_unit), Some(new_unit)) = (normalize(old_normal), normalize(new_normal))
            && dot(old_unit, new_unit) < -0.5
        {
            return Err(RoundError::ClearanceExceeded { face: face.index() });
        }

        // Old-edge -> new-edge mapping for attribute re-keying.
        for position in 0..old_loop.len() {
            let next = (position + 1) % old_loop.len();
            let from = old_loop[position];
            let to = old_loop[next];
            let mapped = (
                *images[position].last().expect("non-empty image"),
                images[next][0],
            );
            let (key, stored) = if from <= to {
                ((from, to), mapped)
            } else {
                ((to, from), (mapped.1, mapped.0))
            };
            // Chain edges split into two tangency lines: each flank maps
            // them differently, and their attributes never transfer.
            if selected_pairs.contains(&key) {
                continue;
            }
            match attr_keys.get(&key) {
                None => {
                    attr_keys.insert(key, stored);
                }
                Some(&existing) if existing == stored => {}
                Some(_) => {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "edge attribute re-key disagreement",
                    });
                }
            }
        }

        let region = self
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .and_then(|layer| layer.get(face.as_id()).copied());
        self.faces.push(NewFace { entries, region });
        Ok(())
    }

    /// Emits corner patches by walking each corner's boundary ring.
    fn emit_corner_patches(
        &mut self,
        chains: &[Chain],
        corners: &[CornerPlan],
    ) -> Result<(), RoundError> {
        for (vertex, center, ends) in corners {
            // Directed ring edges are the twins of the adjoining strip
            // edges: strips traverse an end section descending and a start
            // section ascending, so the ring runs the other way.
            let mut successor = BTreeMap::<u32, u32>::new();
            for &(chain_index, at_end) in ends {
                let chain = &chains[chain_index];
                let position = if at_end { chain.vertex_count() - 1 } else { 0 };
                let section = &chain.sections[position];
                for pair in section.windows(2) {
                    let (from, to) = if at_end {
                        (pair[0], pair[1])
                    } else {
                        (pair[1], pair[0])
                    };
                    if successor.insert(from, to).is_some() {
                        return Err(RoundError::UnsupportedTopology {
                            detail: "corner ring is not a simple cycle",
                        });
                    }
                }
            }
            let Some((&start, _)) = successor.iter().next() else {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            };
            let mut ring = alloc::vec![start];
            let mut current = start;
            loop {
                let Some(&next) = successor.get(&current) else {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "corner ring does not close",
                    });
                };
                if next == start {
                    break;
                }
                ring.push(next);
                current = next;
                if ring.len() > successor.len() {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "corner ring does not close",
                    });
                }
            }
            if ring.len() != successor.len() {
                return Err(RoundError::UnsupportedTopology {
                    detail: "corner ring leaves stray edges",
                });
            }

            if ring.len() == 3 {
                self.faces.push(NewFace {
                    entries: ring.iter().map(|&p| Tok::New(p)).collect(),
                    region: self.policy.region,
                });
                self.stats.patch_faces += 1;
                continue;
            }

            // Fan from the ring centroid; fillet corners push it onto the
            // corner sphere for a rounder patch.
            let inv = 1.0 / ring.len() as f64;
            let mut centroid = [0.0_f64; 3];
            for &point in &ring {
                centroid = add(centroid, scale(self.points[point as usize], inv));
            }
            if let (RoundKind::Fillet { radius }, Some(center)) = (self.policy.kind, center)
                && let Some(direction) = normalize(sub(centroid, *center))
            {
                centroid = add(*center, scale(direction, radius));
            }
            let apex = self.push_point(centroid);
            for index in 0..ring.len() {
                let from = ring[index];
                let to = ring[(index + 1) % ring.len()];
                self.faces.push(NewFace {
                    entries: alloc::vec![Tok::New(apex), Tok::New(from), Tok::New(to)],
                    region: self.policy.region,
                });
                self.stats.patch_faces += 1;
            }
        }
        Ok(())
    }

    /// Validates the planned complex: every directed edge unique, every
    /// undirected edge either paired internally or matched by a surviving
    /// twin outside the affected set.
    fn precheck(
        &self,
        affected: &BTreeSet<FaceId>,
        chain_vertices: &BTreeSet<VertexId>,
    ) -> Result<(), RoundError> {
        let mut directed = BTreeSet::new();
        let mut undirected = BTreeMap::<(Tok, Tok), u8>::new();
        for face in &self.faces {
            if face.entries.len() < 3 {
                return Err(RoundError::UnsupportedTopology {
                    detail: "planned face with fewer than three vertices",
                });
            }
            for index in 0..face.entries.len() {
                let from = face.entries[index];
                let to = face.entries[(index + 1) % face.entries.len()];
                if let Tok::Old(v) = from
                    && chain_vertices.contains(&v)
                {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "consumed vertex survives in a planned face",
                    });
                }
                if from == to || !directed.insert((from, to)) {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "duplicate directed edge in the planned complex",
                    });
                }
                let key = if from <= to { (from, to) } else { (to, from) };
                *undirected.entry(key).or_default() += 1;
            }
        }
        for (&(from, to), &count) in &undirected {
            match count {
                2 => {}
                1 => {
                    let (Tok::Old(a), Tok::Old(b)) = (from, to) else {
                        return Err(RoundError::UnsupportedTopology {
                            detail: "planned boundary edge with a new vertex",
                        });
                    };
                    let survives = [(a, b), (b, a)].into_iter().any(|(x, y)| {
                        self.half_edge_of
                            .get(&(x, y))
                            .and_then(|&h| self.mesh.face(h))
                            .is_some_and(|f| f != FaceId::OUTSIDE && !affected.contains(&f))
                    });
                    if !survives {
                        return Err(RoundError::UnsupportedTopology {
                            detail: "planned boundary edge without a surviving twin",
                        });
                    }
                }
                _ => {
                    return Err(RoundError::UnsupportedTopology {
                        detail: "planned non-manifold edge",
                    });
                }
            }
        }
        Ok(())
    }
}

/// Merges runs of coincident chain vertices connected by zero-length
/// edges into single positions with alias lists.
#[expect(clippy::type_complexity, reason = "internal chain assembly")]
fn merge_coincident(
    verts: Vec<VertexId>,
    edges: Vec<(DirEdge, bool)>,
    closed: bool,
) -> Result<(Vec<VertexId>, Vec<Vec<VertexId>>, Vec<DirEdge>), RoundError> {
    if !edges.iter().any(|(_, zero)| !zero) {
        return Err(RoundError::UnsupportedTopology {
            detail: "sharp chain degenerates to a point",
        });
    }
    let (verts, edges) = if closed {
        // Rotate the ring so it neither starts nor ends on a zero-length
        // edge boundary: begin right after a surviving edge.
        let count = edges.len();
        let offset = (0..count)
            .find(|&j| !edges[(j + count - 1) % count].1)
            .expect("a surviving edge exists");
        let verts: Vec<VertexId> = (0..count).map(|i| verts[(offset + i) % count]).collect();
        let edges: Vec<(DirEdge, bool)> = (0..count).map(|i| edges[(offset + i) % count]).collect();
        (verts, edges)
    } else {
        if edges.first().is_some_and(|(_, zero)| *zero)
            || edges.last().is_some_and(|(_, zero)| *zero)
        {
            return Err(RoundError::UnsupportedTopology {
                detail: "zero-length sharp edge at a chain end",
            });
        }
        (verts, edges)
    };

    let mut merged_verts = alloc::vec![verts[0]];
    let mut aliases: Vec<Vec<VertexId>> = alloc::vec![Vec::new()];
    let mut merged_edges = Vec::new();
    for (index, (edge, zero)) in edges.iter().enumerate() {
        let target = verts[(index + 1) % verts.len()];
        if *zero {
            aliases.last_mut().expect("non-empty").push(target);
        } else {
            merged_edges.push(*edge);
            if closed && index == edges.len() - 1 {
                // The ring's final edge returns to position zero.
                continue;
            }
            merged_verts.push(target);
            aliases.push(Vec::new());
        }
    }
    if closed && merged_verts.len() != merged_edges.len() {
        return Err(RoundError::UnsupportedTopology {
            detail: "ring chain merge lost its closure",
        });
    }
    Ok((merged_verts, aliases, merged_edges))
}

/// Averaged side normals and section endpoints for a non-corner chain
/// vertex, used to classify vertex-only faces.
fn vertex_side_points(
    chains: &[Chain],
    vertex: VertexId,
) -> Option<([f64; 3], [f64; 3], u32, u32)> {
    for chain in chains {
        for (index, &v) in chain.verts.iter().enumerate() {
            if v != vertex && !chain.aliases[index].contains(&vertex) {
                continue;
            }
            let (left_normal, right_normal) = chain.frames[index]?;
            let section = &chain.sections[index];
            return Some((
                left_normal,
                right_normal,
                section[0],
                *section.last().expect("non-empty section"),
            ));
        }
    }
    None
}

fn apply(mesh: &mut Mesh, plan: Plan) -> Result<RoundStats, RoundError> {
    let mut staged = mesh.clone();
    let stats = apply_staged(&mut staged, plan)?;
    *mesh = staged;
    Ok(stats)
}

fn apply_staged(mesh: &mut Mesh, plan: Plan) -> Result<RoundStats, RoundError> {
    let mut session = mesh.edit();
    if delete_faces(&mut session, &plan.affected, DeletePolicy::KeepIsolated).is_err() {
        return Err(RoundError::Internal {
            detail: "affected faces could not be deleted",
        });
    }
    let vertex_ids: Vec<VertexId> = plan
        .points
        .iter()
        .map(|&p| add_vertex(&mut session, narrow(p)))
        .collect();
    let resolve = |tok: Tok| match tok {
        Tok::Old(v) => v,
        Tok::New(p) => vertex_ids[p as usize],
    };
    let mut added = Vec::with_capacity(plan.faces.len());
    for planned in &plan.faces {
        let loop_vertices: Vec<VertexId> = planned.entries.iter().map(|&t| resolve(t)).collect();
        if !crate::session::face_preserves_boundary_continuation(session.mesh(), &loop_vertices) {
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
            return Err(RoundError::UnsupportedTopology {
                detail: "face rewrite would pinch an OUTSIDE boundary vertex",
            });
        }
        let Ok(face) = add_face(&mut session, &loop_vertices) else {
            #[expect(unused_must_use, reason = "discard sink output")]
            {
                session.finish();
            }
            return Err(RoundError::Internal {
                detail: "planned face was rejected by add_face",
            });
        };
        if let Some(region) = planned.region {
            let _ = set_face_region(&mut session, face, region);
        }
        added.push(face);
    }

    // Re-key surviving edge attributes onto the rewritten edges.
    let mut new_half_edges = BTreeMap::new();
    for &face in &added {
        let loop_edges: Vec<HalfEdgeId> = session.mesh().face_loop(face).collect();
        for half_edge in loop_edges {
            let (Some(from), Some(to)) = (
                session.mesh().from_vertex(half_edge),
                session.mesh().to_vertex(half_edge),
            ) else {
                continue;
            };
            new_half_edges.insert((from, to), half_edge);
        }
    }
    for &(from, to, sharpness, seam) in &plan.edge_attrs {
        let from = resolve(from);
        let to = resolve(to);
        let half_edge = new_half_edges
            .get(&(from, to))
            .or_else(|| new_half_edges.get(&(to, from)))
            .copied();
        let Some(half_edge) = half_edge else {
            continue;
        };
        if let Some(sharpness) = sharpness {
            let _ = set_edge_sharpness(&mut session, half_edge, sharpness);
        }
        if let Some(seam) = seam {
            let _ = set_edge_seam(&mut session, half_edge, seam);
        }
    }

    if delete_vertices(&mut session, &plan.consumed).is_err() {
        #[expect(unused_must_use, reason = "discard sink output")]
        {
            session.finish();
        }
        return Err(RoundError::Internal {
            detail: "consumed chain vertices could not be removed",
        });
    }
    #[expect(unused_must_use, reason = "discard sink output")]
    {
        session.finish();
    }
    Ok(plan.stats)
}
