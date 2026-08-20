// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::fmt;

use crate::session::find_outgoing_half_edge_linear_scan;
use crate::session::propagation::capture_edge_tags;
use crate::{ChangeSink, EditSession, FaceId, HalfEdgeId, Mesh, VertexId, attr};

/// Structured edge-collapse error from [`crate::op::collapse_edge()`].
///
/// Every variant is a precondition detected before any mutation: a failed
/// collapse leaves the mesh byte-identical.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CollapseEdgeError {
    /// Half-edge must be live and have a live twin.
    HalfEdgeNotLive {
        /// Stale or invalid half-edge index.
        half_edge: u32,
    },
    /// Both endpoints lie on the boundary but the edge is interior;
    /// collapsing would pinch the surface at the merged vertex.
    BoundaryPinch {
        /// Surviving endpoint index.
        keep: u32,
        /// Removed endpoint index.
        remove: u32,
    },
    /// A face incident to the removed endpoint would end up visiting the
    /// merged vertex twice.
    FaceWouldPinch {
        /// The offending face index.
        face: u32,
    },
    /// An undirected edge of the collapsed neighborhood would end up with
    /// more than two incident faces (the link condition fails).
    LinkConditionViolated {
        /// Smaller endpoint index of the offending edge.
        a: u32,
        /// Larger endpoint index of the offending edge.
        b: u32,
    },
    /// The collapse would leave two faces with an identical vertex set
    /// (for example collapsing a tetrahedron edge into a two-face pillow).
    DegenerateShell {
        /// The face whose merged loop duplicates another face.
        face: u32,
    },
}

impl fmt::Display for CollapseEdgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live or has no live twin: {half_edge}")
            }
            Self::BoundaryPinch { keep, remove } => write!(
                f,
                "collapsing interior edge ({keep}, {remove}) between two boundary vertices would pinch the surface"
            ),
            Self::FaceWouldPinch { face } => {
                write!(f, "face {face} would visit the merged vertex twice")
            }
            Self::LinkConditionViolated { a, b } => write!(
                f,
                "edge ({a}, {b}) would exceed two incident faces after the collapse"
            ),
            Self::DegenerateShell { face } => write!(
                f,
                "face {face} would duplicate another face's vertex set after the collapse"
            ),
        }
    }
}

impl core::error::Error for CollapseEdgeError {}

/// One face incident to the collapsing edge.
struct EdgeSide {
    face: FaceId,
    /// The face's half-edge running along the collapsing edge.
    on_edge: HalfEdgeId,
    /// True when the face is a triangle that degenerates and is removed.
    dropped: bool,
}

/// A dropped triangle's side-edge fusion: its two side edges merge into
/// one, re-twinning the surviving outer half-edges.
struct Fusion {
    survivor_a: HalfEdgeId,
    survivor_b: HalfEdgeId,
    wing: VertexId,
    /// True in the normal case; false when the survivors themselves die
    /// (a vanishing patch), in which case no re-twin happens.
    retwin: bool,
    seam: Option<bool>,
    sharpness: Option<f32>,
}

/// A shrinking (degree > 3) face along the collapsing edge.
struct Shrink {
    face: FaceId,
    on_edge: HalfEdgeId,
    previous: HalfEdgeId,
    /// Set when the corner at the survivor is the dying one: its UV
    /// (possibly absent) transfers onto the corner that now points there.
    uv_transfer: Option<(HalfEdgeId, Option<[f32; 2]>)>,
}

