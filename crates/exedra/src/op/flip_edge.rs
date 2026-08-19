// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use crate::{ChangeSink, EditSession, FaceId, HalfEdgeId, attr};

/// Structured edge-flip error from [`crate::op::flip_edge`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FlipEdgeError {
    /// Half-edge must be live and have a live twin.
    HalfEdgeNotLive {
        /// Stale or invalid half-edge index.
        half_edge: u32,
    },
    /// Only interior edges (a live face on both sides) can flip.
    BoundaryEdge {
        /// The boundary half-edge index.
        half_edge: u32,
    },
    /// Both incident faces must be triangles.
    NonTriangleFace {
        /// The offending face index.
        face: u32,
        /// Its actual degree.
        degree: u32,
    },
    /// The two opposite vertices coincide (a two-triangle pillow); the
    /// flipped diagonal would be a zero-length edge.
    DegenerateOpposite {
        /// The shared opposite vertex index.
        vertex: u32,
    },
    /// The flipped diagonal already exists as a mesh edge; flipping would
    /// make it non-manifold.
    DiagonalExists {
        /// Smaller endpoint index of the would-be diagonal.
        a: u32,
        /// Larger endpoint index of the would-be diagonal.
        b: u32,
    },
}

impl fmt::Display for FlipEdgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HalfEdgeNotLive { half_edge } => {
                write!(f, "half-edge is not live or has no live twin: {half_edge}")
            }
            Self::BoundaryEdge { half_edge } => {
                write!(f, "boundary edges cannot flip: {half_edge}")
            }
            Self::NonTriangleFace { face, degree } => {
                write!(
                    f,
                    "flip requires triangle faces: face {face} has degree {degree}"
                )
            }
            Self::DegenerateOpposite { vertex } => {
                write!(
                    f,
                    "opposite vertices coincide at {vertex}; flip would degenerate"
                )
            }
            Self::DiagonalExists { a, b } => {
                write!(f, "flipped diagonal already exists: ({a}, {b})")
            }
        }
    }
}

impl core::error::Error for FlipEdgeError {}

