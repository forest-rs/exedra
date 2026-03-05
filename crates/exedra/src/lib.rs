// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra: deterministic half-edge mesh kernel.
//!
//! Exedra provides a compact, explicit core for polygonal mesh editing:
//! - stable generational IDs ([`VertexId`], [`HalfEdgeId`], [`FaceId`]),
//! - half-edge topology with explicit OUTSIDE boundary modeling,
//! - typed dense/sparse attribute layers,
//! - transactional edits with deterministic change summaries,
//! - deterministic render extraction ([`Mesh::to_trimesh`]).
//!
//! The intended public surface is the crate root (`exedra::...`) via
//! re-exported core types like [`Mesh`], [`Txn`], [`MeshBuilder`], and
//! attribute/key APIs.
//!
//! For deeper narrative docs, see [`manual`].
//!
//! Common entry points:
//! - Attributes and domains: [`attributes`]
//! - Built-in attribute keys: [`attr`]
//! - Mesh construction/traversal: [`Mesh`], [`MeshBuilder`]
//! - Render extraction: [`ExtractParams`], [`TriMesh`]

#![no_std]
extern crate alloc;

mod arena;
pub mod attr;
pub mod attributes;
mod id;
#[cfg(doc)]
pub mod manual;
pub mod mesh;
mod numeric;
mod render;
mod sorted_merge;
mod topology;
mod txn;

pub use arena::Arena;
pub use id::{CornerId, FaceId, HalfEdgeId, Id, VertexId};
pub use mesh::{
    BuildError, BuildParams, FaceLoopErrorKind, Mesh, MeshBuildResult, MeshBuilder, MeshRevision,
    ValidationError,
};
pub use numeric::NumericPolicy;
pub use render::{ExtractMode, ExtractParams, ExtractStats, TriMesh};
pub use topology::{Face, HalfEdge, Vertex};
pub use txn::{
    AddFaceError, ChangeSet, DeleteEdgesError, DeleteFacesError, DeletePolicy, DeleteVerticesError,
    DirtySet, EdgeAttrPropagation, FaceAttrPropagation, NormalOverridePropagation,
    PositionPropagation, PropagatePolicy, SplitEdgeError, SplitFaceError, Txn, UvPropagation,
};

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::{CornerId, FaceId, HalfEdgeId, Id, VertexId};

    #[test]
    fn option_id_uses_niche_optimization() {
        assert_eq!(size_of::<Option<Id>>(), size_of::<Id>());
        assert_eq!(size_of::<Option<VertexId>>(), size_of::<VertexId>());
        assert_eq!(size_of::<Option<HalfEdgeId>>(), size_of::<HalfEdgeId>());
        assert_eq!(size_of::<Option<CornerId>>(), size_of::<CornerId>());
        assert_eq!(size_of::<Option<FaceId>>(), size_of::<FaceId>());
    }
}
