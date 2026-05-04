// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test helpers for Exedra fixtures, snapshots, and debug dumps.

#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod dump;
pub mod fixtures;
pub mod golden;

pub use dump::{dump_attributes, dump_mesh_topology, mesh_to_obj};
pub use fixtures::{
    capped_cylinder_8, grid_mesh, quad_mesh, sphere_mesh, tetrahedron_mesh, triangle_mesh,
    unit_box, unit_quad, uv_sphere_8,
};
#[cfg(feature = "std")]
pub use golden::{GoldenFileError, assert_golden};
pub use golden::{
    GoldenMismatch, GoldenSelection, SnapshotMismatch, assert_mesh_golden, assert_trimesh_snapshot,
    dump_golden, dump_golden_with_selections, render_trimesh_snapshot, trimesh_signature,
};
