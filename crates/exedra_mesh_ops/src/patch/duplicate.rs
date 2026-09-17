// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use exedra_mesh::{VertexId, op};

pub(crate) fn create_vertex_copies<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    positions: &BTreeMap<VertexId, [f32; 3]>,
) -> BTreeMap<VertexId, VertexId> {
    positions
        .iter()
        .map(|(&source, &position)| (source, op::add_vertex(txn, position)))
        .collect()
}

pub(crate) fn map_vertex_loop(
    vertices: &[VertexId],
    copies: &BTreeMap<VertexId, VertexId>,
) -> Vec<VertexId> {
    vertices
        .iter()
        .map(|vertex| {
            *copies
                .get(vertex)
                .expect("source vertex should have a copied vertex")
        })
        .collect()
}
