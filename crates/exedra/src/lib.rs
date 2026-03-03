// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Structural half-edge mesh kernel.

#![no_std]
extern crate alloc;

pub mod arena;
pub mod attr;
pub mod attributes;
pub mod id;
pub mod mesh;
pub mod numeric;
pub mod topology;

pub use arena::Arena;
pub use attributes::{AttrError, AttrKey, Attributes, DenseLayer, Domain, SparseLayer};
pub use id::{CornerId, FaceId, HalfEdgeId, Id, VertexId};
pub use mesh::{
    BuildError, BuildParams, FaceLoopErrorKind, Mesh, MeshBuildResult, MeshBuilder, ValidationError,
};
pub use numeric::NumericPolicy;
pub use topology::{Face, HalfEdge, Vertex};

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
