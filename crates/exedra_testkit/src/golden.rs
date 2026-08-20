// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic `TriMesh` snapshot rendering and comparison helpers.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use core::fmt;

use exedra::{FaceId, HalfEdgeId, Mesh, TriMesh};

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

/// Mesh golden mismatch payload returned by [`assert_mesh_golden`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoldenMismatch {
    /// Expected mesh golden text.
    pub expected: String,
    /// Actual rendered mesh golden text.
    pub actual: String,
}

impl fmt::Display for GoldenMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mesh golden mismatch")
    }
}

impl core::error::Error for GoldenMismatch {}

/// One named selection emitted by [`dump_golden_with_selections`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct GoldenSelection<'a> {
    /// Stable selection name.
    pub name: &'a str,
    /// Selected faces.
    pub faces: &'a [FaceId],
    /// Selected canonical or caller-chosen half-edges.
    pub edges: &'a [HalfEdgeId],
}

/// File-backed mesh golden assertion error.
#[cfg(feature = "std")]
#[derive(Debug)]
pub enum GoldenFileError {
    /// Reading the expected golden file failed.
    Io(std::io::Error),
    /// Expected and actual mesh golden text differed.
    Mismatch(GoldenMismatch),
}

#[cfg(feature = "std")]
impl fmt::Display for GoldenFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "mesh golden file I/O failed: {error}"),
            Self::Mismatch(error) => error.fmt(f),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for GoldenFileError {}

#[cfg(feature = "std")]
impl From<std::io::Error> for GoldenFileError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Renders a deterministic human-readable mesh golden dump.
///
/// The format is intentionally line-oriented and ordered:
/// vertices/faces/attributes are emitted in stable mesh traversal order, floats
/// are encoded as IEEE-754 bit patterns, and collections are represented as
/// lists rather than maps.
#[must_use]
pub fn dump_golden(mesh: &Mesh) -> String {
    dump_golden_with_selections(mesh, &[])
}

/// Renders a deterministic human-readable mesh golden dump with selections.
///
/// Selection IDs are copied, sorted, and deduplicated before emission so caller
/// ordering cannot destabilize the golden output.
#[must_use]
pub fn dump_golden_with_selections(mesh: &Mesh, selections: &[GoldenSelection<'_>]) -> String {
    let mut out = String::new();
    out.push_str("exedra-mesh-golden-v1\n");
    out.push_str(&format!("vertices {}\n", mesh.vertices().count()));
    out.push_str("positions [\n");
    for vertex in mesh.vertices() {
        let position = mesh
            .vertex_position(vertex)
            .expect("live fixture vertex must have a position");
        out.push_str(&format!(
            "  ({}, {:08x}, {:08x}, {:08x}),\n",
            vertex.index(),
            position[0].to_bits(),
            position[1].to_bits(),
            position[2].to_bits()
        ));
    }
    out.push_str("]\n");

    out.push_str(&format!("faces {}\n", mesh.faces().count()));
    out.push_str("face_loops [\n");
    for face in mesh.faces() {
        out.push_str(&format!("  ({}, [", face.index()));
        for (index, corner) in mesh.face_loop(face).enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            let vertex = mesh
                .from_vertex(corner)
                .expect("face-loop corner must have a source vertex");
            out.push_str(&format!("{}", vertex.index()));
        }
        out.push_str("]),\n");
    }
    out.push_str("]\n");

    dump_mesh_attributes(mesh, &mut out);
    dump_selections(selections, &mut out);
    out
}

/// Compares a mesh against expected deterministic golden text.
pub fn assert_mesh_golden(mesh: &Mesh, expected: &str) -> Result<(), GoldenMismatch> {
    let actual = dump_golden(mesh);
    if actual == expected {
        Ok(())
    } else {
        Err(GoldenMismatch {
            expected: expected.into(),
            actual,
        })
    }
}

/// Reads a golden file from `expected_path` and compares it to `mesh`.
#[cfg(feature = "std")]
pub fn assert_golden(
    mesh: &Mesh,
    expected_path: impl AsRef<std::path::Path>,
) -> Result<(), GoldenFileError> {
    let expected = std::fs::read_to_string(expected_path)?;
    assert_mesh_golden(mesh, &expected).map_err(GoldenFileError::Mismatch)
}

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

