// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic mesh primitive generators for exedra.

#![no_std]
extern crate alloc;

use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId, Mesh};

/// Output bundle produced by primitive constructors.
///
/// Primitive generators return topology (`mesh`) plus semantic metadata used by
/// downstream tooling.
#[derive(Clone, Debug)]
pub struct Primitive {
    /// Generated half-edge mesh.
    pub mesh: Mesh,
    /// Per-face semantic region assignment.
    pub face_region: FaceRegionLayer,
    /// Named canonical face/edge selections.
    pub selections: Selections,
}

/// Semantic region ID used by primitive outputs.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct RegionId(pub u32);

/// Dense face-domain region layer.
///
/// `values` is indexed by `FaceId::index()`. Missing indices resolve to
/// `default`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FaceRegionLayer {
    /// Default region returned when a face index is not present in `values`.
    pub default: RegionId,
    /// Dense values indexed by `FaceId::index()`.
    pub values: Vec<RegionId>,
}

impl FaceRegionLayer {
    /// Creates a new region layer with `len` entries initialized to `default`.
    #[must_use]
    pub fn new(default: RegionId, len: usize) -> Self {
        Self {
            default,
            values: alloc::vec![default; len],
        }
    }

    /// Returns the region for `face`, falling back to `default` when missing.
    #[must_use]
    pub fn get(&self, face: FaceId) -> RegionId {
        self.values
            .get(face.index() as usize)
            .copied()
            .unwrap_or(self.default)
    }
}

/// Stable selection name used for primitive metadata.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SelectionName(pub &'static str);

/// Named canonical face/edge selections produced by primitives.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selections {
    /// Named face sets (for example `faces.all`, `faces.cap_top`).
    pub face_sets: Vec<(SelectionName, FaceSet)>,
    /// Named edge sets (for example `edges.seam`, `edges.rim_top`).
    pub edge_sets: Vec<(SelectionName, EdgeSet)>,
}

/// Canonical set of faces (sorted by stable ID, deduplicated).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FaceSet(Vec<FaceId>);

impl FaceSet {
    /// Returns an empty canonical face set.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Builds a canonical face set from arbitrary input ordering.
    #[must_use]
    pub fn from_vec(mut values: Vec<FaceId>) -> Self {
        sort_and_dedup_faces(&mut values);
        Self(values)
    }

    /// Returns face IDs in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[FaceId] {
        &self.0
    }

    /// Consumes the set and returns the underlying canonical face IDs.
    #[must_use]
    pub fn into_vec(self) -> Vec<FaceId> {
        self.0
    }
}

impl Default for FaceSet {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<FaceId>> for FaceSet {
    fn from(values: Vec<FaceId>) -> Self {
        Self::from_vec(values)
    }
}

/// Canonical set of half-edges (sorted by stable ID, deduplicated).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeSet(Vec<HalfEdgeId>);

impl EdgeSet {
    /// Returns an empty canonical edge set.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Builds a canonical edge set from arbitrary input ordering.
    #[must_use]
    pub fn from_vec(mut values: Vec<HalfEdgeId>) -> Self {
        sort_and_dedup_edges(&mut values);
        Self(values)
    }

    /// Returns half-edge IDs in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[HalfEdgeId] {
        &self.0
    }

    /// Consumes the set and returns the underlying canonical edge IDs.
    #[must_use]
    pub fn into_vec(self) -> Vec<HalfEdgeId> {
        self.0
    }
}

impl Default for EdgeSet {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<HalfEdgeId>> for EdgeSet {
    fn from(values: Vec<HalfEdgeId>) -> Self {
        Self::from_vec(values)
    }
}

/// Canonicalizes face IDs in-place (stable ID sort + dedup).
///
/// Returns `true` when the vector content changed.
pub fn sort_and_dedup_faces(values: &mut Vec<FaceId>) -> bool {
    let before = values.clone();
    values.sort_unstable();
    values.dedup();
    *values != before
}

/// Canonicalizes half-edge IDs in-place (stable ID sort + dedup).
///
/// Returns `true` when the vector content changed.
pub fn sort_and_dedup_edges(values: &mut Vec<HalfEdgeId>) -> bool {
    let before = values.clone();
    values.sort_unstable();
    values.dedup();
    *values != before
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use core::num::NonZeroU32;

    use exedra::{FaceId, HalfEdgeId};

    use crate::{
        EdgeSet, FaceRegionLayer, FaceSet, RegionId, sort_and_dedup_edges, sort_and_dedup_faces,
    };

    #[test]
    fn sort_and_dedup_faces_orders_and_deduplicates() {
        let generation = NonZeroU32::new(1).expect("literal must be non-zero");
        let f1 = FaceId::new(1, generation);
        let f2 = FaceId::new(2, generation);
        let f3 = FaceId::new(3, generation);
        let mut values = vec![f3, f1, f2, f1, f3];

        assert!(sort_and_dedup_faces(&mut values));
        assert_eq!(values, vec![f1, f2, f3]);
    }

    #[test]
    fn sort_and_dedup_faces_reports_unchanged_for_canonical_input() {
        let generation = NonZeroU32::new(1).expect("literal must be non-zero");
        let f1 = FaceId::new(1, generation);
        let f2 = FaceId::new(2, generation);
        let mut values = vec![f1, f2];

        assert!(!sort_and_dedup_faces(&mut values));
        assert_eq!(values, vec![f1, f2]);
    }

    #[test]
    fn sort_and_dedup_edges_orders_and_deduplicates() {
        let generation = NonZeroU32::new(1).expect("literal must be non-zero");
        let e1 = HalfEdgeId::new(10, generation);
        let e2 = HalfEdgeId::new(20, generation);
        let e3 = HalfEdgeId::new(30, generation);
        let mut values = vec![e2, e3, e1, e3, e2];

        assert!(sort_and_dedup_edges(&mut values));
        assert_eq!(values, vec![e1, e2, e3]);
    }

    #[test]
    fn face_and_edge_set_from_vec_are_canonical() {
        let generation = NonZeroU32::new(1).expect("literal must be non-zero");
        let f1 = FaceId::new(1, generation);
        let f2 = FaceId::new(2, generation);
        let e1 = HalfEdgeId::new(11, generation);
        let e2 = HalfEdgeId::new(22, generation);

        let face_set = FaceSet::from_vec(vec![f2, f1, f2]);
        let edge_set = EdgeSet::from_vec(vec![e2, e1, e1]);

        assert_eq!(face_set.as_slice(), &[f1, f2]);
        assert_eq!(edge_set.as_slice(), &[e1, e2]);
    }

    #[test]
    fn face_region_layer_get_uses_default_for_missing_slot() {
        let generation = NonZeroU32::new(1).expect("literal must be non-zero");
        let default = RegionId(7);
        let layer = FaceRegionLayer::new(default, 1);
        let missing = FaceId::new(5, generation);

        assert_eq!(layer.get(missing), default);
    }
}
