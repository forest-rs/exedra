// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Canonical selection representation helpers.

use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId, VertexId};

/// Canonical face selection (`Vec<FaceId>` sorted and deduplicated).
///
/// Used by operator params such as
/// [`TagFaceRegionParams::faces`](crate::TagFaceRegionParams::faces).
pub type FaceSet = Vec<FaceId>;
/// Canonical edge selection (`Vec<HalfEdgeId>` sorted and deduplicated).
///
/// Used by edge-mark operators and UV/tagging selection APIs.
pub type EdgeSet = Vec<HalfEdgeId>;
/// Canonical vertex selection (`Vec<VertexId>` sorted and deduplicated).
pub type VertexSet = Vec<VertexId>;

/// Canonicalizes a face selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_face_set(faces: &mut FaceSet) -> bool {
    canonicalize_sorted_unique(faces)
}

/// Canonicalizes a half-edge selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_edge_set(edges: &mut EdgeSet) -> bool {
    canonicalize_sorted_unique(edges)
}

/// Canonicalizes a vertex selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_vertex_set(vertices: &mut VertexSet) -> bool {
    canonicalize_sorted_unique(vertices)
}

fn canonicalize_sorted_unique<T: Ord>(values: &mut Vec<T>) -> bool {
    if values.windows(2).all(|pair| pair[0] < pair[1]) {
        return false;
    }
    values.sort_unstable();
    values.dedup();
    true
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use exedra::{FaceId, HalfEdgeId, Id, VertexId};

    use super::{
        EdgeSet, FaceSet, VertexSet, canonicalize_edge_set, canonicalize_face_set,
        canonicalize_vertex_set,
    };

    #[test]
    fn canonicalize_face_set_sorts_and_dedups() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let f2 = FaceId::from(Id::new(2, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f2, f1, f2, f0];

        let changed = canonicalize_face_set(&mut faces);
        assert!(changed);
        assert_eq!(faces, vec![f0, f1, f2]);
    }

    #[test]
    fn canonicalize_face_set_reports_no_change_for_canonical_input() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f0, f1];
        let changed = canonicalize_face_set(&mut faces);
        assert!(!changed);
    }

    #[test]
    fn canonicalize_face_set_empty_input_is_stable() {
        let mut faces: FaceSet = vec![];
        let changed = canonicalize_face_set(&mut faces);
        assert!(!changed);
        assert!(faces.is_empty());
    }

    #[test]
    fn canonicalize_face_set_reports_change_for_reordered_unique_input() {
        let f0 = FaceId::from(Id::new(0, NonZeroU32::MIN));
        let f1 = FaceId::from(Id::new(1, NonZeroU32::MIN));
        let mut faces: FaceSet = vec![f1, f0];

        let changed = canonicalize_face_set(&mut faces);
        assert!(changed);
        assert_eq!(faces, vec![f0, f1]);
    }

    #[test]
    fn canonicalize_edge_set_sorts_and_dedups() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let e1 = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        let e2 = HalfEdgeId::from(Id::new(2, NonZeroU32::MIN));
        let mut edges: EdgeSet = vec![e2, e1, e2, e0];

        let changed = canonicalize_edge_set(&mut edges);
        assert!(changed);
        assert_eq!(edges, vec![e0, e1, e2]);
    }

    #[test]
    fn canonicalize_edge_set_reports_no_change_for_canonical_input() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let e1 = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        let mut edges: EdgeSet = vec![e0, e1];
        let changed = canonicalize_edge_set(&mut edges);
        assert!(!changed);
    }

    #[test]
    fn canonicalize_vertex_set_sorts_and_dedups() {
        let v0 = VertexId::from(Id::new(0, NonZeroU32::MIN));
        let v1 = VertexId::from(Id::new(1, NonZeroU32::MIN));
        let v2 = VertexId::from(Id::new(2, NonZeroU32::MIN));
        let mut vertices: VertexSet = vec![v2, v1, v2, v0];

        let changed = canonicalize_vertex_set(&mut vertices);
        assert!(changed);
        assert_eq!(vertices, vec![v0, v1, v2]);
    }
}