fn dump_mesh_attributes(mesh: &Mesh, out: &mut String) {
    out.push_str("attributes [\n");

    out.push_str("  vertex.sharpness [");
    if let Some(layer) = mesh.attrs().sparse(exedra::attr::VERTEX_SHARPNESS) {
        let mut first = true;
        for vertex in mesh.vertices() {
            if let Some(sharpness) = layer.get(vertex.as_id()) {
                push_comma(&mut first, out);
                out.push_str(&format!(
                    "({}, {:08x})",
                    vertex.index(),
                    sharpness.to_bits()
                ));
            }
        }
    }
    out.push_str("]\n");

    out.push_str("  face.region [");
    if let Some(layer) = mesh.attrs().dense(exedra::attr::FACE_REGION) {
        let mut first = true;
        for face in mesh.faces() {
            if let Some(region) = layer.get(face.as_id()) {
                push_comma(&mut first, out);
                out.push_str(&format!("({}, {})", face.index(), region));
            }
        }
    }
    out.push_str("]\n");

    out.push_str("  corner.uv [");
    if let Some(layer) = mesh.attrs().sparse(exedra::attr::CORNER_UV) {
        let mut first = true;
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                if let Some(uv) = layer.get(corner.as_id()) {
                    push_comma(&mut first, out);
                    out.push_str(&format!(
                        "({}, {:08x}, {:08x})",
                        corner.index(),
                        uv[0].to_bits(),
                        uv[1].to_bits()
                    ));
                }
            }
        }
    }
    out.push_str("]\n");

    out.push_str("  corner.normal_override [");
    if let Some(layer) = mesh.attrs().sparse(exedra::attr::CORNER_NORMAL_OVERRIDE) {
        let mut first = true;
        for face in mesh.faces() {
            for corner in mesh.face_loop(face) {
                if let Some(normal) = layer.get(corner.as_id()) {
                    push_comma(&mut first, out);
                    out.push_str(&format!(
                        "({}, {:08x}, {:08x}, {:08x})",
                        corner.index(),
                        normal[0].to_bits(),
                        normal[1].to_bits(),
                        normal[2].to_bits()
                    ));
                }
            }
        }
    }
    out.push_str("]\n");

    let canonical_edges = canonical_edges(mesh);
    out.push_str("  edge.seam [");
    let mut first = true;
    for edge in &canonical_edges {
        if mesh.edge_seam(*edge) == Some(true) {
            push_comma(&mut first, out);
            out.push_str(&format!("{}", edge.index()));
        }
    }
    out.push_str("]\n");

    out.push_str("  edge.sharpness [");
    let mut first = true;
    for edge in canonical_edges {
        let Some(sharpness) = mesh.edge_sharpness(edge) else {
            continue;
        };
        if sharpness.to_bits() == 0.0_f32.to_bits() {
            continue;
        }
        push_comma(&mut first, out);
        out.push_str(&format!("({}, {:08x})", edge.index(), sharpness.to_bits()));
    }
    out.push_str("]\n");

    out.push_str("]\n");
}

fn dump_selections(selections: &[GoldenSelection<'_>], out: &mut String) {
    out.push_str("selections [\n");
    for selection in selections {
        let mut faces = selection.faces.to_vec();
        faces.sort_unstable();
        faces.dedup();
        let mut edges = selection.edges.to_vec();
        edges.sort_unstable();
        edges.dedup();

        out.push_str(&format!("  ({:?}, faces [", selection.name));
        for (index, face) in faces.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{}", face.index()));
        }
        out.push_str("], edges [");
        for (index, edge) in edges.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{}", edge.index()));
        }
        out.push_str("]),\n");
    }
    out.push_str("]\n");
}

fn canonical_edges(mesh: &Mesh) -> BTreeSet<HalfEdgeId> {
    let mut edges = BTreeSet::new();
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            if let Some(canonical) = mesh.canonical_edge(corner) {
                let _ = edges.insert(canonical);
            }
        }
    }
    edges
}

fn push_comma(first: &mut bool, out: &mut String) {
    if *first {
        *first = false;
    } else {
        out.push_str(", ");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(all(feature = "std", not(target_os = "wasi")))]
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    use exedra::{BuildParams, ExtractParams, Mesh};

    use super::{
        GoldenSelection, assert_mesh_golden, assert_trimesh_snapshot, dump_golden,
        dump_golden_with_selections, render_trimesh_snapshot, trimesh_signature,
    };
    use crate::fixtures::unit_quad;

    #[test]
    fn mesh_golden_dump_is_deterministic_and_diffable() {
        let mesh = unit_quad();
        let first = dump_golden(&mesh);
        let second = dump_golden(&mesh);
        assert_eq!(first, second);
        assert!(first.starts_with("exedra-mesh-golden-v1\n"));
        assert!(first.contains("positions [\n"));
        assert!(first.contains("face_loops [\n"));
        assert!(first.contains("attributes [\n"));
        assert_mesh_golden(&mesh, &first).expect("matching golden should pass");
    }

    #[test]
    fn mesh_golden_dump_can_include_ordered_selections() {
        let mesh = unit_quad();
        let face = mesh.faces().next().expect("quad face should exist");
        let edges = mesh.face_loop(face).collect::<Vec<_>>();
        let dump = dump_golden_with_selections(
            &mesh,
            &[GoldenSelection {
                name: "quad.boundary",
                faces: &[face],
                edges: &[edges[2], edges[0], edges[2]],
            }],
        );

        assert!(dump.contains("(\"quad.boundary\", faces [0], edges [0, 2]),"));
    }

    #[test]
    fn mesh_golden_comparison_reports_mismatch() {
        let mesh = unit_quad();
        let mismatch = assert_mesh_golden(&mesh, "exedra-mesh-golden-v1\ninvalid\n")
            .expect_err("comparison should fail");
        assert!(mismatch.actual.contains("face_loops [\n"));
    }

    #[cfg(all(feature = "std", not(target_os = "wasi")))]
    #[test]
    fn file_backed_mesh_golden_assertion_reads_expected_file() {
        let mesh = unit_quad();
        let expected = dump_golden(&mesh);
        let path =
            std::env::temp_dir().join(format!("exedra-testkit-golden-{}.txt", std::process::id()));
        std::fs::write(&path, expected).expect("write golden file");
        super::assert_golden(&mesh, &path).expect("file-backed golden should match");
        let _ = std::fs::remove_file(path);
    }

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
