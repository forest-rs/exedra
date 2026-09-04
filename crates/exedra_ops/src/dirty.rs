// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Ops runtime cache dirtiness channels.
//!
//! This module tracks Exedra Ops-side cache invalidation. Exedra remains the
//! source of truth for mesh topology/attribute edits; this layer exists for
//! runtime caches and workflow state.
//!
//! Memory impact:
//! - one set of `DirtyKey` values per channel (fixed 4 channels)
//! - each entry stores one `DirtyKey` plus hash table overhead
//! - channels should prefer face granularity where possible; use global marks
//!   only when domain-specific IDs are unavailable or too expensive

use alloc::vec::Vec;

use exedra::{ChangeSet, CornerId, FaceId, VertexId};
use invalidation::{Channel, InvalidationSet};

/// Fixed runtime cache channels for Exedra Ops.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DirtyChannel {
    /// Selection and region query caches.
    Selection,
    /// Adjacency and one-ring derived caches.
    Adjacency,
    /// UV-derived runtime caches (not authored UV layers).
    UvDerived,
    /// Generic operator-local cache invalidation.
    OperatorCache,
}

impl DirtyChannel {
    const fn raw(self) -> Channel {
        match self {
            Self::Selection => Channel::new(0),
            Self::Adjacency => Channel::new(1),
            Self::UvDerived => Channel::new(2),
            Self::OperatorCache => Channel::new(3),
        }
    }
}

/// Key granularity for Exedra Ops runtime invalidation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirtyKey {
    /// Coarse invalidation for the whole channel.
    Global,
    /// Face-scoped invalidation.
    Face(FaceId),
    /// Vertex-scoped invalidation.
    Vertex(VertexId),
    /// Corner-scoped invalidation.
    Corner(CornerId),
}

/// Exedra Ops runtime dirty-tracking set.
///
/// Stored in [`OpContext::cache_dirty`](crate::OpContext::cache_dirty) and
/// updated by [`OperatorRunner`](crate::OperatorRunner) after commit.
#[derive(Clone, Debug, Default)]
pub struct CacheDirtySet {
    inner: InvalidationSet<DirtyKey>,
}

impl CacheDirtySet {
    /// Creates an empty cache dirty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when all channels are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the underlying mutation generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation()
    }

    /// Returns `true` when a channel has any dirty keys.
    #[must_use]
    pub fn has_any(&self, channel: DirtyChannel) -> bool {
        self.inner.has_invalidated(channel.raw())
    }

    /// Marks a channel as globally dirty.
    pub fn mark_global(&mut self, channel: DirtyChannel) {
        self.inner.mark(DirtyKey::Global, channel.raw());
    }

    /// Marks one face dirty in `channel`.
    pub fn mark_face(&mut self, channel: DirtyChannel, face: FaceId) {
        self.inner.mark(DirtyKey::Face(face), channel.raw());
    }

    /// Marks one vertex dirty in `channel`.
    pub fn mark_vertex(&mut self, channel: DirtyChannel, vertex: VertexId) {
        self.inner.mark(DirtyKey::Vertex(vertex), channel.raw());
    }

    /// Marks one corner dirty in `channel`.
    pub fn mark_corner(&mut self, channel: DirtyChannel, corner: CornerId) {
        self.inner.mark(DirtyKey::Corner(corner), channel.raw());
    }

    /// Clears one channel.
    pub fn clear(&mut self, channel: DirtyChannel) {
        self.inner.clear(channel.raw());
    }

    /// Drains one channel in deterministic key order into `out`.
    pub fn drain_into(&mut self, channel: DirtyChannel, out: &mut Vec<DirtyKey>) {
        out.clear();
        out.extend(self.inner.drain(channel.raw()));
        out.sort_unstable();
    }

    /// Maps an Exedra change set into Exedra Ops runtime cache channels.
    pub fn mark_from_change_set(&mut self, change_set: &ChangeSet) {
        for face in change_set
            .created_faces
            .iter()
            .copied()
            .chain(change_set.deleted_faces.iter().copied())
        {
            self.mark_face(DirtyChannel::Adjacency, face);
        }
        for vertex in change_set
            .created_vertices
            .iter()
            .copied()
            .chain(change_set.deleted_vertices.iter().copied())
        {
            self.mark_vertex(DirtyChannel::Adjacency, vertex);
        }
        for corner in change_set
            .created_half_edges
            .iter()
            .copied()
            .chain(change_set.deleted_half_edges.iter().copied())
        {
            self.mark_corner(DirtyChannel::Adjacency, CornerId::from(corner.as_id()));
        }

        if !change_set.dirty.is_empty()
            || !change_set.created_vertices.is_empty()
            || !change_set.created_half_edges.is_empty()
            || !change_set.created_faces.is_empty()
            || !change_set.deleted_vertices.is_empty()
            || !change_set.deleted_half_edges.is_empty()
            || !change_set.deleted_faces.is_empty()
        {
            self.mark_global(DirtyChannel::Adjacency);
            self.mark_global(DirtyChannel::OperatorCache);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use super::{CacheDirtySet, DirtyChannel, DirtyKey};

    #[test]
    fn channel_set_clear_and_query() {
        let mut dirty = CacheDirtySet::new();
        let face = exedra::FaceId::from(exedra::Id::new(3, NonZeroU32::MIN));
        assert!(!dirty.has_any(DirtyChannel::Adjacency));

        dirty.mark_face(DirtyChannel::Adjacency, face);
        assert!(dirty.has_any(DirtyChannel::Adjacency));
        assert!(!dirty.has_any(DirtyChannel::Selection));

        dirty.clear(DirtyChannel::Adjacency);
        assert!(!dirty.has_any(DirtyChannel::Adjacency));
    }

    #[test]
    fn drain_is_deterministic() {
        let mut dirty = CacheDirtySet::new();
        let f2 = exedra::FaceId::from(exedra::Id::new(2, NonZeroU32::MIN));
        let f0 = exedra::FaceId::from(exedra::Id::new(0, NonZeroU32::MIN));
        dirty.mark_face(DirtyChannel::Adjacency, f2);
        dirty.mark_global(DirtyChannel::Adjacency);
        dirty.mark_face(DirtyChannel::Adjacency, f0);

        let mut drained = Vec::new();
        dirty.drain_into(DirtyChannel::Adjacency, &mut drained);
        assert_eq!(
            drained,
            vec![DirtyKey::Global, DirtyKey::Face(f0), DirtyKey::Face(f2)]
        );

        dirty.drain_into(DirtyChannel::Adjacency, &mut drained);
        assert!(drained.is_empty());
    }

    #[test]
    fn change_set_mapping_marks_runtime_channels() {
        let mut change_set = exedra::ChangeSet::default();
        let face = exedra::FaceId::from(exedra::Id::new(1, NonZeroU32::MIN));
        change_set.created_faces.push(face);
        change_set.dirty.mark_face(face);

        let mut dirty = CacheDirtySet::new();
        dirty.mark_from_change_set(&change_set);

        let mut adjacency = Vec::new();
        dirty.drain_into(DirtyChannel::Adjacency, &mut adjacency);
        assert!(adjacency.contains(&DirtyKey::Global));
        assert!(adjacency.contains(&DirtyKey::Face(face)));

        let mut op_cache = Vec::new();
        dirty.drain_into(DirtyChannel::OperatorCache, &mut op_cache);
        assert_eq!(op_cache, vec![DirtyKey::Global]);
    }
}
