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
        .map(|(&source, &position)| {
            let copy = op::add_vertex(txn, position);
            copy_vertex_layers(txn, copy, source);
            (source, copy)
        })
        .collect()
}

/// Carries `source`'s caller-defined vertex values onto `copy`.
pub(crate) fn copy_vertex_layers<S: exedra_mesh::ChangeSink>(
    txn: &mut exedra_mesh::EditSession<'_, S>,
    copy: VertexId,
    source: VertexId,
) {
    let captured = txn.mesh().capture_attributes(&[(source, 1.0)]);
    let _ = op::restore_attributes(txn, copy, &captured);
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
