// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Patch classification: which side of the other mesh each face patch
//! lies on.
//!
//! After splitting, the intersection curves are real edge chains on both
//! meshes, so patches are purely topological: connected face regions
//! bounded by cut edges. Classification floods faces across shared edges
//! (never crossing a cut edge), then decides Inside/Outside per patch by
//! exact ray parity against the other mesh — a segment-triangle test built
//! entirely from [`exedra_triangulate::predicates::orient3d`] signs over
//! exactly promoted coordinates. Rays that graze anything retry with the
//! next direction from a fixed, documented set; only if every direction
//! degenerates does the patch classify as [`PatchSide::Suspect`]. A ray
//! endpoint lying on a triangle's supporting plane but exactly outside
//! the triangle is a miss, not a graze — split meshes carry whole fans of
//! triangles exactly in the cut planes, and sample points on those planes
//! would otherwise never classify.
//!
//! Coplanar face-on-face contact regions (recorded before splitting by
//! [`super::collect_coplanar_contacts`]) cannot be ray-classified — their
//! sample points lie exactly on the other mesh — so they classify first,
//! by exact 2D containment of the patch faces in the counterpart face
//! polygons: such patches become [`PatchSide::Boundary`] carrying the
//! outward-normal agreement the stitch selection table consumes.
//!
//! Suspicion is typed, never guessed: a patch is suspect when its cut
//! network is provably incomplete (a graph edge attributed to one of its
//! faces neither materialized as a mesh edge nor split the face — the
//! splitting stage deferred it), when a coplanar contact region is not
//! cleanly separated from its surroundings, or when ray classification
//! exhausts its direction set. Suspect patches poison the boolean op
//! downstream rather than silently producing wrong geometry.
//!
//! Determinism: faces flood in ascending index order, patches emerge in
//! ascending lowest-face order (mesh A before mesh B), sample points and
//! ray directions derive from fixed rules, and every geometric sign is
//! exact.

use alloc::vec::Vec;

use exedra_triangulate::predicates::{Orientation, Orientation3d, orient2d, orient3d};
use hashbrown::{HashMap, HashSet};

use super::coplanar::{
    CoplanarContact, Placement, dominant_axis, place_point_in_polygon, project_point,
};
use super::diag::{BooleanDiagnostic, BooleanDiagnostics, BooleanFailureKind};
use super::graph::IntersectionGraph;
use super::split::{MeshSide, MeshSplitOutcome};
use crate::{FaceId, FaceTriangulation, Mesh, VertexId};
use exedra_math::promote;

/// Which side of the other mesh a patch lies on.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PatchSide {
    /// The patch lies inside the other mesh's volume.
    Inside,
    /// The patch lies outside the other mesh's volume.
    Outside,
    /// The patch lies exactly on the other mesh's boundary: a coplanar
    /// face-on-face contact region.
    Boundary {
        /// True when the operands' outward normals oppose across the
        /// contact (typical touching solids); false when they agree
        /// (flush overlapping boundaries).
        opposed: bool,
    },
    /// The patch could not be classified soundly (incomplete cut network,
    /// an uncarved coplanar contact, or exhausted ray directions); typed
    /// poison for downstream stages.
    Suspect,
}

/// One connected face patch of one mesh, bounded by cut edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// Which operand mesh the patch belongs to.
    pub mesh: MeshSide,
    /// Patch faces, ascending by index.
    pub faces: Vec<FaceId>,
    /// Classified side relative to the other mesh.
    pub side: PatchSide,
}

/// Deterministic classification counters.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ClassifyStats {
    /// Patches found on mesh A.
    pub patches_a: u64,
    /// Patches found on mesh B.
    pub patches_b: u64,
    /// Ray-parity tests performed.
    pub ray_tests: u64,
    /// Ray directions retried due to degeneracy.
    pub ray_retries: u64,
    /// Patches classified suspect.
    pub suspect_patches: u64,
}

/// The classification of both meshes' patches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatchClassification {
    /// All patches: mesh A's first, then mesh B's, each in ascending
    /// lowest-face order.
    pub patches: Vec<Patch>,
    /// Classification counters.
    pub stats: ClassifyStats,
}

