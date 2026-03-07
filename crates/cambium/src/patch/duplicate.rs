// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::collections::BTreeMap;

use exedra::{VertexId, op};

pub(crate) fn create_vertex_copies<S: exedra::ChangeSink>(
    txn: &mut exedra::EditSession<'_, S>,
    positions: &BTreeMap<VertexId, [f32; 3]>,
) -> BTreeMap<VertexId, VertexId> {
    positions
        .iter()
        .map(|(&source, &position)| (source, op::add_vertex(txn, position)))
        .collect()
}
