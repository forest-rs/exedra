// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lightweight kernel operation catalog for Exedra.
//!
//! This module defines explicit kernel edits over [`EditSession`](crate::EditSession).
//! It is intentionally much smaller than Cambium's workflow-operator layer:
//! - no compile/apply plan lifecycle,
//! - no reports, diagnostics, or artifacts,
//! - no preview/runner abstraction.
//!
//! The boundary is:
//! - [`EditSession`](crate::EditSession) owns transaction hosting, bookkeeping,
//!   dirty tracking, and low-level mutation plumbing.
//! - `exedra::op::*` owns the public kernel mutation catalog.
//!
//! # Example
//! ```rust
//! use exedra::{op, BuildParams, Mesh, PropagatePolicy};
//!
//! let mut mesh = Mesh::from_indexed_triangles(
//!     &[
//!         [0.0, 0.0, 0.0],
//!         [1.0, 0.0, 0.0],
//!         [0.0, 1.0, 0.0],
//!         [1.0, 1.0, 0.0],
//!     ],
//!     &[[0, 1, 2], [2, 1, 3]],
//!     &BuildParams::default(),
//! )
//! .expect("mesh should build");
//! let half_edge = mesh
//!     .faces()
//!     .flat_map(|face| mesh.face_loop(face))
//!     .find(|&half_edge| {
//!         let Some(twin) = mesh.twin(half_edge) else {
//!             return false;
//!         };
//!         core::cmp::min(half_edge, twin) == half_edge
//!             && mesh.face(half_edge) != Some(exedra::FaceId::OUTSIDE)
//!             && mesh.face(twin) != Some(exedra::FaceId::OUTSIDE)
//!     })
//!     .expect("interior edge should exist");
//!
//! let mut session = mesh.begin();
//! let inserted = op::split_edge(&mut session, half_edge, &PropagatePolicy::default())
//!     .expect("split should succeed");
//! let changes = session.commit();
//!
//! assert!(mesh.vertex_position(inserted).is_some());
//! assert_eq!(changes.created_vertices, vec![inserted]);
//! ```

mod add_face;
mod add_vertex;
mod delete_edges;
mod delete_faces;
mod delete_vertices;
mod set_corner_uv;
mod set_edge_seam;
mod set_edge_sharpness;
mod set_face_region;
mod set_vertex_position;
mod split_edge;
mod split_face;

pub use add_face::add_face;
pub use add_vertex::add_vertex;
pub use delete_edges::delete_edges;
pub use delete_faces::delete_faces;
pub use delete_vertices::delete_vertices;
pub use crate::session::{
    AddFaceError, DeleteEdgesError, DeleteFacesError, DeleteVerticesError, SplitEdgeError,
    SplitFaceError,
};
pub use set_corner_uv::{SetCornerUvError, set_corner_uv};
pub use set_edge_seam::{SetEdgeSeamError, set_edge_seam};
pub use set_edge_sharpness::{SetEdgeSharpnessError, set_edge_sharpness};
pub use set_face_region::{SetFaceRegionError, set_face_region};
pub use set_vertex_position::{SetVertexPositionError, set_vertex_position};
pub use split_edge::split_edge;
pub use split_face::split_face;

#[cfg(test)]
mod tests {
    use alloc::vec;

    use crate::{BuildParams, DeletePolicy, FaceId, Mesh, MeshBuilder, PropagatePolicy, op};

    fn triangle_mesh() -> Mesh {
        Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle mesh should build")
    }

    fn quad_mesh() -> Mesh {
        let mut builder = MeshBuilder::new();
        let v0 = builder.push_vertex([0.0, 0.0, 0.0]);
        let v1 = builder.push_vertex([1.0, 0.0, 0.0]);
        let v2 = builder.push_vertex([1.0, 1.0, 0.0]);
        let v3 = builder.push_vertex([0.0, 1.0, 0.0]);
        builder
            .add_face(&[v0, v1, v2, v3])
            .expect("quad face should be accepted");
        builder.build().expect("quad mesh should build").mesh
    }

