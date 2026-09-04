// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic primitive mesh generators for Exedra.
//!
//! This crate provides a curated constructor surface at the crate root:
//! - constructors: [`quad()`], [`grid()`], [`box_primitive()`], [`cylinder()`],
//!   [`cone()`], [`torus()`], [`icosphere()`], [`uv_sphere()`],
//! - parameter types: [`QuadParams`], [`GridParams`], [`BoxParams`],
//!   [`CylinderParams`], [`ConeParams`], [`TorusParams`], [`IcosphereParams`],
//!   [`UvSphereParams`],
//! - semantic region/selection metadata in [`Primitive`].
//!
//! Primitive outputs are deterministic for fixed input parameters and include:
//! - generated [`Mesh`] topology,
//! - face-region assignment ([`FaceRegionLayer`]),
//! - named canonical face/edge selections ([`Selections`]).
//!
//! # Numeric Policy
//!
//! Rotational primitives sample angles as `i * TAU / segments` and use the
//! crate-local trigonometric backend only to compute coordinates. Topology,
//! region assignment, and selections are derived from integer segment/ring
//! indices, never from trigonometric results.
//!
//! The default `std` feature uses the platform `f32` math implementation.
//! Enabling `libm` selects the optional `libm` backend, including when Cargo
//! feature unification also enables `std`. The crate does not maintain a
//! custom polynomial approximation. Bit-identical coordinates across `std`
//! and `libm` backends are not guaranteed; deterministic output is guaranteed
//! for a fixed backend, fixed target, and fixed parameter set.
//!
//! Tests enforce a unit-circle sampled-angle absolute error budget of `2e-6`
//! relative to an `f64` reference for the angles used by cylinder, cone, torus,
//! and UV sphere generation. Coordinate error scales linearly with radius.
//!
//! Typical flow:
//! 1. Build a primitive (for example [`quad()`]).
//! 2. Read semantic metadata from [`Primitive::face_region`] and
//!    [`Primitive::selections`].
//! 3. Pass [`Primitive::mesh`] into Exedra/Exedra Ops workflows.
//!
//! ```
//! use exedra_primitives::{BoxParams, box_primitive};
//!
//! let primitive = box_primitive(&BoxParams {
//!     size: [2.0, 1.0, 0.5],
//!     ..BoxParams::default()
//! });
//! assert!(primitive.mesh.validate_deep().is_empty());
//!
//! let (_, top) = primitive
//!     .selections
//!     .face_sets
//!     .iter()
//!     .find(|(name, _)| name.0 == "faces.side_z_pos")
//!     .expect("boxes publish a top-face selection");
//! assert_eq!(top.as_slice().len(), 1);
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[cfg(all(test, not(feature = "std")))]
extern crate std;
#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_primitives requires either the `std` or `libm` feature");

use alloc::vec::Vec;

use exedra_mesh::{FaceId, HalfEdgeId, Mesh};

mod box_primitive;
mod common;
mod cone;
mod cylinder;
mod grid;
mod icosphere;
mod quad;
mod torus;
mod uv_sphere;

pub use box_primitive::{BoxParams, box_primitive};
pub use box_primitive::{
    REGION_SIDE_X_NEG, REGION_SIDE_X_POS, REGION_SIDE_Y_NEG, REGION_SIDE_Y_POS, REGION_SIDE_Z_NEG,
    REGION_SIDE_Z_POS,
};
pub use cone::{
    ConeParams, REGION_CAP_BOTTOM as REGION_CONE_CAP_BOTTOM, REGION_SIDE as REGION_CONE_SIDE, cone,
};
pub use cylinder::{CylinderParams, cylinder};
pub use cylinder::{REGION_CAP_BOTTOM, REGION_CAP_TOP, REGION_SIDE};
pub use grid::{GridParams, REGION_GRID, grid};
pub use icosphere::{IcosphereParams, REGION_BODY as REGION_ICOSPHERE_BODY, icosphere};
pub use quad::{QuadParams, REGION_FACE, quad};
pub use torus::{REGION_BODY as REGION_TORUS_BODY, TorusParams, torus};
pub use uv_sphere::{REGION_BODY, REGION_POLE_BOTTOM, REGION_POLE_TOP};
pub use uv_sphere::{UvSphereParams, uv_sphere};

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

/// Semantic region ID used by [`FaceRegionLayer`] and primitive region constants.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct RegionId(pub u32);

/// Cap face generation mode for rotational primitives.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum CapFill {
    /// Do not generate cap faces.
    None,
    /// Generate one ngon face per cap.
    Ngon,
    /// Generate one triangle fan per cap (shared center vertex).
    TriangleFan,
}

/// Dense face-domain semantic region metadata produced by primitive constructors.
///
/// Primitive generators assign stable region IDs (for example side/cap/pole) so
/// downstream tools can select and process semantic parts of a shape without
/// re-deriving them from geometry.
///
/// This layer is intentionally shipped as primitive metadata (rather than
/// written directly into `mesh.attrs()`) so callers can choose how to map or
/// merge region semantics into their own attribute schemas.
///
/// [`FaceRegionLayer::values`] is indexed by [`FaceId::index`]. Missing indices
/// resolve to [`FaceRegionLayer::default`].
///
/// Typical usage:
/// 1. Build a primitive.
/// 2. For each mesh face, read `face_region.get(face)`.
/// 3. Map region IDs into your material/selection/tagging workflow.
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

/// Stable selection name used in [`Selections`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SelectionName(pub &'static str);

/// Named canonical face/edge selections produced by primitive constructors.
///
/// Selections provide pre-authored semantic subsets (for example `"faces.all"`,
/// `"faces.cap_top"`, `"edges.seam"`) so callers can target meaningful parts
/// of a primitive without recomputing adjacency or geometry predicates.
///
/// Selection names are stable API strings and should be treated as contracts by
/// downstream tools. The contained sets are canonical (sorted + deduplicated)
/// for deterministic processing.
///
/// Typical usage:
/// 1. Find a named set in [`Selections::face_sets`] or [`Selections::edge_sets`].
/// 2. Iterate IDs from [`FaceSet::as_slice`] / [`EdgeSet::as_slice`].
/// 3. Apply region/tag/UV operations to that deterministic subset.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selections {
    /// Named face sets (for example `faces.all`, `faces.cap_top`).
    pub face_sets: Vec<(SelectionName, FaceSet)>,
    /// Named edge sets (for example `edges.seam`, `edges.rim_top`).
    pub edge_sets: Vec<(SelectionName, EdgeSet)>,
}

/// Canonical set of faces (sorted by stable ID, deduplicated).
///
/// Used inside [`Selections::face_sets`].
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
///
/// Used inside [`Selections::edge_sets`].
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
/// Returns `true` when the vector content changed. This is used by
/// [`FaceSet::from_vec`].
pub fn sort_and_dedup_faces(values: &mut Vec<FaceId>) -> bool {
    let before = values.clone();
    values.sort_unstable();
    values.dedup();
    *values != before
}

/// Canonicalizes half-edge IDs in-place (stable ID sort + dedup).
///
/// Returns `true` when the vector content changed. This is used by
/// [`EdgeSet::from_vec`].
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

    use exedra_mesh::{FaceId, HalfEdgeId};

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
