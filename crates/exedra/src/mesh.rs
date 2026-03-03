// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mesh container and core topology traversal over half-edge records.

use crate::{Arena, Attributes, Face, FaceId, HalfEdge, HalfEdgeId, Vertex, VertexId};

/// Half-edge mesh storage with explicit OUTSIDE boundary semantics.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub(crate) vertices: Arena<Vertex>,
    pub(crate) half_edges: Arena<HalfEdge>,
    pub(crate) faces: Arena<Face>,
    pub(crate) attrs: Attributes,
}

impl Mesh {
    /// Creates an empty mesh.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vertices: Arena::new(),
            half_edges: Arena::new(),
            faces: Arena::new(),
            attrs: Attributes::new(),
        }
    }

    /// Returns immutable attribute storage.
    #[must_use]
    pub const fn attrs(&self) -> &Attributes {
        &self.attrs
    }

    /// Returns mutable attribute storage.
    #[must_use]
    pub fn attrs_mut(&mut self) -> &mut Attributes {
        &mut self.attrs
    }

    /// Returns one outgoing half-edge for the given vertex.
    #[must_use]
    pub fn vertex_out(&self, vertex: VertexId) -> Option<HalfEdgeId> {
        self.vertices
            .get(vertex.as_id())
            .map(|record| record.out)
            .filter(|out| *out != HalfEdgeId::INVALID)
    }

    /// Returns one loop half-edge for the given interior face.
    #[must_use]
    pub fn face_edge(&self, face: FaceId) -> Option<HalfEdgeId> {
        if face == FaceId::OUTSIDE {
            return None;
        }
        self.faces.get(face.as_id()).map(|record| record.edge)
    }

    /// Returns the twin half-edge.
    #[must_use]
    pub fn twin(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.twin)
    }

    /// Returns the next half-edge in the owning face loop.
    #[must_use]
    pub fn next(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.next)
    }

    /// Returns the owning face for a half-edge.
    #[must_use]
    pub fn face(&self, half_edge: HalfEdgeId) -> Option<FaceId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.face)
    }

    /// Returns the destination vertex for a half-edge.
    #[must_use]
    pub fn to_vertex(&self, half_edge: HalfEdgeId) -> Option<VertexId> {
        self.half_edges
            .get(half_edge.as_id())
            .map(|record| record.to)
    }

    /// Returns the previous half-edge in the owning face loop.
    ///
    /// This is derived by walking `next` links (v0.1 decision: no stored prev).
    #[must_use]
    pub fn prev(&self, half_edge: HalfEdgeId) -> Option<HalfEdgeId> {
        let face = self.face(half_edge)?;
        let max_steps = if face == FaceId::OUTSIDE {
            self.half_edges.len().max(1)
        } else {
            self.faces
                .get(face.as_id())
                .and_then(|record| usize::try_from(record.degree).ok())
                .unwrap_or_else(|| self.half_edges.len().max(1))
                .max(1)
        };
        let mut cursor = self.next(half_edge).unwrap_or_else(|| {
            panic!(
                "mesh topology corruption: prev() missing next for half-edge {}",
                half_edge.index()
            )
        });
        for _ in 0..max_steps {
            let next = self.next(cursor).unwrap_or_else(|| {
                panic!(
                    "mesh topology corruption: prev() encountered missing next from half-edge {}",
                    cursor.index()
                )
            });
            if next == half_edge {
                return Some(cursor);
            }
            cursor = next;
        }
        panic!(
            "mesh topology corruption: prev() exceeded {} steps for half-edge {} on face {}",
            max_steps,
            half_edge.index(),
            face.index()
        );
    }

    /// Returns the origin vertex for a half-edge.
    ///
    /// This call is `O(face_degree)` because it derives origin via [`Mesh::prev`].
    #[must_use]
    pub fn from_vertex(&self, half_edge: HalfEdgeId) -> Option<VertexId> {
        let prev = self.prev(half_edge)?;
        self.to_vertex(prev)
    }

    /// Iterates half-edges on an interior face loop.
    #[must_use]
    pub fn face_loop(&self, face: FaceId) -> FaceLoopIter<'_> {
        let start = self.face_edge(face);
        let max_steps = self.half_edges.slot_count().max(1);
        FaceLoopIter {
            mesh: self,
            face,
            start,
            current: start,
            steps: 0,
            max_steps,
            exhausted: start.is_none(),
        }
    }

    /// Iterates outgoing half-edges in a vertex star.
    ///
    /// Iteration is deterministic in half-edge slot order.
    ///
    /// Current v0.1 implementation scans all half-edges and filters by
    /// [`Mesh::from_vertex`], so it is `O(total_half_edges * face_degree)` in
    /// the worst case.
    pub fn vertex_star(&self, vertex: VertexId) -> impl Iterator<Item = HalfEdgeId> + '_ {
        self.half_edges.iter().filter_map(move |(id, _)| {
            let typed = HalfEdgeId::from(id);
            (self.from_vertex(typed) == Some(vertex)).then_some(typed)
        })
    }
}