    #[test]
    fn add_face_creates_face() {
        let mut mesh = MeshBuilder::new().build().expect("empty build").mesh;
        let mut session = mesh.begin();
        let v0 = op::add_vertex(&mut session, [0.0, 0.0, 0.0]);
        let v1 = op::add_vertex(&mut session, [1.0, 0.0, 0.0]);
        let v2 = op::add_vertex(&mut session, [0.0, 1.0, 0.0]);

        let face = op::add_face(&mut session, &[v0, v1, v2]).expect("add face should succeed");
        let changes = session.commit();

        assert!(mesh.face_edge(face).is_some());
        assert_eq!(changes.created_faces, vec![face]);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_edge_succeeds() {
        let mut mesh = triangle_mesh();
        let half_edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .next()
            .expect("triangle should have one face loop");
        let mut session = mesh.begin();

        let inserted = op::split_edge(&mut session, half_edge, &PropagatePolicy::default())
            .expect("split should succeed");
        let changes = session.commit();

        assert!(mesh.vertex_position(inserted).is_some());
        assert_eq!(changes.created_vertices, vec![inserted]);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_faces_succeeds() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("quad should have one face");
        let mut session = mesh.begin();

        op::delete_faces(&mut session, &[face], DeletePolicy::KeepIsolated)
            .expect("delete faces should succeed");
        let changes = session.commit();

        assert_eq!(changes.deleted_faces, vec![face]);
        assert_eq!(mesh.faces().count(), 0);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn split_face_succeeds() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("quad should have one face");
        let corners = mesh.face_loop(face).collect::<vec::Vec<_>>();
        let mut session = mesh.begin();

        let new_face = op::split_face(
            &mut session,
            corners[0],
            corners[2],
            &PropagatePolicy::default(),
        )
        .expect("split face should succeed");
        let changes = session.commit();

        assert_eq!(mesh.faces().count(), 2);
        assert_eq!(changes.created_faces, vec![new_face]);
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_edges_succeeds() {
        let mut mesh = Mesh::from_indexed_triangles(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            &[[0, 1, 2], [2, 1, 3]],
            &BuildParams::default(),
        )
        .expect("mesh should build");
        let edge = mesh
            .faces()
            .flat_map(|face| mesh.face_loop(face))
            .find(|&half_edge| {
                let Some(twin) = mesh.twin(half_edge) else {
                    return false;
                };
                core::cmp::min(half_edge, twin) == half_edge
                    && mesh.face(half_edge) != Some(FaceId::OUTSIDE)
                    && mesh.face(twin) != Some(FaceId::OUTSIDE)
            })
            .expect("interior edge should exist");
        let mut session = mesh.begin();

        op::delete_edges(&mut session, &[edge], DeletePolicy::KeepIsolated)
            .expect("delete edges should succeed");
        let changes = session.commit();

        assert!(!changes.deleted_faces.is_empty());
        assert!(mesh.validate_deep().is_empty());
    }

    #[test]
    fn delete_vertices_succeeds() {
        let mut mesh = MeshBuilder::new().build().expect("empty build").mesh;
        let mut session = mesh.begin();
        let vertex = op::add_vertex(&mut session, [0.0, 0.0, 0.0]);
        let changes_before = session.commit();
        assert_eq!(changes_before.created_vertices, vec![vertex]);

        let mut session = mesh.begin();
        op::delete_vertices(&mut session, &[vertex]).expect("delete vertices should succeed");
        let changes = session.commit();

        assert_eq!(changes.deleted_vertices, vec![vertex]);
        assert!(mesh.vertices().next().is_none());
    }

    #[test]
    fn authored_write_ops_succeed() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("quad should have one face");
        let corner = mesh
            .face_loop(face)
            .next()
            .expect("face should have a corner");
        let vertex = mesh
            .to_vertex(corner)
            .expect("corner should have destination");
        let mut session = mesh.begin();

        op::set_vertex_position(&mut session, vertex, [2.0, 3.0, 4.0])
            .expect("position write should succeed");
        op::set_face_region(&mut session, face, 7).expect("region write should succeed");
        op::set_corner_uv(&mut session, corner, [0.25, 0.75])
            .expect("corner uv write should succeed");
        op::set_edge_seam(&mut session, corner, true).expect("edge seam should succeed");
        op::set_edge_sharpness(&mut session, corner, 2.5).expect("edge sharpness should succeed");
        let _ = session.commit();

        assert_eq!(mesh.vertex_position(vertex), Some(&[2.0, 3.0, 4.0]));
        assert_eq!(
            mesh.attrs()
                .dense(crate::attr::FACE_REGION)
                .and_then(|l| l.get(face.as_id())),
            Some(&7)
        );
        assert_eq!(mesh.edge_seam(corner), Some(true));
        assert_eq!(mesh.edge_sharpness(corner), Some(2.5));
    }
}