/// Collapses one undirected edge in place, merging its endpoints into the
/// smaller-id vertex.
///
/// The surviving vertex keeps its authored position. Faces along the edge
/// lose one loop vertex — triangles degenerate and are removed, fusing
/// their two side edges — and every other half-edge into the removed
/// vertex is re-aimed at the survivor, so surviving faces, corners, and
/// edges keep their identities. Deterministic propagation (see ADR-0009):
///
/// - Surviving corner UVs stay in place; where a shrinking face's corner
///   at the survivor is the one that dies, the survivor's authored UV
///   transfers onto the corner that now points there.
/// - Authored corner-normal overrides are cleared on every corner of
///   every surviving face that was incident to the removed vertex, and on
///   fused side edges (faces not incident to it keep theirs).
/// - Undirected-edge seam/sharpness survive in place; where two side
///   edges fuse into one, seams OR together and sharpness takes the
///   maximum.
/// - Authored vertex sharpness merges by maximum onto the survivor.
/// - `FACE_REGION` stays with every surviving face.
///
/// # Errors
///
/// Every precondition — the link condition (no edge may end up with more
/// than two incident faces), boundary pinches, face pinches, and
/// degenerate shells — is checked before any mutation; on error the mesh
/// is unchanged. See [`CollapseEdgeError`].
pub fn collapse_edge<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    half_edge: HalfEdgeId,
) -> Result<VertexId, CollapseEdgeError> {
    let not_live = CollapseEdgeError::HalfEdgeNotLive {
        half_edge: half_edge.index(),
    };

    let mesh = session.mesh();
    let twin = mesh.twin(half_edge).ok_or(not_live)?;
    let from = mesh.from_vertex(half_edge).ok_or(not_live)?;
    let to = mesh.to_vertex(half_edge).ok_or(not_live)?;
    let h_face = mesh.face(half_edge).ok_or(not_live)?;
    let t_face = mesh.face(twin).ok_or(not_live)?;

    // Deterministic survivor: the smaller vertex id wins.
    let (keep, remove) = if from.index() <= to.index() {
        (from, to)
    } else {
        (to, from)
    };

    let edge_is_boundary = h_face == FaceId::OUTSIDE || t_face == FaceId::OUTSIDE;
    if !edge_is_boundary
        && vertex_on_boundary(mesh, keep).ok_or(not_live)?
        && vertex_on_boundary(mesh, remove).ok_or(not_live)?
    {
        return Err(CollapseEdgeError::BoundaryPinch {
            keep: keep.index(),
            remove: remove.index(),
        });
    }

    // Interior faces around the removed vertex, in rotation order starting
    // at the collapsing edge (deterministic given the mesh state).
    let start = if from == remove { half_edge } else { twin };
    let ring = outgoing_ring(mesh, start).ok_or(not_live)?;
    let mut incident = BTreeSet::<FaceId>::new();
    let mut merged_loops = Vec::<(FaceId, Vec<VertexId>)>::new();
    for &outgoing in &ring {
        let face = mesh.face(outgoing).ok_or(not_live)?;
        if face == FaceId::OUTSIDE {
            continue;
        }
        incident.insert(face);
        if let Some(merged) = merged_loop(mesh, face, keep, remove)? {
            merged_loops.push((face, merged));
        }
    }

    // Link condition, expressed as final edge multiplicity: every edge of
    // the merged neighborhood must end with at most two incident faces.
    let edge_sides = collect_edge_sides(mesh);
    let mut planned_counts = BTreeMap::<(VertexId, VertexId), u32>::new();
    for (_, merged) in &merged_loops {
        for (a, b) in loop_edges(merged) {
            *planned_counts.entry(ordered(a, b)).or_insert(0) += 1;
        }
    }
    for (&(a, b), &planned) in &planned_counts {
        let untouched = edge_sides.get(&(a, b)).map_or(0, |sides| {
            sides
                .faces
                .iter()
                .filter(|face| **face != FaceId::OUTSIDE && !incident.contains(face))
                .count()
        });
        let untouched = u32::try_from(untouched).unwrap_or(u32::MAX);
        if planned + untouched > 2 {
            return Err(CollapseEdgeError::LinkConditionViolated {
                a: a.index().min(b.index()),
                b: a.index().max(b.index()),
            });
        }
    }

    // Degenerate-shell guard: no merged face may duplicate the vertex set
    // of another merged face or of an untouched face around the survivor.
    let mut shells = BTreeSet::<Vec<VertexId>>::new();
    if let Some(keep_ring) = vertex_outgoing_ring(mesh, keep) {
        for outgoing in keep_ring {
            let face = mesh.face(outgoing).ok_or(not_live)?;
            if face == FaceId::OUTSIDE || incident.contains(&face) {
                continue;
            }
            let mut vertices: Vec<VertexId> = mesh
                .face_loop(face)
                .filter_map(|corner| mesh.to_vertex(corner))
                .collect();
            vertices.sort_unstable();
            shells.insert(vertices);
        }
    }
    for (face, merged) in &merged_loops {
        let mut vertices = merged.clone();
        vertices.sort_unstable();
        if !shells.insert(vertices) {
            return Err(CollapseEdgeError::DegenerateShell { face: face.index() });
        }
    }

    // --- Surgery plan (still read-only). ---
    let mut sides = Vec::<EdgeSide>::new();
    for (on_edge, face) in [(half_edge, h_face), (twin, t_face)] {
        if face == FaceId::OUTSIDE {
            continue;
        }
        let degree = mesh
            .faces
            .get(face.as_id())
            .map(|record| record.degree)
            .ok_or(not_live)?;
        sides.push(EdgeSide {
            face,
            on_edge,
            dropped: degree == 3,
        });
    }

    let mut dying = BTreeSet::<HalfEdgeId>::new();
    dying.insert(half_edge);
    dying.insert(twin);
    let mut fusions = Vec::<Fusion>::new();
    for side in sides.iter().filter(|side| side.dropped) {
        let side_a = mesh.next(side.on_edge).ok_or(not_live)?;
        let side_b = mesh.next(side_a).ok_or(not_live)?;
        let survivor_a = mesh.twin(side_a).ok_or(not_live)?;
        let survivor_b = mesh.twin(side_b).ok_or(not_live)?;
        let wing = mesh.to_vertex(side_a).ok_or(not_live)?;
        let canonical_a = mesh.canonical_edge(side_a).ok_or(not_live)?;
        let canonical_b = mesh.canonical_edge(side_b).ok_or(not_live)?;
        let (seam_a, sharp_a) = capture_edge_tags(mesh, canonical_a);
        let (seam_b, sharp_b) = capture_edge_tags(mesh, canonical_b);
        dying.insert(side_a);
        dying.insert(side_b);
        fusions.push(Fusion {
            survivor_a,
            survivor_b,
            wing,
            retwin: true,
            seam: merge_seam(seam_a, seam_b),
            sharpness: merge_sharpness(sharp_a, sharp_b),
        });
    }
    // Fusions whose survivors are dying (a two-triangle pillow collapsing
    // to nothing) or are both boundary sides (the local patch vanishes)
    // do not re-twin; such survivors die with the patch.
    let mut extra_dying = Vec::<HalfEdgeId>::new();
    for fusion in &mut fusions {
        let survivor_dies =
            dying.contains(&fusion.survivor_a) || dying.contains(&fusion.survivor_b);
        let both_outside = mesh.face(fusion.survivor_a) == Some(FaceId::OUTSIDE)
            && mesh.face(fusion.survivor_b) == Some(FaceId::OUTSIDE);
        if survivor_dies {
            fusion.retwin = false;
        } else if both_outside {
            fusion.retwin = false;
            extra_dying.push(fusion.survivor_a);
            extra_dying.push(fusion.survivor_b);
        }
    }
    for extra in extra_dying {
        dying.insert(extra);
    }

    let mut shrinks = Vec::<Shrink>::new();
    for side in sides.iter().filter(|side| !side.dropped) {
        let previous = previous_in_face(mesh, side.on_edge).ok_or(not_live)?;
        let uv_transfer = (mesh.to_vertex(side.on_edge) == Some(keep)).then(|| {
            let uv = mesh
                .attrs()
                .sparse(attr::CORNER_UV)
                .and_then(|layer| layer.get(side.on_edge.as_id()).copied());
            (previous, uv)
        });
        shrinks.push(Shrink {
            face: side.face,
            on_edge: side.on_edge,
            previous,
            uv_transfer,
        });
    }

    // Every half-edge aimed at the removed vertex re-aims at the survivor.
    let relabeled: Vec<HalfEdgeId> = mesh
        .half_edges
        .iter()
        .filter(|(_, record)| record.to == remove)
        .map(|(id, _)| HalfEdgeId::from(id))
        .collect();
    let wings: Vec<VertexId> = fusions.iter().map(|fusion| fusion.wing).collect();
    let keep_sharpness = mesh.vertex_sharpness(keep);
    let remove_sharpness = mesh.vertex_sharpness(remove);

    // --- Mutation. ---
    for &corner in &relabeled {
        session
            .mesh_mut()
            .half_edges
            .get_mut(corner.as_id())
            .expect("collected live half-edge")
            .to = keep;
        session.mark_corner_dirty(corner);
    }

    for shrink in &shrinks {
        let next = session
            .mesh()
            .next(shrink.on_edge)
            .expect("validated shrinking face");
        session
            .mesh_mut()
            .half_edges
            .get_mut(shrink.previous.as_id())
            .expect("validated shrinking face")
            .next = next;
        let record = session
            .mesh_mut()
            .faces
            .get_mut(shrink.face.as_id())
            .expect("validated shrinking face");
        record.degree -= 1;
        if record.edge == shrink.on_edge {
            record.edge = next;
        }
        session.mark_face_dirty(shrink.face);
    }

    for fusion in fusions.iter().filter(|fusion| fusion.retwin) {
        session
            .mesh_mut()
            .half_edges
            .get_mut(fusion.survivor_a.as_id())
            .expect("fusion survivor must be live")
            .twin = fusion.survivor_b;
        session
            .mesh_mut()
            .half_edges
            .get_mut(fusion.survivor_b.as_id())
            .expect("fusion survivor must be live")
            .twin = fusion.survivor_a;
        // Old per-side entries die; the merged value keys the fused edge's
        // canonical id.
        for survivor in [fusion.survivor_a, fusion.survivor_b] {
            if let Some(layer) = session.mesh_mut().attrs_mut().sparse_mut(attr::EDGE_SEAM) {
                let _ = layer.remove(survivor.as_id());
            }
            if let Some(layer) = session
                .mesh_mut()
                .attrs_mut()
                .sparse_mut(attr::EDGE_SHARPNESS)
            {
                let _ = layer.remove(survivor.as_id());
            }
        }
        if let Some(seam) = fusion.seam {
            let _ = session.mesh_mut().set_edge_seam(fusion.survivor_a, seam);
        }
        if let Some(sharpness) = fusion.sharpness {
            let _ = session
                .mesh_mut()
                .set_edge_sharpness(fusion.survivor_a, sharpness);
        }
        session.mark_corner_dirty(fusion.survivor_a);
        session.mark_corner_dirty(fusion.survivor_b);
    }

    // Splice outside loops across the dying half-edges.
    let mut splices = Vec::<(HalfEdgeId, HalfEdgeId)>::new();
    let limit = session.mesh().half_edges.slot_count().max(1);
    for (id, record) in session.mesh().half_edges.iter() {
        let outside = HalfEdgeId::from(id);
        if record.face != FaceId::OUTSIDE || dying.contains(&outside) {
            continue;
        }
        if !dying.contains(&record.next) {
            continue;
        }
        let mut cursor = record.next;
        let mut steps = 0_usize;
        while dying.contains(&cursor) {
            cursor = session
                .mesh()
                .next(cursor)
                .expect("outside loops are closed");
            steps += 1;
            assert!(steps <= limit, "outside splice must terminate");
        }
        splices.push((outside, cursor));
    }
    for (outside, next) in splices {
        session
            .mesh_mut()
            .half_edges
            .get_mut(outside.as_id())
            .expect("live outside half-edge")
            .next = next;
    }

    // Remove the dropped faces, the dying half-edges, and the vertex.
    for side in sides.iter().filter(|side| side.dropped) {
        let removed = session.mesh_mut().faces.remove(side.face.as_id());
        debug_assert!(removed.is_some(), "validated dropped face should remove");
        session.record_deleted_face(side.face);
    }
    for &dead in &dying {
        let removed = session.mesh_mut().half_edges.remove(dead.as_id());
        debug_assert!(removed.is_some(), "validated dying half-edge should remove");
        session.record_deleted_half_edge(dead);
        crate::session::clear_deleted_corner_attrs(session.mesh_mut(), dead);
    }
    let removed = session.mesh_mut().vertices.remove(remove.as_id());
    debug_assert!(removed.is_some(), "validated vertex should remove");
    session.record_deleted_vertex(remove);

    // Survivor-UV transfers for shrunk faces.
    for shrink in &shrinks {
        let Some((previous, uv)) = shrink.uv_transfer else {
            continue;
        };
        if let Some(layer) = session.mesh_mut().attrs_mut().sparse_mut(attr::CORNER_UV) {
            match uv {
                Some(uv) => layer.set(previous.as_id(), uv),
                None => {
                    let _ = layer.remove(previous.as_id());
                }
            }
        }
        session.mark_corner_dirty(previous);
    }

    // Authored normal overrides clear on every affected corner: all
    // corners of the surviving faces that were incident to the removed
    // vertex, plus re-aimed corners and fused side edges.
    let mut cleared = Vec::<HalfEdgeId>::new();
    cleared.extend(relabeled.iter().copied());
    for &face in &incident {
        if session.mesh().faces.get(face.as_id()).is_none() {
            continue;
        }
        cleared.extend(session.mesh().face_loop(face));
    }
    for fusion in fusions.iter().filter(|fusion| fusion.retwin) {
        cleared.push(fusion.survivor_a);
        cleared.push(fusion.survivor_b);
    }
    for corner in cleared {
        if session.mesh().half_edges.get(corner.as_id()).is_none() {
            continue;
        }
        if let Some(layer) = session
            .mesh_mut()
            .attrs_mut()
            .sparse_mut(attr::CORNER_NORMAL_OVERRIDE)
        {
            let _ = layer.remove(corner.as_id());
        }
        let face = session.mesh().face(corner);
        if let Some(face) = face.filter(|&face| face != FaceId::OUTSIDE) {
            session.mark_face_dirty(face);
        }
        session.mark_corner_dirty(corner);
    }

    // Repair stored outgoing references at the survivor and the wings.
    let mut repair = Vec::<VertexId>::with_capacity(1 + wings.len());
    repair.push(keep);
    repair.extend(wings.iter().copied());
    repair.sort_unstable();
    repair.dedup();
    for vertex in repair {
        let Some(stored) = session
            .mesh()
            .vertices
            .get(vertex.as_id())
            .map(|record| record.out)
        else {
            continue;
        };
        let valid =
            stored != HalfEdgeId::INVALID && session.mesh().from_vertex(stored) == Some(vertex);
        if valid {
            continue;
        }
        let found = find_outgoing_half_edge_linear_scan(session.mesh(), vertex);
        session
            .mesh_mut()
            .vertices
            .get_mut(vertex.as_id())
            .expect("repaired vertex must be live")
            .out = found.unwrap_or(HalfEdgeId::INVALID);
        session.mark_vertex_dirty(vertex);
    }

    // Authored vertex sharpness merges by maximum onto the survivor.
    if let Some(removed_sharpness) = remove_sharpness {
        let merged = keep_sharpness.map_or(removed_sharpness, |kept| kept.max(removed_sharpness));
        let _ = session.mesh_mut().set_vertex_sharpness(keep, merged);
    }

    let vertex_slots = session.mesh().vertices.slot_count();
    let face_slots = session.mesh().faces.slot_count();
    let half_edge_slots = session.mesh().half_edges.slot_count();
    session
        .mesh_mut()
        .attrs
        .sync_capacities(vertex_slots, face_slots, half_edge_slots);
    session.mark_vertex_dirty(keep);
    session.invalidate_outgoing_index();
    Ok(keep)
}

