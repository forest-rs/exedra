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
//! rim); mitered turns sharing a planar flank and equal dihedral angles;
//! gently bent chains (per-vertex averaged frames elsewhere); per-edge
//! varying flank faces (faceted walls). Everything outside that envelope is a typed
//! [`RoundError`] and the mesh is left byte-identical: concave edges,
//! junction valence other than one, two, or three, non-trihedral corners,
//! chain turns beyond [`RoundPolicy::max_tangent_turn`], non-planar
//! affected faces, and rewrites that would invert or degenerate a face.
//!
//! Planar-flank miters preserve the requested setback or cylinder radius on
//! both incident edges, including polygonal drill rims. Fillet miters retain
//! a crease between cylinders; they are not spherical corner blends. Square
//! rims require opting into a 90-degree [`RoundPolicy::max_tangent_turn`].
//!
//! Two quality caveats are deliberate v1 scope: averaged per-vertex frames
//! outside the planar-flank case can leave flank faces slightly non-planar
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
mod uv;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

use crate::math::FloatExt;
use crate::op::{
    AddFaceError, add_face, add_vertex, delete_faces, delete_vertices, set_corner_normal_override,
    set_corner_uv, set_edge_seam, set_edge_sharpness, set_face_region,
};
use crate::{DeletePolicy, FaceId, HalfEdgeId, Mesh, VertexId, attr};

use exedra_math::{add, cross, dot, narrow, norm, normalize, promote, scale, sub};
use geom::{
    Plane, arc_points, corner_quad, line_intersection, newell, solve3, spherical_triangle_error,
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
    /// Explicit fillet band count and corner radial-layer count; `None`
    /// derives them from [`RoundPolicy::chord_tolerance`]. Explicit counts
    /// must be in `1..=256`; derived counts exceeding 256 are refused.
    /// Chamfers always use one band.
    pub segments: Option<u32>,
    /// Maximum chord deviation for fillet bands, elliptical miter seams, and
    /// spherical corner patches when `segments` is `None`, before the final
    /// f32 coordinate rounding.
    pub chord_tolerance: f64,
    /// Edges with [`attr::EDGE_SHARPNESS`] at or above this value round.
    pub sharpness_threshold: f32,
    /// [`attr::FACE_REGION`] assigned to new strip and patch faces, or the
    /// first source face's region when absent.
    pub region: Option<u32>,
    /// Maximum absolute deviation for affected-face planarity and end-face
    /// containment checks.
    pub max_planar_deviation: f64,
    /// Maximum turn angle (radians) between consecutive chain edges.
    /// Defaults to 0.7; use [`core::f64::consts::FRAC_PI_2`] for square rims.
    /// Angle comparisons allow 1e-6 radians for stored-coordinate rounding.
    pub max_tangent_turn: f64,
}

impl RoundPolicy {
    /// Checks scalar parameters before an operation is planned.
    ///
    /// # Errors
    /// Returns [`RoundError::InvalidPolicy`] for non-finite or out-of-range
    /// values. Geometric clearance and required band counts are checked later.
    pub fn validate(&self) -> Result<(), RoundError> {
        validate_policy(self)
    }
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
    /// An explicitly selected half-edge is not live in the input mesh.
    InvalidEdge {
        /// The rejected identifier.
        edge: HalfEdgeId,
    },
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
    /// The offset would collapse a face, reverse a rewritten face, or make
    /// the ends of a rounded edge cross.
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
            Self::InvalidEdge { edge } => write!(f, "selected edge {edge:?} is not live"),
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
    /// Consumed chain vertices removed.
    pub removed_vertices: u32,
    /// New vertices added.
    pub added_vertices: u32,
    /// Largest band count used by any chain.
    pub max_segments: u32,
}

/// Input faces responsible for one replacement or generated face.
///
/// Identifiers refer to the mesh before the operation. Retain any source
/// attributes needed for remapping before calling [`round_edges`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoundFaceSource {
    /// A trimmed or extended input face.
    Face(FaceId),
    /// A band along the boundary of these two input faces, in ascending ID order.
    Edge([FaceId; 2]),
    /// A patch at a trihedral corner, with input faces in ascending ID order.
    Corner([FaceId; 3]),
}

