// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test helpers for Exedra fixtures, snapshots, and debug dumps.

#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod dump;
pub mod fixtures;
pub mod golden;

pub use dump::{dump_attributes, dump_mesh_topology, mesh_to_obj};
pub use fixtures::{grid_mesh, quad_mesh, tetrahedron_mesh, triangle_mesh};
pub use golden::{
    SnapshotMismatch, assert_trimesh_snapshot, render_trimesh_snapshot, trimesh_signature,
};
