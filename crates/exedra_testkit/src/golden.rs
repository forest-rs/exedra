// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic `TriMesh` snapshot rendering and comparison helpers.

use alloc::format;
use alloc::string::String;
use core::fmt;

use exedra::TriMesh;

/// Snapshot mismatch payload returned by [`assert_trimesh_snapshot`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotMismatch {
    /// Expected text snapshot.
    pub expected: String,
    /// Actual rendered text snapshot.
    pub actual: String,
}

impl fmt::Display for SnapshotMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TriMesh snapshot mismatch")
    }
}

impl core::error::Error for SnapshotMismatch {}

/// Renders a deterministic text snapshot for a [`TriMesh`].
///
/// Floats are encoded via IEEE-754 bit patterns to avoid formatting drift.
#[must_use]
pub fn render_trimesh_snapshot(mesh: &TriMesh) -> String {
    let mut out = String::new();
    out.push_str("exedra-trimesh-v1\n");

    out.push_str("positions ");
    out.push_str(&format!("{}\n", mesh.positions.len()));
    for (index, position) in mesh.positions.iter().enumerate() {
        out.push_str(&format!(
            "p {index} {:08x} {:08x} {:08x}\n",
            position[0].to_bits(),
            position[1].to_bits(),
            position[2].to_bits()
        ));
    }

    out.push_str("uvs ");
    out.push_str(&format!("{}\n", mesh.uvs.len()));
    for (index, uv) in mesh.uvs.iter().enumerate() {
        out.push_str(&format!(
            "uv {index} {:08x} {:08x}\n",
            uv[0].to_bits(),
            uv[1].to_bits()
        ));
    }

    out.push_str("normals ");
    out.push_str(&format!("{}\n", mesh.normals.len()));
    for (index, normal) in mesh.normals.iter().enumerate() {
        out.push_str(&format!(
            "n {index} {:08x} {:08x} {:08x}\n",
            normal[0].to_bits(),
            normal[1].to_bits(),
            normal[2].to_bits()
        ));
    }

    out.push_str("indices ");
    out.push_str(&format!("{}\n", mesh.indices.len()));
    for (index, value) in mesh.indices.iter().enumerate() {
        out.push_str(&format!("i {index} {value}\n"));
    }

    out
}

/// Returns a compact deterministic signature for quick equivalence checks.
#[must_use]
pub fn trimesh_signature(mesh: &TriMesh) -> u64 {
    let snapshot = render_trimesh_snapshot(mesh);
    let mut hash = 14_695_981_039_346_656_037_u64; // FNV-1a offset basis
    for byte in snapshot.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211_u64); // FNV-1a prime
    }
    hash
}

/// Compares a [`TriMesh`] against an expected deterministic snapshot.
pub fn assert_trimesh_snapshot(mesh: &TriMesh, expected: &str) -> Result<(), SnapshotMismatch> {
    let actual = render_trimesh_snapshot(mesh);
    if actual == expected {
        Ok(())
    } else {
        Err(SnapshotMismatch {
            expected: expected.into(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use exedra::{BuildParams, ExtractParams, Mesh};

    use super::{assert_trimesh_snapshot, render_trimesh_snapshot, trimesh_signature};

    #[test]
    fn snapshot_helpers_are_deterministic() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
        let first = render_trimesh_snapshot(&tri);
        let second = render_trimesh_snapshot(&tri);
        assert_eq!(first, second);
        assert_eq!(trimesh_signature(&tri), trimesh_signature(&tri));
    }

    #[test]
    fn snapshot_comparison_reports_mismatch() {
        let mesh = Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .expect("triangle build should succeed");
        let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
        let expected = "exedra-trimesh-v1\ninvalid\n";
        let mismatch = assert_trimesh_snapshot(&tri, expected).expect_err("comparison should fail");
        assert_eq!(mismatch.expected, expected.to_string());
        assert!(mismatch.actual.starts_with("exedra-trimesh-v1\npositions "));
    }
}