impl RoundFaceSource {
    /// Input faces in deterministic ownership order.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        match self {
            Self::Face(face) => core::slice::from_ref(face),
            Self::Edge(faces) => faces,
            Self::Corner(faces) => faces,
        }
    }

    /// Whether this face is a new band or corner patch.
    #[must_use]
    pub fn is_generated(self) -> bool {
        !matches!(self, Self::Face(_))
    }
}

/// Work and source-face mapping from an explicit rounding pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RoundResult {
    /// Work performed by the pass.
    pub stats: RoundStats,
    /// New and replacement face IDs paired with their input ownership.
    /// Unchanged faces retain their IDs and are absent from this list.
    pub face_provenance: Vec<(FaceId, RoundFaceSource)>,
}

/// Rounds every edge whose sharpness meets the policy threshold.
///
/// Returns work counters; a pass that selects no edges is a successful
/// no-op with zeroed counters. Attribute and normal behavior matches
/// [`round_edges`].
///
/// # Errors
///
/// Any configuration outside the documented v1 envelope fails typed with
/// the mesh left byte-identical; see [`RoundError`].
pub fn round_sharp_edges(mesh: &mut Mesh, policy: &RoundPolicy) -> Result<RoundStats, RoundError> {
    validate_policy(policy)?;
    let Some(plan) = plan(mesh, policy, None)? else {
        return Ok(RoundStats::default());
    };
    apply(mesh, plan).map(|result| result.stats)
}