/// Flips the diagonal of the two-triangle region around one interior edge.
///
/// For an interior edge `(a, b)` between triangles `(a, b, c)` and
/// `(b, a, d)`, the flip rewires the region in place into triangles
/// `(c, d, b)` and `(d, c, a)` sharing the new diagonal `(c, d)`. No
/// entities are created or deleted: the edge's two half-edges are re-aimed
/// as the new diagonal, so their canonical edge identity carries over and
/// the perimeter half-edges keep their identities (and therefore their
/// authored edge attributes) untouched.
///
/// Deterministic propagation (see ADR-0009):
/// - The face that keeps the source triangle's `next(half_edge)` perimeter
///   edge keeps that triangle's face record — and with it `FACE_REGION`.
/// - The old diagonal's authored seam/sharpness are cleared; the new
///   diagonal starts smooth.
/// - The two re-aimed diagonal corners derive their UV from the sole
///   source corner at their new destination vertex (clear when missing).
/// - Authored corner-normal overrides are cleared on all six corners.
///
/// Returns the half-edge of the new diagonal running `c -> d` (it reuses
/// `half_edge`'s identity).
///
/// # Errors
///
/// All preconditions are checked before any mutation; on error the mesh is
/// unchanged. See [`FlipEdgeError`].
pub fn flip_edge<S: ChangeSink>(
    session: &mut EditSession<'_, S>,
    half_edge: HalfEdgeId,
) -> Result<HalfEdgeId, FlipEdgeError> {
    let not_live = FlipEdgeError::HalfEdgeNotLive {
        half_edge: half_edge.index(),
    };

    let mesh = session.mesh();
    let twin = mesh.twin(half_edge).ok_or(not_live)?;
    let h_face = mesh.face(half_edge).ok_or(not_live)?;
    let t_face = mesh.face(twin).ok_or(not_live)?;
    if h_face == FaceId::OUTSIDE {
        return Err(FlipEdgeError::BoundaryEdge {
            half_edge: half_edge.index(),
        });
    }
    if t_face == FaceId::OUTSIDE {
        return Err(FlipEdgeError::BoundaryEdge {
            half_edge: twin.index(),
        });
    }
    for face in [h_face, t_face] {
        let degree = mesh
            .faces
            .get(face.as_id())
            .map(|record| record.degree)
            .ok_or(not_live)?;
        if degree != 3 {
            return Err(FlipEdgeError::NonTriangleFace {
                face: face.index(),
                degree,
            });
        }
    }

    // Triangle corners: h runs a -> b in h_face; twin runs b -> a in t_face.
    let h_next = mesh.next(half_edge).ok_or(not_live)?;
    let h_prev = mesh.next(h_next).ok_or(not_live)?;
    let t_next = mesh.next(twin).ok_or(not_live)?;
    let t_prev = mesh.next(t_next).ok_or(not_live)?;
    let a = mesh.from_vertex(half_edge).ok_or(not_live)?;
    let b = mesh.to_vertex(half_edge).ok_or(not_live)?;
    let c = mesh.to_vertex(h_next).ok_or(not_live)?;
    let d = mesh.to_vertex(t_next).ok_or(not_live)?;

    if c == d {
        return Err(FlipEdgeError::DegenerateOpposite { vertex: c.index() });
    }
    if session.has_undirected_edge(c, d) {
        return Err(FlipEdgeError::DiagonalExists {
            a: u32::min(c.index(), d.index()),
            b: u32::max(c.index(), d.index()),
        });
    }

    // Captures before mutation: the diagonal corners re-aim to d and c and
    // derive their UV from the sole source corner at that vertex.
    let uv_at_d = session.corner_uv(t_next);
    let uv_at_c = session.corner_uv(h_next);

    // Clear the old diagonal's authored edge attributes: the new diagonal
    // reuses the same canonical half-edge pair and starts smooth.
    let canonical = core::cmp::min(half_edge, twin);
    crate::session::propagation::clear_edge_tags(session.mesh_mut(), canonical);

    // Rewire in place:
    //   h_face becomes (c -> d -> b -> c): half_edge, t_prev, h_next.
    //   t_face becomes (d -> c -> a -> d): twin, h_prev, t_next.
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(half_edge.as_id())
            .expect("validated half-edge");
        record.to = d;
        record.next = t_prev;
    }
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(t_prev.as_id())
            .expect("validated triangle corner");
        record.face = h_face;
        record.next = h_next;
    }
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(h_next.as_id())
            .expect("validated triangle corner");
        record.next = half_edge;
    }
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(twin.as_id())
            .expect("validated twin");
        record.to = c;
        record.next = h_prev;
    }
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(h_prev.as_id())
            .expect("validated triangle corner");
        record.face = t_face;
        record.next = t_next;
    }
    {
        let record = session
            .mesh_mut()
            .half_edges
            .get_mut(t_next.as_id())
            .expect("validated triangle corner");
        record.next = twin;
    }
    session
        .mesh_mut()
        .faces
        .get_mut(h_face.as_id())
        .expect("validated face")
        .edge = half_edge;
    session
        .mesh_mut()
        .faces
        .get_mut(t_face.as_id())
        .expect("validated face")
        .edge = twin;

    // The diagonal no longer leaves a or b; repoint their outgoing
    // references at surviving outgoing half-edges when they referenced it.
    if session.mesh().vertices.get(a.as_id()).map(|v| v.out) == Some(half_edge) {
        session
            .mesh_mut()
            .vertices
            .get_mut(a.as_id())
            .expect("validated vertex")
            .out = t_next;
    }
    if session.mesh().vertices.get(b.as_id()).map(|v| v.out) == Some(twin) {
        session
            .mesh_mut()
            .vertices
            .get_mut(b.as_id())
            .expect("validated vertex")
            .out = h_next;
    }

    // Diagonal corner UVs derive from the sole source corner at the new
    // destination vertex; missing sources clear.
    if let Some(uv) = uv_at_d {
        let _ = session.set_corner_uv_impl(half_edge, uv);
    } else if let Some(layer) = session.mesh_mut().attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(half_edge.as_id());
    }
    if let Some(uv) = uv_at_c {
        let _ = session.set_corner_uv_impl(twin, uv);
    } else if let Some(layer) = session.mesh_mut().attrs_mut().sparse_mut(attr::CORNER_UV) {
        let _ = layer.remove(twin.as_id());
    }

    // Authored corner-normal overrides clear on every affected corner.
    for corner in [half_edge, twin, h_next, h_prev, t_next, t_prev] {
        let _ = session.set_corner_normal_override_impl(corner, None);
        session.mark_corner_dirty(corner);
    }

    session.mark_face_dirty(h_face);
    session.mark_face_dirty(t_face);
    for vertex in [a, b, c, d] {
        session.mark_vertex_dirty(vertex);
    }
    session.invalidate_outgoing_index();
    Ok(half_edge)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use crate::{BuildParams, FaceId, Mesh, attr, op};

    use super::FlipEdgeError;

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

    fn interior_half_edge(mesh: &Mesh) -> crate::HalfEdgeId {
        mesh.faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                mesh.face(half_edge) != Some(FaceId::OUTSIDE)
                    && mesh
                        .twin(half_edge)
                        .and_then(|twin| mesh.face(twin))
                        .is_some_and(|face| face != FaceId::OUTSIDE)
            })
            .expect("interior edge should exist")
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

    #[test]
    fn flip_swaps_diagonal() {
        let mut mesh = two_triangles();
        let edge = interior_half_edge(&mesh);
        let mut session = mesh.edit();
        let diagonal = op::flip_edge(&mut session, edge).expect("flip should succeed");
        let _: () = session.finish();

        assert!(
            mesh.validate_deep().is_empty(),
            "{:?}",
            mesh.validate_deep()
        );
        assert_eq!(mesh.faces().count(), 2);
        assert_eq!(mesh.vertices().count(), 4);
        // The old diagonal (1, 2) is gone; the new diagonal (0, 3) exists.
        assert_eq!(face_vertex_sets(&mesh), [[0, 1, 3], [0, 2, 3]]);
        let from = mesh.from_vertex(diagonal).expect("diagonal from");
        let to = mesh.to_vertex(diagonal).expect("diagonal to");
        assert_eq!(
            [from.index().min(to.index()), from.index().max(to.index())],
            [0, 3]
        );
    }

    #[test]
    fn flip_twice_restores_topology() {
        let mut mesh = two_triangles();
        let before = face_vertex_sets(&mesh);
        let edge = interior_half_edge(&mesh);
        let mut session = mesh.edit();
        let diagonal = op::flip_edge(&mut session, edge).expect("first flip");
        let _: () = session.finish();
        let mut session = mesh.edit();
        op::flip_edge(&mut session, diagonal).expect("second flip");
        let _: () = session.finish();
        assert_eq!(face_vertex_sets(&mesh), before);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn flip_rejects_boundary_edge_and_leaves_mesh_unchanged() {
        let mut mesh = two_triangles();
        let boundary = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                mesh.twin(half_edge)
                    .and_then(|twin| mesh.face(twin))
                    .is_some_and(|face| face == FaceId::OUTSIDE)
            })
            .expect("boundary edge should exist");
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::flip_edge(&mut session, boundary);
        let _: () = session.finish();
        assert!(matches!(result, Err(FlipEdgeError::BoundaryEdge { .. })));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn flip_rejects_non_triangle_face() {
        let mut builder = crate::MeshBuilder::new();
        for position in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, -1.0, 0.0],
        ] {
            builder.push_vertex(position);
        }
        builder.add_face(&[0, 1, 2, 3]).expect("quad");
        builder.add_face(&[1, 0, 4]).expect("triangle");
        let mut mesh = builder.build().expect("mesh should build").mesh;
        let edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                let from = mesh.from_vertex(half_edge).map(crate::VertexId::index);
                let to = mesh.to_vertex(half_edge).map(crate::VertexId::index);
                matches!((from, to), (Some(0), Some(1)) | (Some(1), Some(0)))
                    && mesh.face(half_edge) != Some(FaceId::OUTSIDE)
            })
            .expect("shared edge");
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::flip_edge(&mut session, edge);
        let _: () = session.finish();
        assert!(matches!(result, Err(FlipEdgeError::NonTriangleFace { .. })));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn flip_rejects_existing_diagonal() {
        // An open tetrahedron shell: the flipped diagonal (2, 3) already
        // exists through the third face.
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.5, 1.0, 0.0],
                [0.5, 0.5, 1.0],
            ],
            &[[0, 1, 2], [1, 0, 3], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("mesh should build");
        let edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                let interior = mesh.face(half_edge) != Some(FaceId::OUTSIDE)
                    && mesh
                        .twin(half_edge)
                        .and_then(|twin| mesh.face(twin))
                        .is_some_and(|face| face != FaceId::OUTSIDE);
                let from = mesh.from_vertex(half_edge).map(crate::VertexId::index);
                let to = mesh.to_vertex(half_edge).map(crate::VertexId::index);
                interior && matches!((from, to), (Some(0), Some(1)) | (Some(1), Some(0)))
            })
            .expect("interior edge (0, 1)");
        let before = snapshot(&mesh);
        let mut session = mesh.edit();
        let result = op::flip_edge(&mut session, edge);
        let _: () = session.finish();
        assert!(matches!(result, Err(FlipEdgeError::DiagonalExists { .. })));
        assert_eq!(snapshot(&mesh), before);
    }

    #[test]
    fn flip_propagates_attributes() {
        let mut mesh = two_triangles();
        let edge = interior_half_edge(&mesh);
        let twin = mesh.twin(edge).expect("twin");
        let h_next = mesh.next(edge).expect("next");
        let h_prev = mesh.next(h_next).expect("prev");
        let t_next = mesh.next(twin).expect("twin next");
        let f1 = mesh.face(edge).expect("face");
        let f2 = mesh.face(twin).expect("twin face");
        let uv_c = [0.25, 0.5];
        let uv_d = [0.75, 0.5];

        let mut session = mesh.edit();
        op::set_face_region(&mut session, f1, 7).expect("region f1");
        op::set_face_region(&mut session, f2, 9).expect("region f2");
        op::set_edge_sharpness(&mut session, edge, 4.0).expect("diagonal sharpness");
        op::set_edge_sharpness(&mut session, h_next, 2.0).expect("perimeter sharpness");
        op::set_corner_uv(&mut session, h_next, uv_c).expect("uv at c");
        op::set_corner_uv(&mut session, t_next, uv_d).expect("uv at d");
        op::set_corner_normal_override(&mut session, h_prev, Some([0.0, 0.0, 1.0]))
            .expect("override");
        let _: () = session.finish();

        let c = mesh.to_vertex(h_next).expect("c");
        let d = mesh.to_vertex(t_next).expect("d");
        let mut session = mesh.edit();
        let diagonal = op::flip_edge(&mut session, edge).expect("flip");
        let _: () = session.finish();

        assert!(mesh.validate_deep().is_empty());
        // The new diagonal starts smooth (the authored entry is cleared,
        // so the live default of 0.0 applies); the perimeter edge keeps
        // its authored sharpness.
        assert_eq!(mesh.edge_sharpness(diagonal), Some(0.0));
        assert_eq!(mesh.edge_sharpness(h_next), Some(2.0));
        // The face that kept `next(edge)` keeps its region.
        let f1_vertices: Vec<u32> = mesh
            .face_loop(f1)
            .filter_map(|corner| mesh.to_vertex(corner))
            .map(|vertex| vertex.index())
            .collect();
        assert!(f1_vertices.contains(&c.index()));
        assert_eq!(
            mesh.attrs()
                .dense(attr::FACE_REGION)
                .and_then(|l| l.get(f1.as_id())),
            Some(&7)
        );
        assert_eq!(
            mesh.attrs()
                .dense(attr::FACE_REGION)
                .and_then(|l| l.get(f2.as_id())),
            Some(&9)
        );
        // Diagonal corners derive their UVs from the sole source corner at
        // their new destination vertex.
        assert_eq!(mesh.to_vertex(diagonal), Some(d));
        assert_eq!(
            mesh.attrs()
                .sparse(attr::CORNER_UV)
                .and_then(|l| l.get(diagonal.as_id())),
            Some(&uv_d)
        );
        assert_eq!(
            mesh.attrs()
                .sparse(attr::CORNER_UV)
                .and_then(|l| l.get(twin.as_id())),
            Some(&uv_c)
        );
        // Authored normal overrides clear on all affected corners.
        assert!(
            mesh.attrs()
                .sparse(attr::CORNER_NORMAL_OVERRIDE)
                .and_then(|l| l.get(h_prev.as_id()))
                .is_none()
        );
    }
}
