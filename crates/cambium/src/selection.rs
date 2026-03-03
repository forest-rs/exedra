// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Canonical selection representation helpers.

use alloc::vec::Vec;

use exedra::FaceId;

/// Canonical face selection (`Vec<FaceId>` sorted and deduplicated).
pub type FaceSet = Vec<FaceId>;

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

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use exedra::{FaceId, Id};

    use super::{FaceSet, canonicalize_face_set};

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
}