impl PatchClassification {
    /// Iterates the patches of one mesh side.
    pub fn of(&self, side: MeshSide) -> impl Iterator<Item = &Patch> {
        self.patches.iter().filter(move |p| p.mesh == side)
    }

    /// True when any patch is suspect.
    #[must_use]
    pub fn any_suspect(&self) -> bool {
        self.patches.iter().any(|p| p.side == PatchSide::Suspect)
    }
}

/// Classifies the split meshes' patches against each other.
///
/// Both meshes must already be split along `graph` (the outcomes carry the
/// materialized cut vertices). `contacts` are the coplanar face-on-face
/// contacts recorded on the pre-split meshes by
/// [`super::collect_coplanar_contacts`] (pass an empty slice when no
/// coplanar handling is wanted). `strategy` is the triangulation strategy
/// the pipeline runs under (used to enumerate the other mesh's triangles
/// for ray parity and to sample face interiors).
#[expect(
    clippy::too_many_arguments,
    reason = "pipeline stage threading fixed pipeline context"
)]
pub fn classify_patches(
    mesh_a: &Mesh,
    mesh_b: &Mesh,
    graph: &IntersectionGraph,
    outcome_a: &MeshSplitOutcome,
    outcome_b: &MeshSplitOutcome,
    contacts: &[CoplanarContact],
    strategy: FaceTriangulation,
    scratch: &mut super::BooleanScratch,
    diagnostics: &mut BooleanDiagnostics,
) -> PatchClassification {
    let mut classification = PatchClassification::default();
    let mut buffer = core::mem::take(&mut scratch.narrow_face_a);

    for (side, mesh, other, outcome) in [
        (MeshSide::A, mesh_a, mesh_b, outcome_a),
        (MeshSide::B, mesh_b, mesh_a, outcome_b),
    ] {
        let patches = classify_one_side(
            side,
            mesh,
            other,
            graph,
            outcome,
            contacts,
            strategy,
            &mut buffer,
            &mut classification.stats,
            diagnostics,
        );
        match side {
            MeshSide::A => classification.stats.patches_a = patches.len() as u64,
            MeshSide::B => classification.stats.patches_b = patches.len() as u64,
        }
        classification.patches.extend(patches);
    }

    scratch.narrow_face_a = buffer;
    classification.stats.suspect_patches = classification
        .patches
        .iter()
        .filter(|p| p.side == PatchSide::Suspect)
        .count() as u64;
    classification
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal stage threading fixed pipeline context"
)]
fn classify_one_side(
    side: MeshSide,
    mesh: &Mesh,
    other: &Mesh,
    graph: &IntersectionGraph,
    outcome: &MeshSplitOutcome,
    contacts: &[CoplanarContact],
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
    stats: &mut ClassifyStats,
    diagnostics: &mut BooleanDiagnostics,
) -> Vec<Patch> {
    // --- Coplanar contacts keyed by this side's pre-split face, with the
    // split stage's origin mapping to reach post-split faces.
    let mut contacts_by_face: HashMap<FaceId, Vec<u32>> = HashMap::new();
    for (index, contact) in contacts.iter().enumerate() {
        let key = match side {
            MeshSide::A => contact.face_a,
            MeshSide::B => contact.face_b,
        };
        contacts_by_face
            .entry(key)
            .or_default()
            .push(u32::try_from(index).unwrap_or(u32::MAX));
    }
    let origins: HashMap<FaceId, FaceId> = outcome.face_origins.iter().copied().collect();
    // --- Cut edges as sorted mesh-vertex pairs.
    let mut cut_edges: HashSet<(VertexId, VertexId)> = HashSet::new();
    let mut cut_vertices: HashSet<VertexId> = HashSet::new();
    let mut incomplete_cut = false;
    for polyline in &graph.polylines {
        let count = polyline.vertices.len();
        let edges = if polyline.closed {
            count
        } else {
            count.saturating_sub(1)
        };
        for i in 0..edges {
            let p = polyline.vertices[i] as usize;
            let q = polyline.vertices[(i + 1) % count] as usize;
            let (Some(u), Some(v)) = (outcome.graph_vertices[p], outcome.graph_vertices[q]) else {
                // The cut edge never materialized on this side: splitting
                // deferred it, so the cut network is incomplete.
                incomplete_cut = true;
                continue;
            };
            cut_vertices.insert(u);
            cut_vertices.insert(v);
            cut_edges.insert(sorted_pair(u, v));
        }
    }

    // --- Suspect faces: a graph edge attributed solely to a live face on
    // this side whose endpoints do not form a mesh edge means the split
    // stage deferred that face.
    let mut suspect_faces: HashSet<FaceId> = HashSet::new();
    if incomplete_cut {
        for edge in &graph.edges {
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
            if faces.len() != 1 {
                continue;
            }
            let face = faces[0];
            // Still live means never re-partitioned.
            let live = mesh.face_edge(face).is_some();
            let [p, q] = edge.vertices;
            let materialized = outcome.graph_vertices[p as usize]
                .zip(outcome.graph_vertices[q as usize])
                .is_some_and(|(u, v)| cut_edges.contains(&sorted_pair(u, v)));
            if live && !materialized {
                suspect_faces.insert(face);
            }
        }
    }

    // --- Coplanar membership is also a patch boundary. The contact outline
    // can follow an existing mesh edge, in which case the transversal
    // intersection graph contributes no cut edge there. Classify each face
    // before flooding so contact faces cannot merge with adjacent clear
    // surface faces across that existing edge.
    let faces: Vec<FaceId> = mesh.faces().collect();
    let zero_area_faces: Vec<FaceId> = faces
        .iter()
        .copied()
        .filter(|&face| !face_has_area(mesh, face, strategy, buffer))
        .collect();
    let mut face_contacts: HashMap<FaceId, PatchContact> = HashMap::new();
    for &face in &faces {
        let origin = origins.get(&face).copied().unwrap_or(face);
        let membership = contacts_by_face
            .get(&origin)
            .map_or(PatchContact::Clear, |indices| {
                face_contact(mesh, face, contacts, indices, side, strategy, buffer)
            });
        face_contacts.insert(face, membership);
    }
    // A split can leave a collinear bookkeeping face exactly on a contact
    // outline. It carries the edge subdivision needed by neighboring faces
    // but has no interior point to classify. Inherit a consistent adjacent
    // contact rather than isolating it as a clear patch and asking ray parity
    // to classify a point on the boundary.
    // Iterate in stable face order to a fixed point: a run of adjacent
    // bookkeeping faces may need the contact verdict to propagate across
    // more than one face. Each pass changes Clear to a terminal verdict, so
    // convergence is bounded by the number of zero-area faces.
    loop {
        let mut changed = false;
        for &face in &zero_area_faces {
            if face_contacts.get(&face) != Some(&PatchContact::Clear) {
                continue;
            }
            let mut inherited: Option<bool> = None;
            for half_edge in mesh.face_loop(face) {
                let Some(neighbor) = mesh.twin(half_edge).and_then(|twin| mesh.face(twin)) else {
                    continue;
                };
                match face_contacts.get(&neighbor).copied() {
                    Some(PatchContact::Contact { opposed }) => match inherited {
                        None => inherited = Some(opposed),
                        Some(seen) if seen == opposed => {}
                        Some(_) => {
                            face_contacts.insert(face, PatchContact::Ambiguous);
                            inherited = None;
                            changed = true;
                            break;
                        }
                    },
                    Some(PatchContact::Ambiguous) => {
                        face_contacts.insert(face, PatchContact::Ambiguous);
                        inherited = None;
                        changed = true;
                        break;
                    }
                    Some(PatchContact::Clear) | None => {}
                }
            }
            if let Some(opposed) = inherited {
                face_contacts.insert(face, PatchContact::Contact { opposed });
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut contact_edges: HashSet<(VertexId, VertexId)> = HashSet::new();
    for &face in &faces {
        for half_edge in mesh.face_loop(face) {
            let (Some(from), Some(to), Some(twin)) = (
                mesh.from_vertex(half_edge),
                mesh.to_vertex(half_edge),
                mesh.twin(half_edge),
            ) else {
                continue;
            };
            let Some(neighbor) = mesh.face(twin) else {
                continue;
            };
            if neighbor == FaceId::OUTSIDE {
                continue;
            }
            if face_contacts.get(&face) != face_contacts.get(&neighbor) {
                cut_vertices.insert(from);
                cut_vertices.insert(to);
                contact_edges.insert(sorted_pair(from, to));
            }
        }
    }

    // --- Flood fill across shared edges, stopping at transversal cuts and
    // coplanar-contact boundaries.
    let mut assigned: HashMap<FaceId, usize> = HashMap::new();
    let mut patches: Vec<Patch> = Vec::new();

    for &seed in &faces {
        if assigned.contains_key(&seed) {
            continue;
        }
        let patch_index = patches.len();
        let mut patch_faces = Vec::new();
        let mut stack = alloc::vec![seed];
        assigned.insert(seed, patch_index);
        while let Some(face) = stack.pop() {
            patch_faces.push(face);
            for half_edge in mesh.face_loop(face) {
                let (Some(from), Some(to)) =
                    (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
                else {
                    continue;
                };
                let edge = sorted_pair(from, to);
                if cut_edges.contains(&edge) || contact_edges.contains(&edge) {
                    continue;
                }
                let Some(twin) = mesh.twin(half_edge) else {
                    continue;
                };
                let Some(neighbor) = mesh.face(twin) else {
                    continue;
                };
                if neighbor == FaceId::OUTSIDE || assigned.contains_key(&neighbor) {
                    continue;
                }
                assigned.insert(neighbor, patch_index);
                stack.push(neighbor);
            }
        }
        patch_faces.sort_unstable_by_key(|f| f.index());
        patches.push(Patch {
            mesh: side,
            faces: patch_faces,
            side: PatchSide::Suspect, // classified below
        });
    }

    // --- Classify each patch.
    for patch in &mut patches {
        if patch.faces.iter().any(|f| suspect_faces.contains(f)) {
            patch.side = PatchSide::Suspect;
            diagnostics.push(BooleanDiagnostic {
                kind: BooleanFailureKind::SplitDeferred,
                a: None,
                b: None,
                detail: "patch borders a face whose split was deferred",
            });
            continue;
        }
        // --- Coplanar contact regions classify by exact 2D containment;
        // their sample points lie on the other mesh, so rays cannot see
        // them. A patch must be entirely contact (consistent normal
        // agreement) or entirely clear — a mix means the contact region
        // was not carved out cleanly, which is typed, not guessed.
        match patch_contact(patch, &face_contacts) {
            PatchContact::Clear => {}
            PatchContact::Contact { opposed } => {
                patch.side = PatchSide::Boundary { opposed };
                continue;
            }
            PatchContact::Ambiguous => {
                patch.side = PatchSide::Suspect;
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::CoplanarAmbiguity,
                    a: None,
                    b: None,
                    detail: "coplanar contact region is not cleanly separated in this patch",
                });
                continue;
            }
        }
        let sample = sample_point(mesh, patch, &cut_vertices, strategy, buffer);
        stats.ray_tests += 1;
        match ray_parity(sample, other, strategy, buffer, &mut stats.ray_retries) {
            Some(true) => patch.side = PatchSide::Inside,
            Some(false) => patch.side = PatchSide::Outside,
            None => {
                patch.side = PatchSide::Suspect;
                diagnostics.push(BooleanDiagnostic {
                    kind: BooleanFailureKind::NumericalInstability,
                    a: None,
                    b: None,
                    detail: "ray parity exhausted its direction set",
                });
            }
        }
    }

    patches
}

fn sorted_pair(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a.index() <= b.index() {
        (a, b)
    } else {
        (b, a)
    }
}

/// True when the face contributes positive-area surface geometry under the
/// pipeline's triangulation strategy. Split operations can leave collinear
/// bookkeeping faces where an intersection follows an existing edge; those
/// faces bound no volume and must not become classifiable surface patches.
fn face_has_area(
    mesh: &Mesh,
    face: FaceId,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> bool {
    let _ = mesh.face_triangles_into(face, strategy, buffer);
    buffer.iter().any(|triangle| {
        let mut corners = [[0.0_f64; 3]; 3];
        for (slot, corner) in corners.iter_mut().zip(triangle) {
            let Some(position) = mesh
                .to_vertex(*corner)
                .and_then(|vertex| mesh.vertex_position(vertex))
            else {
                return false;
            };
            *slot = promote(*position);
        }
        !triangle_is_flat(corners)
    })
}

/// Whether a patch is a coplanar contact region.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PatchContact {
    /// No face of the patch lies in a contact region.
    Clear,
    /// Every face lies in a contact region with consistent normal
    /// agreement.
    Contact {
        /// Outward-normal agreement across the contact.
        opposed: bool,
    },
    /// Mixed or undecidable membership; typed suspicion.
    Ambiguous,
}

fn patch_contact(patch: &Patch, face_contacts: &HashMap<FaceId, PatchContact>) -> PatchContact {
    let mut contact: Option<bool> = None;
    let mut clear = false;
    for &face in &patch.faces {
        match face_contacts
            .get(&face)
            .copied()
            .unwrap_or(PatchContact::Clear)
        {
            PatchContact::Clear => clear = true,
            PatchContact::Contact { opposed } => match contact {
                None => contact = Some(opposed),
                Some(seen) if seen == opposed => {}
                Some(_) => return PatchContact::Ambiguous,
            },
            PatchContact::Ambiguous => return PatchContact::Ambiguous,
        }
    }
    match contact {
        Some(_) if clear => PatchContact::Ambiguous,
        Some(opposed) => PatchContact::Contact { opposed },
        None => PatchContact::Clear,
    }
}

/// Decides whether one post-split face lies inside a counterpart contact
/// polygon.
///
/// Splitting carves contact regions along their boundary curves, so a
/// face is expected to be entirely inside or entirely outside each
/// contact: a strictly-inside vertex with no strictly-outside vertex is
/// containment, the reverse is clearance, a mix is typed ambiguity. When
/// every vertex sits exactly on the counterpart boundary (a face that
/// coincides with the whole contact region), an interior sample from the
/// face's own triangulation decides — after an exact check that the
/// sample really is interior to the face.
fn face_contact(
    mesh: &Mesh,
    face: FaceId,
    contacts: &[CoplanarContact],
    indices: &[u32],
    side: MeshSide,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> PatchContact {
    let mut contact: Option<bool> = None;
    for &index in indices {
        let entry = &contacts[index as usize];
        let counterpart = match side {
            MeshSide::A => &entry.polygon_b,
            MeshSide::B => &entry.polygon_a,
        };
        let mut strictly_inside = 0_u32;
        let mut strictly_outside = 0_u32;
        let mut own_polygon: Vec<[f64; 2]> = Vec::new();
        for half_edge in mesh.face_loop(face) {
            let Some(p) = mesh
                .to_vertex(half_edge)
                .and_then(|v| mesh.vertex_position(v))
            else {
                continue;
            };
            let projected = project_point(promote(*p), entry.axis);
            own_polygon.push(projected);
            match place_point_in_polygon(projected, counterpart) {
                Placement::Inside => strictly_inside += 1,
                Placement::Outside => strictly_outside += 1,
                Placement::OnBoundary => {}
            }
        }
        if strictly_inside > 0 && strictly_outside > 0 {
            return PatchContact::Ambiguous;
        }
        let inside = if strictly_inside > 0 {
            true
        } else if strictly_outside > 0 {
            false
        } else {
            // Every vertex on the counterpart boundary: decide by an
            // interior sample of this face.
            match interior_sample_placement(
                mesh,
                face,
                entry.axis,
                &own_polygon,
                counterpart,
                strategy,
                buffer,
            ) {
                Some(Placement::Inside) => true,
                Some(Placement::Outside) => false,
                _ => return PatchContact::Ambiguous,
            }
        };
        if inside {
            match contact {
                None => contact = Some(entry.opposed),
                Some(seen) if seen == entry.opposed => {}
                Some(_) => return PatchContact::Ambiguous,
            }
        }
    }
    match contact {
        Some(opposed) => PatchContact::Contact { opposed },
        None => PatchContact::Clear,
    }
}

/// Places a face-interior sample against the counterpart polygon.
///
/// The sample is the centroid of the face's first non-flat triangle under
/// the pipeline's strategy — the same triangles every other stage treats
/// as the face's authoritative geometry. The centroid is an f64
/// construction, so before trusting it the sample must place strictly
/// inside the face's own projected polygon (exact test); otherwise the
/// configuration is reported as undecidable rather than guessed.
fn interior_sample_placement(
    mesh: &Mesh,
    face: FaceId,
    axis: usize,
    own_polygon: &[[f64; 2]],
    counterpart: &[[f64; 2]],
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> Option<Placement> {
    let _ = mesh.face_triangles_into(face, strategy, buffer);
    for triangle in buffer.iter() {
        let mut corners = [[0.0_f64; 3]; 3];
        let mut live = true;
        for (slot, corner) in corners.iter_mut().zip(triangle) {
            match mesh
                .to_vertex(*corner)
                .and_then(|v| mesh.vertex_position(v))
            {
                Some(p) => *slot = promote(*p),
                None => live = false,
            }
        }
        if !live || triangle_is_flat(corners) {
            continue;
        }
        let centroid = project_point(
            [
                (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0,
                (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0,
                (corners[0][2] + corners[1][2] + corners[2][2]) / 3.0,
            ],
            axis,
        );
        if place_point_in_polygon(centroid, own_polygon) != Placement::Inside {
            return None; // Not a trustworthy interior sample.
        }
        return Some(place_point_in_polygon(centroid, counterpart));
    }
    None
}

/// A deterministic sample point for the patch: the lowest-index patch
/// vertex not on the cut curve (exact original geometry), else the
/// centroid of the largest-area triangle across the patch's faces.
///
/// The fallback must stand as far from the cut curve as the patch allows:
/// cut vertices are f32-narrowed constructions, so sliver faces hugging
/// the curve genuinely poke a hair past the other solid's surface, and a
/// sample taken on such a sliver classifies the whole patch by narrowing
/// noise (a through-hole disk patch — every vertex on the cut — once
/// classified Outside this way, leaving the result an open tube). The
/// largest triangle's centroid maximizes clearance deterministically:
/// areas compare in f64 over exactly promoted coordinates, ties keep the
/// first (lowest face, earliest triangle).
fn sample_point(
    mesh: &Mesh,
    patch: &Patch,
    cut_vertices: &HashSet<VertexId>,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
) -> [f64; 3] {
    let mut best: Option<VertexId> = None;
    for &face in &patch.faces {
        for half_edge in mesh.face_loop(face) {
            let Some(vertex) = mesh.to_vertex(half_edge) else {
                continue;
            };
            if cut_vertices.contains(&vertex) {
                continue;
            }
            if best.is_none_or(|b| vertex.index() < b.index()) {
                best = Some(vertex);
            }
        }
    }
    if let Some(vertex) = best
        && let Some(p) = mesh.vertex_position(vertex)
    {
        return promote(*p);
    }
    // All patch vertices lie on the cut: sample the centroid of the
    // largest triangle in the patch (patch faces are sorted ascending).
    let mut best_area = -1.0_f64;
    let mut best_centroid = [0.0_f64; 3];
    for &face in &patch.faces {
        let _ = mesh.face_triangles_into(face, strategy, buffer);
        for triangle in buffer.iter() {
            let mut corners = [[0.0_f64; 3]; 3];
            let mut live = true;
            for (slot, corner) in corners.iter_mut().zip(triangle) {
                match mesh
                    .to_vertex(*corner)
                    .and_then(|v| mesh.vertex_position(v))
                {
                    Some(p) => *slot = promote(*p),
                    None => live = false,
                }
            }
            if !live {
                continue;
            }
            let u = [
                corners[1][0] - corners[0][0],
                corners[1][1] - corners[0][1],
                corners[1][2] - corners[0][2],
            ];
            let v = [
                corners[2][0] - corners[0][0],
                corners[2][1] - corners[0][1],
                corners[2][2] - corners[0][2],
            ];
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let area = cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2];
            if area > best_area {
                best_area = area;
                best_centroid = [
                    (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0,
                    (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0,
                    (corners[0][2] + corners[1][2] + corners[2][2]) / 3.0,
                ];
            }
        }
    }
    best_centroid
}

/// The fixed ray-direction set: mixed-component directions first (least
/// likely to graze axis-aligned geometry), axis-aligned last. Documented
/// order; first non-degenerate ray wins.
const RAY_DIRECTIONS: [[f64; 3]; 8] = [
    [1.0, 0.372, 0.618],
    [0.372, 1.0, 0.618],
    [0.618, 0.372, 1.0],
    [-1.0, 0.531, 0.274],
    [0.274, -1.0, 0.531],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
];

/// Exact ray parity of `point` against `mesh`: `Some(true)` when inside.
///
/// Casts a segment from `point` to a far point beyond the mesh bounds and
/// counts strict triangle crossings with exact [`orient3d`] signs. Any
/// coplanar sign anywhere retries with the next direction; `None` when
/// every direction degenerates.
fn ray_parity(
    point: [f64; 3],
    mesh: &Mesh,
    strategy: FaceTriangulation,
    buffer: &mut Vec<[crate::CornerId; 3]>,
    retries: &mut u64,
) -> Option<bool> {
    // Far-point scale from the mesh bounds plus the sample point.
    let mut min = point;
    let mut max = point;
    for vertex in mesh.vertices() {
        if let Some(p) = mesh.vertex_position(vertex) {
            let p = promote(*p);
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
    }
    let diameter = (max[0] - min[0]) + (max[1] - min[1]) + (max[2] - min[2]);
    let scale = 4.0 * diameter.max(1.0);

    'directions: for direction in RAY_DIRECTIONS {
        let far = [
            point[0] + direction[0] * scale,
            point[1] + direction[1] * scale,
            point[2] + direction[2] * scale,
        ];
        let mut crossings = 0_u64;
        for face in mesh.faces() {
            let _ = mesh.face_triangles_into(face, strategy, buffer);
            for triangle in buffer.iter() {
                let mut corners = [[0.0_f64; 3]; 3];
                let mut live = true;
                for (slot, corner) in corners.iter_mut().zip(triangle) {
                    match mesh
                        .to_vertex(*corner)
                        .and_then(|v| mesh.vertex_position(v))
                    {
                        Some(p) => *slot = promote(*p),
                        None => live = false,
                    }
                }
                if !live {
                    continue;
                }
                // Zero-area triangles (fan output over loops with repeated
                // positions or collinear runs after splitting) contribute
                // nothing to parity; the cross product of f32-promoted
                // coordinates is exact enough for an exact-zero test.
                if triangle_is_flat(corners) {
                    continue;
                }
                match segment_crosses_triangle(point, far, corners) {
                    Crossing::Crosses => crossings += 1,
                    Crossing::Misses => {}
                    Crossing::Degenerate => {
                        *retries += 1;
                        continue 'directions;
                    }
                }
            }
        }
        return Some(crossings % 2 == 1);
    }
    None
}

/// True when the triangle has exactly zero area.
///
/// Corners come from promoted f32 positions, so differences and products
/// are exact in f64 and a true zero cross product compares exactly.
fn triangle_is_flat(t: [[f64; 3]; 3]) -> bool {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    cross == [0.0, 0.0, 0.0]
}

enum Crossing {
    Crosses,
    Misses,
    Degenerate,
}

/// Exact segment-triangle crossing: strict crossings only.
///
/// An endpoint on the triangle's supporting plane is not automatically a
/// degeneracy: split meshes carry many triangles exactly in the cut
/// planes, and patch samples of touching solids lie in contact planes
/// shared with the other mesh — such points would otherwise degenerate
/// every ray direction. When exactly one endpoint is coplanar and lies
/// strictly outside the triangle (exact in-plane test), the segment only
/// touches the plane where the triangle is not — a miss. An endpoint
/// inside or on the triangle, or a segment lying in the plane, is a
/// genuine degeneracy (the caller retries a different ray).
fn segment_crosses_triangle(p: [f64; 3], q: [f64; 3], t: [[f64; 3]; 3]) -> Crossing {
    let sp = orient3d(t[0], t[1], t[2], p);
    let sq = orient3d(t[0], t[1], t[2], q);
    match (sp == Orientation3d::Coplanar, sq == Orientation3d::Coplanar) {
        (true, true) => return Crossing::Degenerate,
        (true, false) => {
            return if point_strictly_outside_triangle(p, t) {
                Crossing::Misses
            } else {
                Crossing::Degenerate
            };
        }
        (false, true) => {
            return if point_strictly_outside_triangle(q, t) {
                Crossing::Misses
            } else {
                Crossing::Degenerate
            };
        }
        (false, false) => {}
    }
    if sp == sq {
        return Crossing::Misses;
    }
    let s1 = orient3d(p, q, t[0], t[1]);
    let s2 = orient3d(p, q, t[1], t[2]);
    let s3 = orient3d(p, q, t[2], t[0]);
    if s1 == Orientation3d::Coplanar
        || s2 == Orientation3d::Coplanar
        || s3 == Orientation3d::Coplanar
    {
        return Crossing::Degenerate;
    }
    if s1 == s2 && s2 == s3 {
        Crossing::Crosses
    } else {
        Crossing::Misses
    }
}

/// Exact test that a point in a (non-flat) triangle's plane lies strictly
/// outside the triangle, via [`orient2d`] signs in the dominant-axis
/// projection: strictly outside means on the opposite side of some edge
/// from the triangle's interior.
fn point_strictly_outside_triangle(p: [f64; 3], t: [[f64; 3]; 3]) -> bool {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let axis = dominant_axis(normal);
    let corners = [
        project_point(t[0], axis),
        project_point(t[1], axis),
        project_point(t[2], axis),
    ];
    let interior = orient2d(corners[0], corners[1], corners[2]);
    if interior == Orientation::Collinear {
        return false; // Flat in projection; the caller filtered flats.
    }
    (0..3).any(|i| {
        let side = orient2d(corners[i], corners[(i + 1) % 3], project_point(p, axis));
        side != Orientation::Collinear && side != interior
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeshBuilder;
    use crate::boolean::{
        BooleanBvh, BooleanScratch, build_intersection_graph, narrow_phase, split_mesh_along_graph,
    };

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

    fn classify_two_cubes() -> (PatchClassification, BooleanDiagnostics) {
        let mut mesh_a = cube([0.0, 0.0, 0.0]);
        let mut mesh_b = cube([0.5, 0.5, 0.5]);
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
        let classification = classify_patches(
            &mesh_a,
            &mesh_b,
            &graph,
            &outcome_a,
            &outcome_b,
            &[],
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        (classification, diagnostics)
    }

    #[test]
    fn two_cube_overlap_classifies_inside_and_outside() {
        let (classification, diagnostics) = classify_two_cubes();
        assert!(diagnostics.is_clean(), "{:?}", diagnostics.entries());
        assert!(!classification.any_suspect());

        for side in [MeshSide::A, MeshSide::B] {
            let patches: Vec<&Patch> = classification.of(side).collect();
            assert_eq!(patches.len(), 2, "{side:?}: one inside, one outside");
            let inside: Vec<&&Patch> = patches
                .iter()
                .filter(|p| p.side == PatchSide::Inside)
                .collect();
            let outside: Vec<&&Patch> = patches
                .iter()
                .filter(|p| p.side == PatchSide::Outside)
                .collect();
            assert_eq!(inside.len(), 1, "{side:?}");
            assert_eq!(outside.len(), 1, "{side:?}");
            // The corner-overlap cut: 3 small corner faces inside, the
            // other 6 (3 split remainders + 3 untouched) outside.
            assert_eq!(inside[0].faces.len(), 3, "{side:?}");
            assert_eq!(outside[0].faces.len(), 6, "{side:?}");
        }
    }

    #[test]
    fn classification_is_deterministic() {
        let (first, _) = classify_two_cubes();
        let (second, _) = classify_two_cubes();
        assert_eq!(first, second);
    }

    #[test]
    fn disjoint_meshes_classify_all_outside() {
        let mesh_a = cube([0.0, 0.0, 0.0]);
        let mesh_b = cube([5.0, 5.0, 5.0]);
        let mut scratch = BooleanScratch::new();
        let strategy = FaceTriangulation::Fan;
        let graph = IntersectionGraph::default();
        let outcome_a = MeshSplitOutcome::default();
        let outcome_b = MeshSplitOutcome::default();
        let mut diagnostics = BooleanDiagnostics::default();
        let classification = classify_patches(
            &mesh_a,
            &mesh_b,
            &graph,
            &outcome_a,
            &outcome_b,
            &[],
            strategy,
            &mut scratch,
            &mut diagnostics,
        );
        assert_eq!(classification.patches.len(), 2, "one patch per mesh");
        assert!(
            classification
                .patches
                .iter()
                .all(|p| p.side == PatchSide::Outside)
        );
    }
}