/// Rounds the explicit input edges, independently of their sharpness values.
///
/// Twins and duplicate IDs denote the same target. Targets are processed in
/// canonical mesh order, so input order does not affect the result. Empty
/// selection is a no-op. Untargeted sharpness and seam attributes survive.
///
/// Fillets author radial corner normals, with separate cylindrical normals
/// across miter creases; chamfers and trimmed planar faces
/// retain flat boundaries. Use [`crate::NormalsSource::CustomOrDerived`] for
/// those normals. Valid normal overrides at unchanged corners of rewritten
/// faces survive. Unchanged faces retain all their attributes.
/// New bands and patches use `policy.region`, or their first source face's
/// region when it is absent.
///
/// # UVs
///
/// Surviving corners keep their exact UVs. New corners interpolate the input
/// face's robust triangulation; bands and patches project onto the first source
/// face's chart, matching material ownership. Projection uses the source chart's
/// coordinates: textures stretch toward perpendicular tangencies and can fold
/// on surfaces turning beyond them. It is not an arc-length unwrap.
/// Outside the source polygon, the closest triangle's mapping is extended.
/// Different source charts can meet at seams,
/// which are marked on changed edges without clearing authored seams.
///
/// A source face with missing or non-finite UVs, or a failed triangulation,
/// supplies no interpolated UVs. Surviving corner values still remain intact.
/// Non-finite interpolation results are left unset. No default mapping or
/// texture scale is invented for untextured inputs; callers can remap faces
/// using [`RoundResult::face_provenance`].
///
/// # Errors
///
/// A stale edge or any configuration outside the documented rounding envelope
/// fails before commit. The input mesh remains byte-identical on every error.
pub fn round_edges(
    mesh: &mut Mesh,
    edges: &[HalfEdgeId],
    policy: &RoundPolicy,
) -> Result<RoundResult, RoundError> {
    validate_policy(policy)?;
    let mut selected = BTreeSet::new();
    for &edge in edges {
        selected.insert(
            mesh.canonical_edge(edge)
                .ok_or(RoundError::InvalidEdge { edge })?,
        );
    }
    let Some(plan) = plan(mesh, policy, Some(&selected))? else {
        return Ok(RoundResult::default());
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
    if policy.segments.is_some_and(|n| !(1..=256).contains(&n)) {
        return Err(RoundError::InvalidPolicy {
            detail: "segments must be in 1..=256",
        });
    }
    if !policy.sharpness_threshold.is_finite() {
        return Err(RoundError::InvalidPolicy {
            detail: "sharpness_threshold must be finite",
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
    edges: Vec<DirEdge>,
    closed: bool,
    segments: u32,
    /// Cross-section point ids per chain vertex, ordered left to right.
    sections: Vec<Vec<u32>>,
    /// Averaged (left, right) flank normals per chain vertex, where the
    /// vertex owns a per-vertex frame (interior and open-end vertices).
    frames: Vec<Option<([f64; 3], [f64; 3])>>,
    /// Shared miter sections have a different radial normal on each incident
    /// strip, rather than one normal attached to the section point.
    miters: Vec<bool>,
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

/// Linear map from profile plane offsets to a shared miter section.
struct Miter {
    common: [f64; 3],
    sides: [f64; 3],
    sweep: f64,
    reversed: bool,
}

impl Miter {
    /// A circular profile becomes an ellipse at a miter. Its largest stretch
    /// bounds 3D chord error, including the seam curve between the cylinders.
    fn radius_scale(&self) -> f64 {
        let a = add(self.common, scale(self.sides, self.sweep.cos_ext()));
        let b = scale(self.sides, self.sweep.sin_ext());
        let aa = dot(a, a);
        let bb = dot(b, b);
        let ab = dot(a, b);
        // Largest eigenvalue of the two-dimensional Gram matrix.
        ((aa + bb + ((aa - bb) * (aa - bb) + 4.0 * ab * ab).sqrt_ext()) * 0.5).sqrt_ext()
    }

    fn points(&self, anchor: [f64; 3], kind: RoundKind, segments: u32) -> Vec<[f64; 3]> {
        let mut points = Vec::with_capacity(segments as usize + 1);
        for band in 0..=segments {
            let (common_height, side_height) = match kind {
                RoundKind::Chamfer { setback } => {
                    let height = -setback * self.sweep.sin_ext();
                    if band == 0 {
                        (0.0, height)
                    } else {
                        (height, 0.0)
                    }
                }
                RoundKind::Fillet { radius } => {
                    let theta = self.sweep * f64::from(band) / f64::from(segments);
                    (
                        radius * (theta.cos_ext() - 1.0),
                        radius * ((self.sweep - theta).cos_ext() - 1.0),
                    )
                }
            };
            points.push(add(
                anchor,
                add(
                    scale(self.common, common_height),
                    scale(self.sides, side_height),
                ),
            ));
        }
        if self.reversed {
            points.reverse();
        }
        points
    }
}

#[derive(Clone, Debug)]
struct NewFace {
    entries: Vec<Tok>,
    region: Option<u32>,
    source: RoundFaceSource,
    normals: Vec<[f32; 3]>,
    uvs: Vec<Option<[f32; 2]>>,
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
    point_normals: Vec<Option<[f64; 3]>>,
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

fn plan(
    mesh: &Mesh,
    policy: &RoundPolicy,
    explicit: Option<&BTreeSet<HalfEdgeId>>,
) -> Result<Option<Plan>, RoundError> {
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
            if !explicit.map_or(sharpness >= policy.sharpness_threshold, |targets| {
                targets.contains(&canonical)
            }) {
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
        point_normals: Vec::new(),
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

        // Validate every selected edge before tracing. In particular, a
        // zero-length edge is invalid mesh input: Boolean stitching owns seam
        // identity canonicalization, and rounding must not silently weld
        // topology using only coordinates.
        let mut sweeps = Vec::with_capacity(selected.len());
        for edge in selected {
            sweeps.push(self.edge_sweep(edge)?);
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

        // Flank substitutions.
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
                let direction = sub(
                    self.position(chain.verts[(index + 1) % chain.vertex_count()]),
                    self.position(chain.verts[index]),
                );
                // Opposite corner trims can cross while every face retains
                // its winding (a cube inset past half its width does this).
                // Every band rail must still advance along its source edge.
                for (&a, &b) in section_a.iter().zip(section_b) {
                    let advance = sub(
                        promote(narrow(self.points[b as usize])),
                        promote(narrow(self.points[a as usize])),
                    );
                    if dot(direction, advance) <= 0.0 {
                        return Err(RoundError::ClearanceExceeded {
                            face: chain.edges[index].left.index(),
                        });
                    }
                }
                for band in 0..chain.segments as usize {
                    let edge = chain.edges[index];
                    let mut sources = [edge.left, edge.right];
                    sources.sort_unstable();
                    let points = [
                        section_b[band],
                        section_a[band],
                        section_a[band + 1],
                        section_b[band + 1],
                    ];
                    let miter_a = chain.miters[index];
                    let miter_b = chain.miters[(index + 1) % chain.vertex_count()];
                    let normals =
                        self.strip_normals(edge, points, [miter_b, miter_a, miter_a, miter_b])?;
                    self.emit_face_with_normals(
                        points.map(Tok::New).to_vec(),
                        RoundFaceSource::Edge(sources),
                        normals,
                    )?;
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

        uv::transfer(self.mesh, &self.points, &mut self.faces);

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
            perimeter += norm(sub(points[(i + 1) % points.len()], points[i]));
        }
        (norm(raw) > SLIVER_NEWELL_FLOOR * perimeter * perimeter)
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
        sweeps: &[f64],
    ) -> Result<Vec<Chain>, RoundError> {
        let mut visited = alloc::vec![false; selected.len()];
        let mut raw = Vec::new();

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
                edges.push(DirEdge {
                    a,
                    b,
                    left,
                    right,
                    sweep: sweeps[edge_index],
                });
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
            let segments = self.chain_segments(&edges, closed)?;
            let count = verts.len();
            chains.push(Chain {
                verts,
                edges,
                closed,
                segments,
                sections: alloc::vec![Vec::new(); count],
                frames: alloc::vec![None; count],
                miters: alloc::vec![false; count],
            });
        }
        Ok(chains)
    }

    fn chain_segments(&self, edges: &[DirEdge], closed: bool) -> Result<u32, RoundError> {
        match self.policy.kind {
            RoundKind::Chamfer { .. } => Ok(1),
            RoundKind::Fillet { radius } => {
                if let Some(explicit) = self.policy.segments {
                    return Ok(explicit);
                }
                let mut radius_scale = 1.0_f64;
                for i in 0..edges.len() {
                    if i == 0 && !closed {
                        continue;
                    }
                    let previous = edges[(i + edges.len() - 1) % edges.len()];
                    if let Some(miter) = self.miter(previous, edges[i], edges[i].a)? {
                        radius_scale = radius_scale.max(miter.radius_scale());
                    }
                }
                let ratio =
                    (1.0 - self.policy.chord_tolerance / (radius * radius_scale)).clamp(-1.0, 1.0);
                let theta = 2.0 * ratio.acos_ext();
                let max_sweep = edges.iter().fold(0.0_f64, |acc, e| acc.max(e.sweep));
                let required = (max_sweep / theta).ceil_ext();
                if !required.is_finite() || required > 256.0 {
                    return Err(RoundError::InvalidPolicy {
                        detail: "chord tolerance needs more than 256 bands",
                    });
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "ceiling of a small positive ratio"
                )]
                let bands = required as u32;
                Ok(bands.max(1))
            }
        }
    }

    fn push_point(&mut self, point: [f64; 3]) -> u32 {
        let id = u32::try_from(self.points.len()).expect("point count fits u32");
        self.points.push(point);
        self.point_normals.push(None);
        id
    }

    fn push_round_point(&mut self, point: [f64; 3], center: Option<[f64; 3]>) -> u32 {
        let id = self.push_point(point);
        self.point_normals[id as usize] = center.and_then(|c| normalize(sub(point, c)));
        id
    }

    fn emit_face(&mut self, entries: Vec<Tok>, source: RoundFaceSource) -> Result<(), RoundError> {
        self.emit_face_with_normals(entries, source, None)
    }

    fn emit_face_with_normals(
        &mut self,
        entries: Vec<Tok>,
        source: RoundFaceSource,
        fillet_normals: Option<Vec<[f32; 3]>>,
    ) -> Result<(), RoundError> {
        let region = if source.is_generated() {
            self.policy.region
        } else {
            None
        }
        .or_else(|| {
            self.mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .and_then(|layer| layer.get(source.faces()[0].as_id()).copied())
        });
        let normals = if let RoundFaceSource::Face(face) = source {
            let normal = narrow(self.plane(face)?.normal);
            entries
                .iter()
                .map(|entry| {
                    if let Tok::Old(vertex) = entry {
                        self.mesh
                            .face_loop(face)
                            .find(|&edge| self.mesh.to_vertex(edge) == Some(*vertex))
                            .and_then(|edge| {
                                self.mesh
                                    .attrs()
                                    .sparse(attr::CORNER_NORMAL_OVERRIDE)
                                    .and_then(|layer| layer.get(edge.as_id()).copied())
                            })
                            .unwrap_or(normal)
                    } else {
                        normal
                    }
                })
                .collect()
        } else {
            // Check the emitted precision as well as the construction
            // geometry: a positive radius can still collapse a band at f32.
            let points: Vec<_> = entries
                .iter()
                .map(|entry| match entry {
                    Tok::Old(v) => self.position(*v),
                    Tok::New(p) => promote(narrow(self.points[*p as usize])),
                })
                .collect();
            let clearance = RoundError::ClearanceExceeded {
                face: source.faces()[0].index(),
            };
            if points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .any(|(a, b)| a == b)
            {
                return Err(clearance);
            }
            let normal = normalize(newell(&points)).ok_or(clearance)?;
            if let Some(normals) = fillet_normals {
                normals
            } else if matches!(self.policy.kind, RoundKind::Fillet { .. }) {
                entries
                    .iter()
                    .map(|entry| {
                        let Tok::New(point) = entry else {
                            unreachable!("generated faces use generated points")
                        };
                        self.point_normals[*point as usize]
                            .map(narrow)
                            .ok_or(clearance)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                alloc::vec![narrow(normal); entries.len()]
            }
        };
        self.faces.push(NewFace {
            entries,
            region,
            source,
            normals,
            uvs: Vec::new(),
        });
        Ok(())
    }

    /// A miter lies on two cylinders. Each adjoining strip must use its own
    /// cylinder's radial normals, leaving a crease where those surfaces meet.
    fn strip_normals(
        &mut self,
        edge: DirEdge,
        points: [u32; 4],
        miters: [bool; 4],
    ) -> Result<Option<Vec<[f32; 3]>>, RoundError> {
        let RoundKind::Fillet { radius } = self.policy.kind else {
            return Ok(None);
        };
        if !miters.iter().any(|&miter| miter) {
            return Ok(None);
        }
        let degenerate = RoundError::DegenerateEdge {
            a: edge.a.index(),
            b: edge.b.index(),
        };
        let anchor = self.position(edge.a);
        let direction = normalize(sub(self.position(edge.b), anchor)).ok_or(degenerate)?;
        let rows = [
            self.plane(edge.left)?.normal,
            self.plane(edge.right)?.normal,
            direction,
        ];
        let center = add(
            anchor,
            solve3(rows, [-radius, -radius, 0.0]).ok_or(degenerate)?,
        );
        points
            .into_iter()
            .zip(miters)
            .map(|(point, miter)| {
                let normal = if miter {
                    let offset = sub(self.points[point as usize], center);
                    normalize(sub(offset, scale(direction, dot(offset, direction))))
                } else {
                    self.point_normals[point as usize]
                };
                normal.map(narrow).ok_or(degenerate)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    /// Two edges on a common flank plane with equal dihedral angles meet at
    /// a miter. Intersect their offset planes at each profile sample instead
    /// of shrinking the setback by averaging the chain's turn directions.
    fn miter(
        &self,
        previous: DirEdge,
        next: DirEdge,
        vertex: VertexId,
    ) -> Result<Option<Miter>, RoundError> {
        // Every selected edge's planes were prepared by edge_sweep before
        // chains or their sampling density are planned.
        let left = [
            self.planes[&previous.left].normal,
            self.planes[&next.left].normal,
        ];
        let right = [
            self.planes[&previous.right].normal,
            self.planes[&next.right].normal,
        ];
        // Faces may be separate coplanar fragments. Their planes meet at the
        // shared vertex; the angular allowance covers normals from stored f32
        // coordinates, without treating an ordinary faceted turn as coplanar.
        let shared_left = dot(left[0], left[1]) > 1.0 - 1e-12;
        let shared_right = dot(right[0], right[1]) > 1.0 - 1e-12;
        if shared_left == shared_right || (previous.sweep - next.sweep).abs() > 1e-6 {
            return Ok(None);
        }
        let rows = if shared_left {
            [left[0], right[0], right[1]]
        } else {
            [right[0], left[0], left[1]]
        };
        let unsupported = RoundError::UnsupportedJunction {
            vertex: vertex.index(),
        };
        let common = solve3(rows, [1.0, 0.0, 0.0]).ok_or(unsupported)?;
        let sides = solve3(rows, [0.0, 1.0, 1.0]).ok_or(unsupported)?;
        let sweep = (previous.sweep + next.sweep) * 0.5;
        Ok(Some(Miter {
            common,
            sides,
            sweep,
            reversed: shared_right,
        }))
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
            // Stored f32 positions can put a rotated right angle fractionally
            // above its exact policy boundary.
            if directions.len() == 2
                && dot(directions[0], directions[1])
                    .clamp(-1.0, 1.0)
                    .acos_ext()
                    > self.policy.max_tangent_turn + 1e-6
            {
                return Err(RoundError::UnsupportedJunction {
                    vertex: vertex.index(),
                });
            }
            let tangent = normalize(tangent).ok_or(degenerate)?;
            let left_normal = normalize(left_normal).ok_or(degenerate)?;
            let right_normal = normalize(right_normal).ok_or(degenerate)?;
            if let (Some(previous), Some(next)) = (prev, next)
                && let Some(miter) = self.miter(chain.edges[previous], chain.edges[next], vertex)?
            {
                sections[index] = miter
                    .points(self.position(vertex), self.policy.kind, chain.segments)
                    .into_iter()
                    .map(|point| self.push_point(point))
                    .collect();
                frames[index] = Some((left_normal, right_normal));
                chains[chain_index].miters[index] = true;
                continue;
            }
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
            let center = match self.policy.kind {
                RoundKind::Chamfer { .. } => None,
                RoundKind::Fillet { radius } => Some(scale(
                    add(
                        sub(left_point, scale(left_normal, radius)),
                        sub(right_point, scale(right_normal, radius)),
                    ),
                    0.5,
                )),
            };
            let section_points = if let Some(center) = center {
                arc_points(center, left_point, right_point, chain.segments).ok_or(degenerate)?
            } else {
                alloc::vec![left_point, right_point]
            };
            sections[index] = section_points
                .into_iter()
                .map(|p| self.push_round_point(p, center))
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
                        let id = self.push_round_point(point, Some(center));
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
                            ids.push(self.push_round_point(*point, Some(center)));
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

        // Adjacent substitution images legitimately share endpoints (for
        // example an end-face splice meeting its flank point), so normalize
        // those consecutive tokens. Any face that then has fewer than three
        // corners exceeded geometric clearance; rounding does not delete
        // source topology to make the rewrite fit.
        let mut entries: Vec<Tok> = images.iter().flatten().copied().collect();
        entries.dedup();
        while entries.len() > 1 && entries.first() == entries.last() {
            entries.pop();
        }
        if entries.len() < 3 {
            return Err(RoundError::ClearanceExceeded { face: face.index() });
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
                Tok::New(p) => promote(narrow(self.points[*p as usize])),
            })
            .collect();
        let old_normal = newell(&old_points);
        let new_normal = newell(&new_points);
        if normalize(old_normal)
            .zip(normalize(new_normal))
            .is_none_or(|(old_unit, new_unit)| dot(old_unit, new_unit) <= 0.0)
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

        self.emit_face(entries, RoundFaceSource::Face(face))
    }

    /// Emits corner patches by walking each corner's boundary ring.
    fn emit_corner_patches(
        &mut self,
        chains: &[Chain],
        corners: &[CornerPlan],
    ) -> Result<(), RoundError> {
        for (vertex, center, ends) in corners {
            let mut faces: Vec<_> = ends
                .iter()
                .flat_map(|&(index, at_end)| {
                    let edge = chains[index].end_edge(at_end);
                    [edge.left, edge.right]
                })
                .collect();
            faces.sort_unstable();
            faces.dedup();
            let source = RoundFaceSource::Corner(faces.try_into().expect("trihedral corner"));
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

            if let (RoundKind::Fillet { radius }, Some(center)) = (self.policy.kind, center) {
                self.emit_fillet_corner(&ring, *center, radius, source)?;
                continue;
            }

            self.emit_face(ring.iter().map(|&p| Tok::New(p)).collect(), source)?;
            self.stats.patch_faces += 1;
        }
        Ok(())
    }

    /// Fill a spherical corner without changing the strip's boundary samples.
    /// Increasing only that boundary's density leaves the old center fan's
    /// long radial edges unchanged, so its surface error never converges.
    fn emit_fillet_corner(
        &mut self,
        ring: &[u32],
        center: [f64; 3],
        radius: f64,
        source: RoundFaceSource,
    ) -> Result<(), RoundError> {
        let clearance = RoundError::ClearanceExceeded {
            face: source.faces()[0].index(),
        };
        let boundary: Vec<_> = ring.iter().map(|&p| self.points[p as usize]).collect();
        let mean = scale(
            boundary
                .iter()
                .fold([0.0; 3], |sum, &p| add(sum, sub(p, center))),
            1.0 / ring.len() as f64,
        );
        let apex = add(center, scale(normalize(mean).ok_or(clearance)?, radius));
        // Explicit sampling controls both the strips and interior layers.
        // Otherwise bracket a passing layer count, then bisect the bracket to
        // avoid rounding the whole patch up to a power of two. Keep the last
        // passing rays: every accepted triangle must bound the sphere deviation,
        // even if numerical error makes acceptance non-monotonic.
        let mut layers = self.policy.segments.unwrap_or(1);
        let mut coarse_layers = 0;
        let mut fine_layers = 256;
        let mut accepted_rays = None;
        let rays = loop {
            let rays: Vec<_> = boundary
                .iter()
                .map(|&point| arc_points(center, apex, point, layers).ok_or(clearance))
                .collect::<Result<_, _>>()?;
            if self.policy.segments.is_some() {
                break rays;
            }
            let within_tolerance = |triangle: [[f64; 3]; 3]| {
                spherical_triangle_error(triangle.map(|p| sub(p, center)), radius)
                    .is_some_and(|error| error <= self.policy.chord_tolerance)
            };
            let mut acceptable = true;
            for i in 0..ring.len() {
                let next = (i + 1) % ring.len();
                acceptable &= within_tolerance([apex, rays[i][1], rays[next][1]]);
                for layer in 1..layers as usize {
                    let quad = [
                        rays[i][layer],
                        rays[i][layer + 1],
                        rays[next][layer + 1],
                        rays[next][layer],
                    ];
                    acceptable &= corner_quad(quad)
                        .iter()
                        .all(|indices| within_tolerance(indices.map(|i| quad[i])));
                }
            }
            if acceptable {
                fine_layers = layers;
                accepted_rays = Some(rays);
            } else {
                coarse_layers = layers;
            }
            if fine_layers - coarse_layers <= 1 {
                break accepted_rays.ok_or(RoundError::InvalidPolicy {
                    detail: "corner tolerance needs more than 256 radial layers",
                })?;
            }
            layers = if accepted_rays.is_some() {
                coarse_layers + (fine_layers - coarse_layers) / 2
            } else {
                (layers * 2).min(256)
            };
        };
        // The final probe may have failed; emission uses the retained passing
        // sample count, or the caller's explicit count.
        let layers = self.policy.segments.unwrap_or(fine_layers);
        let apex = self.push_round_point(apex, Some(center));
        // Grow inward from the existing strip boundary so every insertion
        // shares an edge with the mesh, never just an isolated boundary vertex.
        let mut outer = ring.to_vec();
        for layer in (1..=layers as usize).rev() {
            let inner = if layer == 1 {
                alloc::vec![apex; ring.len()]
            } else {
                rays.iter()
                    .map(|ray| self.push_round_point(ray[layer - 1], Some(center)))
                    .collect()
            };
            for i in 0..ring.len() {
                let next = (i + 1) % ring.len();
                if layer == 1 {
                    self.emit_face(
                        alloc::vec![Tok::New(apex), Tok::New(outer[i]), Tok::New(outer[next])],
                        source,
                    )?;
                    self.stats.patch_faces += 1;
                } else {
                    let quad = [inner[i], outer[i], outer[next], inner[next]];
                    for triangle in corner_quad(quad.map(|p| self.points[p as usize])) {
                        self.emit_face(triangle.map(|i| Tok::New(quad[i])).to_vec(), source)?;
                        self.stats.patch_faces += 1;
                    }
                }
            }
            outer = inner;
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

/// Averaged side normals and section endpoints for a non-corner chain
/// vertex, used to classify vertex-only faces.
fn vertex_side_points(
    chains: &[Chain],
    vertex: VertexId,
) -> Option<([f64; 3], [f64; 3], u32, u32)> {
    for chain in chains {
        for (index, &v) in chain.verts.iter().enumerate() {
            if v != vertex {
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

fn apply(mesh: &mut Mesh, plan: Plan) -> Result<RoundResult, RoundError> {
    let mut staged = mesh.clone();
    let stats = apply_staged(&mut staged, plan)?;
    *mesh = staged;
    Ok(stats)
}

fn apply_staged(mesh: &mut Mesh, plan: Plan) -> Result<RoundResult, RoundError> {
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
        let face = match add_face(&mut session, &loop_vertices) {
            Ok(face) => face,
            Err(AddFaceError::NonManifoldVertex { .. }) => {
                #[expect(unused_must_use, reason = "discard sink output")]
                {
                    session.finish();
                }
                return Err(RoundError::UnsupportedTopology {
                    detail: "face rewrite would pinch an OUTSIDE boundary vertex",
                });
            }
            Err(_) => {
                #[expect(unused_must_use, reason = "discard sink output")]
                {
                    session.finish();
                }
                return Err(RoundError::Internal {
                    detail: "planned face was rejected by add_face",
                });
            }
        };
        if let Some(region) = planned.region {
            let _ = set_face_region(&mut session, face, region);
        }
        let attributes: BTreeMap<_, _> = loop_vertices
            .iter()
            .copied()
            .enumerate()
            .map(|(i, vertex)| {
                (
                    vertex,
                    (planned.normals[i], planned.uvs.get(i).copied().flatten()),
                )
            })
            .collect();
        let corners: Vec<_> = session.mesh().face_loop(face).collect();
        for corner in corners {
            let vertex = session.mesh().to_vertex(corner).expect("live new corner");
            let (normal, uv) = attributes[&vertex];
            let _ = set_corner_normal_override(&mut session, corner, Some(normal));
            if let Some(uv) = uv {
                let _ = set_corner_uv(&mut session, corner, uv);
            }
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

    // UV corners belong to each half-edge's destination vertex. Compare both
    // endpoints using the preceding corner in the opposite loop. Preserve
    // authored seams even when their UVs happen to agree.
    if session.mesh().attrs().sparse(attr::CORNER_UV).is_some() {
        for &edge in new_half_edges.values() {
            let twin = session.mesh().twin(edge).expect("new edge has a twin");
            let previous = session.mesh().prev(edge).expect("live loop");
            let twin_previous = session.mesh().prev(twin).expect("live twin loop");
            let bits = |corner| session.corner_uv(corner).map(|uv| uv.map(f32::to_bits));
            if bits(edge) != bits(twin_previous) || bits(previous) != bits(twin) {
                let _ = set_edge_seam(&mut session, edge, true);
            }
        }
    }

    // Tangent fillet joins remain smooth. Chamfer boundaries and fillet end
    // rims stay hard, including edges introduced by a transverse-face splice.
    let generated: BTreeSet<_> = added
        .iter()
        .zip(&plan.faces)
        .filter_map(|(&face, planned)| planned.source.is_generated().then_some(face))
        .collect();
    for &edge in new_half_edges.values() {
        let twin = session.mesh().twin(edge).expect("new edge has a twin");
        if !session
            .mesh()
            .face(edge)
            .is_some_and(|f| generated.contains(&f))
        {
            continue;
        }
        let corners = [
            edge,
            session.mesh().prev(edge).expect("live loop"),
            twin,
            session.mesh().prev(twin).expect("live twin loop"),
        ];
        let normals = corners.map(|corner| session.corner_normal_override(corner));
        let continuous = match normals {
            [Some(b), Some(a), Some(other_a), Some(other_b)] => {
                dot(promote(a), promote(other_a)) > 0.99999
                    && dot(promote(b), promote(other_b)) > 0.99999
            }
            _ => false,
        };
        if !continuous {
            let _ = set_edge_sharpness(&mut session, edge, 1.0);
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
    Ok(RoundResult {
        stats: plan.stats,
        face_provenance: added
            .into_iter()
            .zip(plan.faces.into_iter().map(|f| f.source))
            .collect(),
    })
}
