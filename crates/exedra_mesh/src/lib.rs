// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exedra Mesh: deterministic half-edge mesh kernel.
//!
//! Exedra Mesh provides a compact, explicit core for polygonal mesh editing:
//! - stable generational IDs ([`VertexId`], [`HalfEdgeId`], [`FaceId`]),
//! - half-edge topology with explicit OUTSIDE boundary modeling,
//! - typed dense/sparse attribute layers,
//! - eager edit scopes with optional deterministic change summaries,
//! - deterministic render extraction ([`Mesh::to_trimesh`]).
//!
//! Exedra Mesh is the engine tier. For workflow/operator authoring and end-user
//! modeling flows, prefer the Exedra Ops crate (`exedra_ops::...`). Its runner
//! is mesh-specific; cross-domain work uses explicit adapters owned by the
//! participating domains.
//!
//! The intended engine surface is this crate root (`exedra_mesh::...`) via
//! re-exported kernel types like [`Mesh`], [`EditSession`], [`MeshBuilder`],
//! [`Remap`], and attribute/key APIs.
//! Construction lives on [`MeshBuilder`] and mesh constructors. Public mutation
//! lives in [`op`]. [`EditSession`] is the eager edit host used to apply those
//! mutation functions and finish optional deterministic [`ChangeSet`] summaries.
//!
//! For deeper narrative docs, see [`manual`].
//! The current public surface audit is recorded in
//! `crates/exedra_mesh/docs/api-surface.md`.
//!
//! Common entry points:
//! - Attributes and domains: [`attributes`]
//! - Built-in attribute keys: [`attr`]
//! - Mesh construction/traversal: [`Mesh`], [`MeshBuilder`]
//! - Explicit compaction: [`Mesh::compact`], [`Remap`]
//! - Kernel mutation surface: [`op`]
//! - Render extraction: [`ExtractParams`], [`TriMesh`]
//! - Boolean broad phase: [`boolean`], [`BooleanBvh`]
//!
//! # Guarantees
//!
//! Exedra Mesh guarantees deterministic traversal and output for fixed mesh state:
//! live vertices/faces iterate in stable arena slot order, face fan
//! triangulation is stable, and render extraction appends render vertices at
//! first encounter. Stable IDs include both index and generation, so deleted
//! topology cannot be silently reused through stale handles.
//!
//! # Non-goals
//!
//! Exedra Mesh does not own scene graphs, units, materials, UI/operator workflows,
//! or exact analytic/CAD topology. Higher-level modeling policy belongs in
//! Exedra Ops or sibling domain crates. Exedra Mesh also does not compact IDs
//! implicitly; use [`Mesh::compact`] when you want a tombstone-free copy.
//!
//! # Core Concepts
//!
//! - **Half-edge model**: every edge has two directed half-edges; boundary
//!   twins are explicit records whose face is [`FaceId::OUTSIDE`].
//! - **Corners**: [`CornerId`] is exactly [`HalfEdgeId`], so corner UVs and
//!   normal overrides can differ per face without splitting topology vertices.
//! - **Attributes**: built-ins live under [`attr`]. Required vertex positions
//!   and face regions are dense; UVs, seams, sharpness, and normal overrides are
//!   sparse authored layers.
//! - **Seams and sharpness**: edge-wide tags use the canonical half-edge
//!   representative from [`Mesh::canonical_edge`].
//! - **Render extraction**: [`Mesh::to_trimesh`] triangulates faces with a
//!   stable fan and creates distinct render vertices for a shared topology
//!   vertex when corner UVs or corner normals differ.
//! - **Boolean broad phase**: [`BooleanBvh`] reports deterministic AABB-overlap
//!   candidate pairs over face triangles enumerated under an explicit
//!   [`FaceTriangulation`] strategy.
//! - **Edit sessions**: mutations are eager through [`EditSession`], with
//!   optional [`ChangeSet`] and [`DirtySet`] output for incremental consumers.
//! - **Numeric policy**: [`NumericPolicy`] centralizes tolerances for geometry
//!   operations that need near-equality decisions.
//!
//! # Example
//!
//! ```rust
//! use exedra_mesh::{BuildParams, ChangeSetBuilder, ExtractParams, Mesh, op};
//!
//! let mut mesh = Mesh::from_indexed_triangles(
//!     &[
//!         [0.0, 0.0, 0.0],
//!         [1.0, 0.0, 0.0],
//!         [0.0, 1.0, 0.0],
//!     ],
//!     &[[0, 1, 2]],
//!     &BuildParams::default(),
//! )?;
//! let face = mesh.faces().next().expect("triangle face");
//! let corner = mesh.face_loop(face).next().expect("triangle corner");
//!
//! let mut edit = mesh.edit_with(ChangeSetBuilder::new());
//! op::set_corner_uv(&mut edit, corner, [0.0, 0.0]).expect("corner is live");
//! let changes = edit.finish();
//! assert!(changes.dirty.has_dirty_corners());
//!
//! let (triangles, stats) = mesh.to_trimesh(&ExtractParams::default());
//! assert_eq!(triangles.indices.len(), 3);
//! assert_eq!(stats.triangle_count, 1);
//! # Ok::<(), exedra_mesh::BuildError>(())
//! ```

#![no_std]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

#[cfg(not(any(feature = "std", feature = "libm")))]
compile_error!("exedra_mesh requires either the `std` or `libm` feature");

mod arena;
pub mod attr;
pub mod attributes;
pub mod boolean;
mod id;
#[cfg(doc)]
pub mod manual;
mod math;
pub mod mesh;
mod normals;
mod numeric;
pub mod op;
mod render;
pub mod round;
mod session;
mod sorted_merge;
mod topology;

pub use arena::Arena;
pub use boolean::{
    Aabb, BooleanBroadPhaseStats, BooleanBvh, BooleanCandidatePair, BooleanScratch,
    BooleanTriangleRef,
};
pub use id::{CornerId, FaceId, HalfEdgeId, Id, VertexId};
pub use mesh::{
    BoundaryLoopError, BuildError, BuildParams, ConnectedFaceRegionError, FaceAttrErrorKind,
    FaceBuildAttrs, FaceLoopErrorKind, FaceTriangulation, Mesh, MeshBuildResult, MeshBuilder,
    MeshRevision, Remap, SelectedFaceBoundaryError, SelectedFacePatchEdge, SelectedFacePatchError,
    SelectedFacePatchSharedEdge, SelectedFacePatchTopology, ValidationError,
};
pub use normals::{DerivedCornerNormals, NormalParams, NormalWeightMode, NormalsSource};
pub use numeric::NumericPolicy;
pub use render::{ExtractMode, ExtractParams, ExtractStats, TriMesh, TrimeshCache};
pub use round::{RoundError, RoundKind, RoundPolicy, RoundStats, round_sharp_edges};
pub use session::{
    ChangeSet, ChangeSetBuilder, ChangeSink, DeletePolicy, DirtySet, DiscardChanges,
    EdgeAttrPropagation, EditSession, FaceAttrPropagation, NormalOverridePropagation,
    PositionPropagation, PropagatePolicy, SplitFaceDiagonalEdgePropagation, UvPropagation,
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