/// The substituted loop of one face incident to the removed vertex:
/// `Ok(None)` when the face degenerates (it is removed), an error when the
/// substitution would pinch the face.
fn merged_loop(
    mesh: &Mesh,
    face: FaceId,
    keep: VertexId,
    remove: VertexId,
) -> Result<Option<Vec<VertexId>>, CollapseEdgeError> {
    let mut loop_vertices = Vec::<VertexId>::new();
    for corner in mesh.face_loop(face) {
        let destination = mesh
            .to_vertex(corner)
            .ok_or(CollapseEdgeError::HalfEdgeNotLive {
                half_edge: corner.index(),
            })?;
        loop_vertices.push(if destination == remove {
            keep
        } else {
            destination
        });
    }

    // Collapse consecutive duplicates cyclically (the collapsing edge's
    // two endpoints become one), then reject any remaining repetition.
    let mut deduped = Vec::<VertexId>::with_capacity(loop_vertices.len());
    for &vertex in &loop_vertices {
        if deduped.last() != Some(&vertex) {
            deduped.push(vertex);
        }
    }
    while deduped.len() > 1 && deduped.first() == deduped.last() {
        deduped.pop();
    }
    let mut unique = BTreeSet::<VertexId>::new();
    for &vertex in &deduped {
        if !unique.insert(vertex) {
            return Err(CollapseEdgeError::FaceWouldPinch { face: face.index() });
        }
    }
    if deduped.len() < 3 {
        return Ok(None);
    }
    Ok(Some(deduped))
}

