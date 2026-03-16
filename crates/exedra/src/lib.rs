// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra: deterministic half-edge mesh kernel.
//!
//! Exedra provides a compact, explicit core for polygonal mesh editing:
//! - stable generational IDs ([`VertexId`], [`HalfEdgeId`], [`FaceId`]),
//! - half-edge topology with explicit OUTSIDE boundary modeling,
//! - typed dense/sparse attribute layers,
//! - eager edit scopes with optional deterministic change summaries,
//! - deterministic render extraction ([`Mesh::to_trimesh`]).
//!
//! Exedra is the engine tier. For workflow/operator authoring and end-user
//! modeling flows, prefer the Cambium SDK crate (`cambium::...`).
//!
//! The intended engine surface is this crate root (`exedra::...`) via
//! re-exported kernel types like [`Mesh`], [`EditSession`], [`MeshBuilder`],
//! and attribute/key APIs.
//! Construction lives on [`MeshBuilder`] and mesh constructors. Public mutation
//! lives in [`op`]. [`EditSession`] is the eager edit host used to apply those
//! mutation functions and finish optional deterministic [`ChangeSet`] summaries.
//!
//! For deeper narrative docs, see [`manual`].
//!
//! Common entry points:
//! - Attributes and domains: [`attributes`]
//! - Built-in attribute keys: [`attr`]
//! - Mesh construction/traversal: [`Mesh`], [`MeshBuilder`]
//! - Kernel operation catalog: [`op`]
//! - Render extraction: [`ExtractParams`], [`TriMesh`]

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra requires either the `std` or `libm` feature");

mod arena;
pub mod attr;
pub mod attributes;
mod id;
#[cfg(doc)]
pub mod manual;
mod math;
pub mod mesh;
mod normals;
mod numeric;
pub mod op;
mod render;
mod session;
mod sorted_merge;
mod topology;

pub use arena::Arena;
pub use id::{CornerId, FaceId, HalfEdgeId, Id, VertexId};
pub use mesh::{
    BoundaryLoopError, BuildError, BuildParams, ConnectedFaceRegionError, FaceLoopErrorKind, Mesh,
    MeshBuildResult, MeshBuilder, MeshRevision, SelectedFaceBoundaryError, SelectedFacePatchEdge,
    SelectedFacePatchError, SelectedFacePatchSharedEdge, SelectedFacePatchTopology,
    ValidationError,
};
pub use normals::{DerivedCornerNormals, NormalParams, NormalWeightMode, NormalsSource};
pub use numeric::NumericPolicy;
pub use render::{ExtractMode, ExtractParams, ExtractStats, TriMesh};
pub use session::{
    ChangeSet, ChangeSetBuilder, ChangeSink, DeletePolicy, DirtySet, DiscardChanges,
    EdgeAttrPropagation, EditSession, FaceAttrPropagation, NormalOverridePropagation,
    PositionPropagation, PropagatePolicy, UvPropagation,
};
pub use topology::{Face, HalfEdge, Vertex};

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use exedra_testkit::fixtures::triangle_mesh;

    use super::{CornerId, FaceId, HalfEdgeId, Id, VertexId};

    #[test]
    fn option_id_uses_niche_optimization() {
        assert_eq!(size_of::<Option<Id>>(), size_of::<Id>());
        assert_eq!(size_of::<Option<VertexId>>(), size_of::<VertexId>());
        assert_eq!(size_of::<Option<HalfEdgeId>>(), size_of::<HalfEdgeId>());
        assert_eq!(size_of::<Option<CornerId>>(), size_of::<CornerId>());
        assert_eq!(size_of::<Option<FaceId>>(), size_of::<FaceId>());
    }

    #[test]
    fn exedra_tests_can_use_exedra_testkit_fixtures() {
        let mesh = triangle_mesh();
        assert!(mesh.validate_fast().is_empty());
    }
}
