// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase-local half-edge lookup for Boolean construction.
//!
//! The Boolean splitter and stitcher repeatedly ask whether two already-known
//! vertices share an edge. Walking every face for each query made those
//! identity lookups quadratic in otherwise linear construction phases. This
//! private index maps both directed half-edges of every undirected edge and is
//! used only while the owning mesh is stable, except for the splitter's
//! explicit four-edge refresh after a kernel edge split.

use hashbrown::HashMap;

use crate::{HalfEdgeId, Mesh, VertexId};

#[derive(Clone, Debug, Default)]
pub(super) struct HalfEdgeIndex {
    directed: HashMap<(VertexId, VertexId), HalfEdgeId>,
}

impl HalfEdgeIndex {
    pub(super) fn build(mesh: &Mesh) -> Self {
        let mut index = Self {
            directed: HashMap::with_capacity(mesh.half_edges.len()),
        };
        for (id, _) in mesh.half_edges.iter() {
            index.insert_live(mesh, HalfEdgeId::from(id));
        }
        index
    }

    pub(super) fn find(&self, from: VertexId, to: VertexId) -> Option<HalfEdgeId> {
        self.directed
            .get(&(from, to))
            .copied()
            .or_else(|| self.directed.get(&(to, from)).copied())
    }

    /// Replaces the old undirected edge with the four directed half-edges
    /// created by `op::split_edge`.
    pub(super) fn refresh_after_split(
        &mut self,
        mesh: &Mesh,
        split_half_edge: HalfEdgeId,
        old_from: VertexId,
        old_to: VertexId,
    ) {
        let _ = self.directed.remove(&(old_from, old_to));
        let _ = self.directed.remove(&(old_to, old_from));

        let Some(child) = mesh.next(split_half_edge) else {
            return;
        };
        let Some(split_twin) = mesh.twin(split_half_edge) else {
            return;
        };
        let Some(child_twin) = mesh.twin(child) else {
            return;
        };
        for half_edge in [split_half_edge, child, split_twin, child_twin] {
            self.insert_live(mesh, half_edge);
        }
    }

    fn insert_live(&mut self, mesh: &Mesh, half_edge: HalfEdgeId) {
        let Some(from) = mesh.from_vertex(half_edge) else {
            return;
        };
        let Some(to) = mesh.to_vertex(half_edge) else {
            return;
        };
        let _ = self.directed.insert((from, to), half_edge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::split_edge;
    use crate::{BuildParams, PropagatePolicy};
    use alloc::vec::Vec;

    #[test]
    fn edge_index_refreshes_the_two_edges_created_by_a_split() {
        // The splitter keeps this index across consecutive kernel edge splits;
        // pin both removal of the old carrier and discovery of its children.
        let mut mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle builds");
        let vertices = mesh.vertices().collect::<Vec<_>>();
        let (from, to) = (vertices[0], vertices[1]);
        let mut index = HalfEdgeIndex::build(&mesh);
        let half_edge = index.find(from, to).expect("source edge is indexed");

        let mut session = mesh.edit();
        let inserted =
            split_edge(&mut session, half_edge, &PropagatePolicy::default()).expect("edge splits");
        index.refresh_after_split(session.mesh(), half_edge, from, to);

        assert!(index.find(from, to).is_none());
        assert!(index.find(from, inserted).is_some());
        assert!(index.find(inserted, to).is_some());
    }
}