/// The two face sides of one undirected edge.
struct EdgeSides {
    faces: [FaceId; 2],
}

/// Collects every undirected edge's face pair in one deterministic pass.
fn collect_edge_sides(mesh: &Mesh) -> BTreeMap<(VertexId, VertexId), EdgeSides> {
    let mut sides = BTreeMap::<(VertexId, VertexId), EdgeSides>::new();
    for (id, record) in mesh.half_edges.iter() {
        let half_edge = HalfEdgeId::from(id);
        let (Some(from), Some(to)) = (mesh.from_vertex(half_edge), mesh.to_vertex(half_edge))
        else {
            continue;
        };
        if from.index() > to.index() {
            continue; // Each undirected edge is visited from its canonical side.
        }
        let twin_face = mesh
            .twin(half_edge)
            .and_then(|twin| mesh.face(twin))
            .unwrap_or(FaceId::OUTSIDE);
        sides.insert(
            ordered(from, to),
            EdgeSides {
                faces: [record.face, twin_face],
            },
        );
    }
    sides
}

/// The half-edge preceding `target` in its face loop.
fn previous_in_face(mesh: &Mesh, target: HalfEdgeId) -> Option<HalfEdgeId> {
    let mut cursor = target;
    let limit = mesh.half_edges.slot_count().max(1);
    for _ in 0..=limit {
        let next = mesh.next(cursor)?;
        if next == target {
            return Some(cursor);
        }
        cursor = next;
    }
    None
}

