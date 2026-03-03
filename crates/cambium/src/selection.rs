// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Canonical selection representation helpers.

use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId};

/// Canonical face selection (`Vec<FaceId>` sorted and deduplicated).
pub type FaceSet = Vec<FaceId>;
/// Canonical edge selection (`Vec<HalfEdgeId>` sorted and deduplicated).
pub type EdgeSet = Vec<HalfEdgeId>;

/// Canonicalizes a face selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_face_set(faces: &mut FaceSet) -> bool {
    let mut changed = faces.windows(2).any(|pair| pair[0] >= pair[1]);
    let len_before = faces.len();
    faces.sort_unstable();
    faces.dedup();
    changed |= faces.len() != len_before;
    changed
}

/// Canonicalizes a half-edge selection in-place.
///
/// Returns `true` if the input was modified.
pub fn canonicalize_edge_set(edges: &mut EdgeSet) -> bool {
    let mut changed = edges.windows(2).any(|pair| pair[0] >= pair[1]);
    let len_before = edges.len();
    edges.sort_unstable();
    edges.dedup();
    changed |= edges.len() != len_before;
    changed
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use exedra::{FaceId, HalfEdgeId, Id};

    use super::{EdgeSet, FaceSet, canonicalize_edge_set, canonicalize_face_set};

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
    fn canonicalize_edge_set_sorts_and_dedups() {
        let e0 = HalfEdgeId::from(Id::new(0, NonZeroU32::MIN));
        let e1 = HalfEdgeId::from(Id::new(1, NonZeroU32::MIN));
        let e2 = HalfEdgeId::from(Id::new(2, NonZeroU32::MIN));
        let mut edges: EdgeSet = vec![e2, e1, e2, e0];

        let changed = canonicalize_edge_set(&mut edges);
        assert!(changed);
        assert_eq!(edges, vec![e0, e1, e2]);
    }
}
