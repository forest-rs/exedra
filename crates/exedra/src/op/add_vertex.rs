// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{ChangeSink, EditSession, VertexId};

/// Inserts one vertex with a required position.
#[must_use]
pub fn add_vertex<S: ChangeSink>(session: &mut EditSession<'_, S>, position: [f32; 3]) -> VertexId {
    session.add_vertex_impl(position)
}