/// Iterator over half-edges in a face loop.
#[derive(Debug)]
pub struct FaceLoopIter<'a> {
    mesh: &'a Mesh,
    face: FaceId,
    start: Option<HalfEdgeId>,
    current: Option<HalfEdgeId>,
    steps: usize,
    max_steps: usize,
    exhausted: bool,
}

impl Iterator for FaceLoopIter<'_> {
    type Item = HalfEdgeId;

    fn next(&mut self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        }
        if self.steps >= self.max_steps {
            let start = self.start.map(HalfEdgeId::index).unwrap_or(u32::MAX);
            panic!(
                "mesh topology corruption: face_loop exceeded {} steps for face {} (start half-edge {})",
                self.max_steps,
                self.face.index(),
                start
            );
        }
        let current = self.current?;
        let next = self.mesh.next(current).unwrap_or_else(|| {
            panic!(
                "mesh topology corruption: face_loop encountered missing next from half-edge {} on face {}",
                current.index(),
                self.face.index()
            )
        });
        if Some(next) == self.start {
            self.exhausted = true;
        } else {
            self.current = Some(next);
        }
        self.steps += 1;
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::Mesh;
    use crate::{Face, FaceId, HalfEdge, HalfEdgeId, Id, Vertex, VertexId};

    fn triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        let v0 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v1 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v2 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b2;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;

        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;

        mesh
    }

    fn quad_mesh_open_boundary() -> (Mesh, FaceId) {
        let mut mesh = Mesh::new();
        let v0 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v1 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v2 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v3 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 4,
        }));

        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h3;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").twin = b3;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b3;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").next = b2;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").twin = h3;

        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.vertices.get_mut(v3.as_id()).expect("v3 live").out = h3;
        (mesh, face)
    }

    fn pentagon_mesh_open_boundary() -> (Mesh, FaceId) {
        let mut mesh = Mesh::new();
        let v0 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v1 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v2 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v3 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v4 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 5,
        }));

        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v4,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h4 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        let b0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b3 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v3,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let b4 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v4,
            face: FaceId::OUTSIDE,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h3;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").next = h4;
        mesh.half_edges.get_mut(h4.as_id()).expect("h4 live").next = h0;
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").twin = b0;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").twin = b1;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").twin = b2;
        mesh.half_edges.get_mut(h3.as_id()).expect("h3 live").twin = b3;
        mesh.half_edges.get_mut(h4.as_id()).expect("h4 live").twin = b4;

        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").next = b4;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").next = b0;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").next = b1;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").next = b2;
        mesh.half_edges.get_mut(b4.as_id()).expect("b4 live").next = b3;
        mesh.half_edges.get_mut(b0.as_id()).expect("b0 live").twin = h0;
        mesh.half_edges.get_mut(b1.as_id()).expect("b1 live").twin = h1;
        mesh.half_edges.get_mut(b2.as_id()).expect("b2 live").twin = h2;
        mesh.half_edges.get_mut(b3.as_id()).expect("b3 live").twin = h3;
        mesh.half_edges.get_mut(b4.as_id()).expect("b4 live").twin = h4;

        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h2;
        mesh.vertices.get_mut(v3.as_id()).expect("v3 live").out = h3;
        mesh.vertices.get_mut(v4.as_id()).expect("v4 live").out = h4;
        (mesh, face)
    }

    #[test]
    fn traversal_accessors_work_on_triangle() {
        let mesh = triangle_mesh();
        let face = FaceId::from(Id::new(0, core::num::NonZeroU32::MIN));
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 3);
        let h0 = loop_ids[0];
        let h1 = mesh.next(h0).expect("next exists");
        assert_eq!(mesh.prev(h1), Some(h0));
        let twin = mesh.twin(h0).expect("twin exists");
        assert_eq!(mesh.face(twin), Some(FaceId::OUTSIDE));
        assert_ne!(mesh.from_vertex(h0), mesh.to_vertex(h0));
    }

    #[test]
    fn face_loop_walks_quad() {
        let (mesh, face) = quad_mesh_open_boundary();
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 4);
    }

    #[test]
    fn vertex_star_includes_boundary_half_edges_for_open_mesh() {
        let (mesh, _) = quad_mesh_open_boundary();
        let v0 = VertexId::from(Id::new(0, core::num::NonZeroU32::MIN));
        let star: Vec<_> = mesh.vertex_star(v0).collect();
        assert_eq!(star.len(), 2);
        assert!(
            star.iter()
                .any(|h| mesh.face(*h).expect("live half-edge") == FaceId::OUTSIDE)
        );
    }

    #[test]
    fn mesh_is_cloneable() {
        let (mesh, face) = quad_mesh_open_boundary();
        let clone = mesh.clone();
        let original_loop: Vec<_> = mesh.face_loop(face).collect();
        let clone_loop: Vec<_> = clone.face_loop(face).collect();
        assert_eq!(original_loop, clone_loop);
    }

    #[test]
    fn prev_walk_works_for_ngons() {
        let (mesh, face) = pentagon_mesh_open_boundary();
        let loop_ids: Vec<_> = mesh.face_loop(face).collect();
        assert_eq!(loop_ids.len(), 5);
        for (index, current) in loop_ids.iter().copied().enumerate() {
            let expected_prev = loop_ids[(index + loop_ids.len() - 1) % loop_ids.len()];
            assert_eq!(mesh.prev(current), Some(expected_prev));
        }
    }

    #[test]
    #[should_panic(expected = "prev() exceeded")]
    fn prev_panics_when_face_loop_does_not_reach_target() {
        let mut mesh = Mesh::new();
        let v0 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v1 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v2 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));

        // Corrupt topology: h0 points to h1, but h1 points to itself.
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h1;
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;
        mesh.vertices.get_mut(v0.as_id()).expect("v0 live").out = h0;
        mesh.vertices.get_mut(v1.as_id()).expect("v1 live").out = h1;
        mesh.vertices.get_mut(v2.as_id()).expect("v2 live").out = h1;

        let _ = mesh.prev(h0);
    }

    #[test]
    #[should_panic(expected = "face_loop exceeded")]
    fn face_loop_panics_when_loop_does_not_close() {
        let mut mesh = Mesh::new();
        let v0 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v1 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let v2 = VertexId::from(mesh.vertices.insert(Vertex {
            out: HalfEdgeId::INVALID,
        }));
        let face = FaceId::from(mesh.faces.insert(Face {
            edge: HalfEdgeId::INVALID,
            degree: 3,
        }));
        let h0 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v1,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h1 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v2,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        let h2 = HalfEdgeId::from(mesh.half_edges.insert(HalfEdge {
            to: v0,
            face,
            next: HalfEdgeId::INVALID,
            twin: HalfEdgeId::INVALID,
        }));
        mesh.faces.get_mut(face.as_id()).expect("face live").edge = h0;

        // Corrupt topology: loop never returns to h0.
        mesh.half_edges.get_mut(h0.as_id()).expect("h0 live").next = h1;
        mesh.half_edges.get_mut(h1.as_id()).expect("h1 live").next = h2;
        mesh.half_edges.get_mut(h2.as_id()).expect("h2 live").next = h1;

        let _: Vec<_> = mesh.face_loop(face).collect();
    }
}
