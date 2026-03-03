// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable generational ID types used by mesh topology and attributes.

use core::num::NonZeroU32;

/// Core stable handle representation.
///
/// The pair `(index, gen)` provides deterministic identity plus stale-handle
/// rejection when generations are validated by arena storage.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Id {
    index: u32,
    generation: NonZeroU32,
}

impl Id {
    /// Creates a new stable ID from an index and generation.
    #[must_use]
    pub const fn new(index: u32, generation: NonZeroU32) -> Self {
        Self { index, generation }
    }

    /// Returns the slot index component.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Returns the generation component.
    #[must_use]
    pub const fn generation(self) -> NonZeroU32 {
        self.generation
    }
}

macro_rules! define_id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(Id);

        impl $name {
            /// Creates a new typed ID from an index and generation.
            #[must_use]
            pub const fn new(index: u32, generation: NonZeroU32) -> Self {
                Self(Id::new(index, generation))
            }

            /// Returns the underlying generic ID.
            #[must_use]
            pub const fn as_id(self) -> Id {
                self.0
            }

            /// Returns the slot index component.
            #[must_use]
            pub const fn index(self) -> u32 {
                self.0.index()
            }

            /// Returns the generation component.
            #[must_use]
            pub const fn generation(self) -> NonZeroU32 {
                self.0.generation()
            }
        }

        impl From<Id> for $name {
            fn from(value: Id) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Id {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

define_id_type!(VertexId, "Stable ID for a vertex arena slot.");
define_id_type!(HalfEdgeId, "Stable ID for a half-edge arena slot.");
define_id_type!(FaceId, "Stable ID for a face arena slot.");

impl VertexId {
    /// Reserved sentinel ID for "no vertex".
    pub const INVALID: Self = Self::new(u32::MAX, NonZeroU32::MIN);
}

impl HalfEdgeId {
    /// Reserved sentinel ID for "no half-edge".
    pub const INVALID: Self = Self::new(u32::MAX, NonZeroU32::MIN);
}

impl FaceId {
    /// Reserved sentinel ID representing the outside face region.
    pub const OUTSIDE: Self = Self::new(u32::MAX, NonZeroU32::MIN);
}

/// Corner IDs are exactly half-edge IDs (`CornerId == HalfEdgeId`).
pub type CornerId = HalfEdgeId;

#[cfg(test)]
mod tests {
    use core::hash::{Hash, Hasher};
    use core::mem::size_of;
    use core::num::NonZeroU32;
    extern crate std;
    use std::collections::hash_map::DefaultHasher;

    use super::{FaceId, HalfEdgeId, Id, VertexId};

    #[test]
    fn id_equality_works() {
        let generation = NonZeroU32::new(7).expect("non-zero literal must be valid");
        let a = Id::new(3, generation);
        let b = Id::new(3, generation);
        let c = Id::new(4, generation);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn id_hashing_is_stable_for_same_value() {
        let generation = NonZeroU32::new(11).expect("non-zero literal must be valid");
        let a = Id::new(9, generation);
        let b = Id::new(9, generation);
        let c = Id::new(10, generation);

        let hash = |value: Id| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        };

        assert_eq!(hash(a), hash(b));
        assert_ne!(hash(a), hash(c));
    }

    #[test]
    fn typed_ids_are_distinct_and_round_trip() {
        let generation = NonZeroU32::new(13).expect("non-zero literal must be valid");
        let vid = VertexId::new(2, generation);
        let hid = HalfEdgeId::new(2, generation);
        let fid = FaceId::new(2, generation);

        assert_eq!(vid.index(), 2);
        assert_eq!(hid.index(), 2);
        assert_eq!(fid.index(), 2);
        assert_eq!(Id::from(vid), Id::new(2, generation));
        assert_eq!(Id::from(hid), Id::new(2, generation));
        assert_eq!(Id::from(fid), Id::new(2, generation));
    }

    #[test]
    fn outside_face_identity_is_reserved_and_stable() {
        assert_eq!(FaceId::OUTSIDE.index(), u32::MAX);
        assert_eq!(FaceId::OUTSIDE.generation(), NonZeroU32::MIN);
    }

    #[test]
    fn option_sizes_match_for_niche_optimization() {
        assert_eq!(size_of::<Option<Id>>(), size_of::<Id>());
        assert_eq!(size_of::<Option<VertexId>>(), size_of::<VertexId>());
        assert_eq!(size_of::<Option<HalfEdgeId>>(), size_of::<HalfEdgeId>());
        assert_eq!(size_of::<Option<FaceId>>(), size_of::<FaceId>());
    }
}