/// Cyclic loop edges of a merged face loop.
fn loop_edges(loop_vertices: &[VertexId]) -> impl Iterator<Item = (VertexId, VertexId)> + '_ {
    (0..loop_vertices.len()).map(|index| {
        (
            loop_vertices[index],
            loop_vertices[(index + 1) % loop_vertices.len()],
        )
    })
}

/// Canonical undirected key ordered by vertex id.
fn ordered(a: VertexId, b: VertexId) -> (VertexId, VertexId) {
    if a.index() <= b.index() {
        (a, b)
    } else {
        (b, a)
    }
}

fn merge_seam(current: Option<bool>, incoming: Option<bool>) -> Option<bool> {
    match (current, incoming) {
        (Some(a), Some(b)) => Some(a || b),
        (value, None) | (None, value) => value,
    }
}

fn merge_sharpness(current: Option<f32>, incoming: Option<f32>) -> Option<f32> {
    match (current, incoming) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (value, None) | (None, value) => value,
    }
}

/// All outgoing half-edges around the origin of `start`, in rotation
/// order. Returns `None` when the star does not close (invalid topology).
fn outgoing_ring(mesh: &Mesh, start: HalfEdgeId) -> Option<Vec<HalfEdgeId>> {
    let mut ring = Vec::<HalfEdgeId>::new();
    let mut cursor = start;
    let limit = mesh.half_edges.slot_count().max(1);
    for _ in 0..=limit {
        ring.push(cursor);
        cursor = mesh.next(mesh.twin(cursor)?)?;
        if cursor == start {
            return Some(ring);
        }
    }
    None
}

/// Outgoing ring for a vertex via its stored outgoing reference.
fn vertex_outgoing_ring(mesh: &Mesh, vertex: VertexId) -> Option<Vec<HalfEdgeId>> {
    let out = mesh.vertices.get(vertex.as_id())?.out;
    mesh.half_edges.get(out.as_id())?;
    outgoing_ring(mesh, out)
}

/// True when the vertex has a boundary (OUTSIDE-faced) outgoing half-edge.
fn vertex_on_boundary(mesh: &Mesh, vertex: VertexId) -> Option<bool> {
    let Some(ring) = vertex_outgoing_ring(mesh, vertex) else {
        return Some(false); // Isolated vertices have no boundary fan.
    };
    let mut boundary = false;
    for outgoing in ring {
        if mesh.face(outgoing)? == FaceId::OUTSIDE {
            boundary = true;
        }
    }
    Some(boundary)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use crate::{BuildParams, Mesh, VertexId, attr, op};

    use super::CollapseEdgeError;

    fn two_triangles() -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("mesh should build")
    }

    fn find_half_edge(mesh: &Mesh, from: u32, to: u32) -> crate::HalfEdgeId {
        mesh.faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                mesh.from_vertex(half_edge).map(VertexId::index) == Some(from)
                    && mesh.to_vertex(half_edge).map(VertexId::index) == Some(to)
            })
            .expect("half-edge should exist")
    }

    fn face_vertex_sets(mesh: &Mesh) -> Vec<Vec<u32>> {
        let mut sets: Vec<Vec<u32>> = mesh
            .faces()
            .map(|face| {
                let mut vertices: Vec<u32> = mesh
                    .face_loop(face)
                    .filter_map(|corner| mesh.to_vertex(corner))
                    .map(|vertex| vertex.index())
                    .collect();
                vertices.sort_unstable();
                vertices
            })
            .collect();
        sets.sort();
        sets
    }

    fn snapshot(mesh: &Mesh) -> (Vec<Vec<u32>>, usize, usize) {
        (
            face_vertex_sets(mesh),
            mesh.vertices().count(),
            mesh.half_edges.iter().count(),
        )
    }

    /// A disk fan: rim vertices `0..rim` around center vertex `rim`.
    /// Positions are topology-only test scaffolding.
    fn fan(rim: u32) -> Mesh {
        let mut builder = crate::MeshBuilder::new();
        for index in 0..rim {
            #[expect(clippy::cast_precision_loss, reason = "small test indices")]
            builder.push_vertex([index as f32, (index * index) as f32, 1.0]);
        }
        builder.push_vertex([0.0, 0.0, 0.0]);
        for index in 0..rim {
            builder
                .add_face(&[index, (index + 1) % rim, rim])
                .expect("fan face");
        }
        builder.build().expect("fan should build").mesh
    }

    fn vertex_by_index(mesh: &Mesh, index: u32) -> VertexId {
        mesh.vertices()
            .find(|vertex| vertex.index() == index)
            .expect("vertex should be live")
    }

    fn octahedron() -> Mesh {
        Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0, 0.0, -1.0],
            ],
            &[
                [0, 1, 2],
                [0, 2, 3],
                [0, 3, 4],
                [0, 4, 1],
                [5, 2, 1],
                [5, 3, 2],
                [5, 4, 3],
                [5, 1, 4],
            ],
            &BuildParams::default(),
        )
        .expect("octahedron should build")
    }

    #[test]
    fn collapse_boundary_edge_merges_into_smaller_id() {
        let mut mesh = two_triangles();
        let edge = find_half_edge(&mesh, 0, 1);
        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, edge).expect("collapse should succeed");
        let _: () = session.finish();

        assert_eq!(survivor.index(), 0);
        assert!(
            mesh.validate_deep().is_empty(),
            "{:?}",
            mesh.validate_deep()
        );
        assert_eq!(mesh.vertices().count(), 3);
        assert_eq!(face_vertex_sets(&mesh), [[0, 2, 3]]);
        // The survivor keeps its authored position.
        assert_eq!(
            mesh.vertex_position(survivor),
            Some(&[0.0_f32, 0.0_f32, 0.0_f32])
        );
    }

    #[test]
    fn collapse_rejects_interior_edge_between_boundary_vertices() {
        let mut mesh = two_triangles();
        let edge = find_half_edge(&mesh, 1, 2);
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::collapse_edge(&mut session, edge);
        let _: () = session.finish();
        assert!(matches!(
            result,
            Err(CollapseEdgeError::BoundaryPinch { .. })
        ));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn collapse_spoke_of_fan_rebuilds_around_rim_vertex() {
        let mut mesh = fan(6);
        let edge = find_half_edge(&mesh, 0, 6);
        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, edge).expect("collapse should succeed");
        let _: () = session.finish();

        assert_eq!(survivor.index(), 0);
        assert!(
            mesh.validate_deep().is_empty(),
            "{:?}",
            mesh.validate_deep()
        );
        assert_eq!(mesh.vertices().count(), 6);
        assert_eq!(mesh.faces().count(), 4);
        assert_eq!(
            face_vertex_sets(&mesh),
            [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]]
        );
    }

    #[test]
    fn collapse_octahedron_edge_preserves_closed_topology() {
        let mut mesh = octahedron();
        let edge = find_half_edge(&mesh, 1, 2);
        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, edge).expect("collapse should succeed");
        let _: () = session.finish();

        assert_eq!(survivor.index(), 1);
        assert!(
            mesh.validate_deep().is_empty(),
            "{:?}",
            mesh.validate_deep()
        );
        let vertices = mesh.vertices().count();
        let faces = mesh.faces().count();
        let edges = mesh.half_edges.iter().count() / 2;
        assert_eq!((vertices, edges, faces), (5, 9, 6));
        // Closed surface: Euler characteristic stays 2.
        assert_eq!(vertices + faces, edges + 2);
    }

    #[test]
    fn collapse_tetrahedron_rejects_degenerate_shell() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            &[[0, 2, 1], [0, 1, 3], [1, 2, 3], [0, 3, 2]],
            &BuildParams::default(),
        )
        .expect("tetrahedron should build");
        let edge = find_half_edge(&mesh, 0, 1);
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::collapse_edge(&mut session, edge);
        let _: () = session.finish();
        assert!(matches!(
            result,
            Err(CollapseEdgeError::DegenerateShell { .. })
        ));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn collapse_rejects_link_condition_violation() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.5, 1.0, 0.0],
                [0.5, 2.0, 0.0],
                [-0.5, 2.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3], [0, 2, 3], [0, 3, 4]],
            &BuildParams::default(),
        )
        .expect("mesh should build");
        // Vertex 3 is adjacent to both endpoints through faces that do not
        // contain the collapsing edge (0, 1): edge (0, 3) has two interior
        // faces and edge (1, 3) one, so the fused edge would carry three.
        let edge = find_half_edge(&mesh, 0, 1);
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::collapse_edge(&mut session, edge);
        let _: () = session.finish();
        assert!(matches!(
            result,
            Err(CollapseEdgeError::LinkConditionViolated { .. })
        ));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn collapse_merges_edge_and_vertex_attributes() {
        let mut mesh = fan(6);
        let spoke = find_half_edge(&mesh, 1, 6);
        let rim = find_half_edge(&mesh, 0, 1);
        let region_face = mesh.face(find_half_edge(&mesh, 1, 2)).expect("face");
        let center = vertex_by_index(&mesh, 6);
        let survivor_vertex = vertex_by_index(&mesh, 0);

        let mut session = mesh.edit();
        op::set_edge_sharpness(&mut session, rim, 2.0).expect("rim sharpness");
        op::set_edge_sharpness(&mut session, spoke, 3.0).expect("spoke sharpness");
        op::set_edge_seam(&mut session, rim, true).expect("rim seam");
        op::set_face_region(&mut session, region_face, 7).expect("region");
        op::set_vertex_sharpness(&mut session, center, 5.0).expect("center sharpness");
        op::set_vertex_sharpness(&mut session, survivor_vertex, 1.0).expect("rim sharpness");
        let _: () = session.finish();

        let edge = find_half_edge(&mesh, 0, 6);
        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, edge).expect("collapse should succeed");
        let _: () = session.finish();

        assert!(mesh.validate_deep().is_empty());
        // The rim edge (0, 1) fused with the spoke (1, 6): seams OR, and
        // sharpness takes the maximum.
        let fused = find_half_edge(&mesh, 0, 1);
        assert_eq!(mesh.edge_sharpness(fused), Some(3.0));
        assert_eq!(mesh.edge_seam(fused), Some(true));
        // The rebuilt face keeps its region.
        let region_layer = mesh.attrs().dense(attr::FACE_REGION).expect("regions");
        let regioned = mesh
            .faces()
            .filter(|face| region_layer.get(face.as_id()) == Some(&7))
            .count();
        assert_eq!(regioned, 1);
        // Vertex sharpness merges by maximum onto the survivor.
        assert_eq!(mesh.vertex_sharpness(survivor), Some(5.0));
    }

    #[test]
    fn collapse_prefers_surviving_corner_uv() {
        // A single quad whose collapsing boundary edge carries corner UVs
        // at both endpoints: the survivor's UV wins.
        let mut builder = crate::MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
        ] {
            builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2, 3]).expect("quad");
        let mut mesh = builder.build().expect("mesh should build").mesh;

        let into_survivor = find_half_edge(&mesh, 3, 0);
        let into_removed = find_half_edge(&mesh, 0, 1);
        let mut session = mesh.edit();
        op::set_corner_uv(&mut session, into_survivor, [0.1, 0.1]).expect("survivor uv");
        op::set_corner_uv(&mut session, into_removed, [0.9, 0.9]).expect("removed uv");
        let _: () = session.finish();

        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, into_removed).expect("collapse");
        let _: () = session.finish();

        assert!(mesh.validate_deep().is_empty());
        assert_eq!(face_vertex_sets(&mesh), [[0, 2, 3]]);
        let corner = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&corner| mesh.to_vertex(corner) == Some(survivor))
            .expect("corner into survivor");
        assert_eq!(
            mesh.attrs()
                .sparse(attr::CORNER_UV)
                .and_then(|layer| layer.get(corner.as_id())),
            Some(&[0.1_f32, 0.1_f32])
        );
    }

    #[test]
    fn collapse_clears_normal_overrides_on_rebuilt_corners() {
        let mut mesh = fan(6);
        let rebuilt_corner = find_half_edge(&mesh, 1, 2);
        let mut session = mesh.edit();
        op::set_corner_normal_override(&mut session, rebuilt_corner, Some([0.0, 0.0, 1.0]))
            .expect("override");
        let _: () = session.finish();

        let edge = find_half_edge(&mesh, 0, 6);
        let mut session = mesh.edit();
        op::collapse_edge(&mut session, edge).expect("collapse");
        let _: () = session.finish();

        let overrides = mesh
            .attrs()
            .sparse(attr::CORNER_NORMAL_OVERRIDE)
            .map(|layer| {
                mesh.faces()
                    .flat_map(|face| mesh.face_loop(face))
                    .filter(|corner| layer.get(corner.as_id()).is_some())
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(overrides, 0, "rebuilt corners must not keep overrides");
    }

    #[test]
    fn collapse_lone_triangle_edge_drops_the_face() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle should build");
        let edge = find_half_edge(&mesh, 0, 1);
        let mut session = mesh.edit();
        let survivor = op::collapse_edge(&mut session, edge).expect("collapse");
        let _: () = session.finish();

        assert_eq!(survivor.index(), 0);
        assert_eq!(mesh.faces().count(), 0);
        assert_eq!(mesh.vertices().count(), 2);
        assert!(mesh.validate_deep().is_empty());
    }
}
