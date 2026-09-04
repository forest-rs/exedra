// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic operator plans, source-state bindings, and signatures.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use exedra::{FaceId, HalfEdgeId, Mesh, MeshRevision, attr};

/// Deterministic fingerprint for a compiled [`EditPlan`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PlanFingerprint(u64);

impl PlanFingerprint {
    /// Creates a fingerprint from a precomputed 64-bit value.
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw 64-bit value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Deterministic compiled plan for one operator invocation.
#[derive(Clone, Debug)]
pub struct EditPlan<P> {
    /// Stable operator identifier (for example `edit.delete.faces`).
    pub operator: &'static str,
    /// Mesh state the plan was compiled against.
    pub source_state: PlanSourceState,
    /// Deterministic plan fingerprint.
    pub fingerprint: PlanFingerprint,
    /// Operator-specific compiled payload.
    pub payload: P,
}

/// Mesh state binding captured when compiling an [`EditPlan`].
///
/// Plans are intentionally bound to the mesh state they were compiled from so
/// runners can reject stale or drifted plans before mutation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PlanSourceState {
    /// Monotonic mesh revision observed at compile time.
    pub revision: MeshRevision,
    /// Deterministic full mesh-state signature observed at compile time.
    pub mesh_state_signature: u64,
}

impl PlanSourceState {
    /// Captures the current source state for `mesh`.
    #[must_use]
    pub fn capture(mesh: &Mesh) -> Self {
        Self {
            revision: mesh.revision(),
            mesh_state_signature: mesh_signature(mesh),
        }
    }
}

/// Deterministic FNV-1a hasher used for plan fingerprinting.
#[derive(Clone, Debug)]
pub(crate) struct PlanHasher {
    state: u64,
}

impl PlanHasher {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0001_0000_01b3;

    /// Creates a new hasher.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            state: Self::OFFSET_BASIS,
        }
    }

    /// Appends raw bytes.
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    /// Appends a UTF-8 string.
    pub(crate) fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
        self.write_u8(0xff);
    }

    /// Appends a `u8`.
    pub(crate) fn write_u8(&mut self, value: u8) {
        self.write_bytes(&[value]);
    }

    /// Appends a `u32`.
    pub(crate) fn write_u32(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Appends a `u64`.
    pub(crate) fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// Appends an `f32` by exact bit pattern.
    pub(crate) fn write_f32_bits(&mut self, value: f32) {
        self.write_u32(value.to_bits());
    }

    /// Appends a `usize`.
    pub(crate) fn write_len(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    /// Appends a face selection (must already be in stable order).
    pub(crate) fn write_face_set(&mut self, faces: &[FaceId]) {
        self.write_len(faces.len());
        for &face in faces {
            self.write_u32(face.index());
        }
    }

    /// Finalizes into a [`PlanFingerprint`].
    #[must_use]
    pub(crate) const fn finish(self) -> PlanFingerprint {
        PlanFingerprint::from_u64(self.state)
    }
}

impl Default for PlanHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable topology-only mesh signature used by determinism tests and caches
/// that intentionally ignore authored attributes.
///
/// This intentionally walks live topology in deterministic ID/loop order.
#[must_use]
pub fn mesh_topology_signature(mesh: &Mesh) -> u64 {
    let mut hasher = PlanHasher::new();
    hasher.write_str("mesh-topology-signature/v1");

    let mut vertices: Vec<_> = mesh.vertices().collect();
    vertices.sort_unstable();
    for vertex in vertices {
        hasher.write_u32(vertex.index());
        if let Some(position) = mesh.vertex_position(vertex) {
            hasher.write_f32_bits(position[0]);
            hasher.write_f32_bits(position[1]);
            hasher.write_f32_bits(position[2]);
        } else {
            hasher.write_u8(0xfe);
        }
    }

    let mut faces: Vec<_> = mesh.faces().collect();
    faces.sort_unstable();
    for face in faces {
        hasher.write_u32(face.index());
        let loop_edges: Vec<_> = mesh.face_loop(face).collect();
        hasher.write_len(loop_edges.len());
        for edge in loop_edges {
            hasher.write_u32(edge.index());
        }
    }

    hasher.finish().value()
}

/// Stable full mesh-state signature used by plan binding and state-equivalence
/// tests.
///
/// This extends [`mesh_topology_signature`] with built-in authored attributes
/// that materially affect mesh semantics:
/// - [`attr::FACE_REGION`]
/// - [`attr::CORNER_UV`]
/// - [`attr::CORNER_NORMAL_OVERRIDE`]
/// - [`attr::EDGE_SEAM`]
/// - [`attr::EDGE_SHARPNESS`]
#[must_use]
pub fn mesh_signature(mesh: &Mesh) -> u64 {
    let mut hasher = PlanHasher::new();
    hasher.write_str("mesh-signature/v2");
    hasher.write_u64(mesh_topology_signature(mesh));

    let face_regions = mesh.attrs().dense(attr::FACE_REGION);
    let mut faces: Vec<_> = mesh.faces().collect();
    faces.sort_unstable();
    for face in &faces {
        hasher.write_u32(face.index());
        hasher.write_u32(
            face_regions
                .and_then(|layer| layer.get(face.as_id()).copied())
                .unwrap_or(0),
        );
    }

    let corner_uvs = mesh.attrs().sparse(attr::CORNER_UV);
    let corner_normals = mesh.attrs().sparse(attr::CORNER_NORMAL_OVERRIDE);
    for face in &faces {
        for corner in mesh.face_loop(*face) {
            hasher.write_u32(corner.index());
            write_optional_vec2(
                &mut hasher,
                corner_uvs.and_then(|layer| layer.get(corner.as_id()).copied()),
            );
            write_optional_vec3(
                &mut hasher,
                corner_normals.and_then(|layer| layer.get(corner.as_id()).copied()),
            );
        }
    }

    let mut canonical_edges = BTreeSet::<HalfEdgeId>::new();
    for face in faces {
        for edge in mesh.face_loop(face) {
            let canonical = mesh
                .canonical_edge(edge)
                .expect("face loop half-edge should resolve to a canonical edge");
            let _ = canonical_edges.insert(canonical);
        }
    }
    for edge in canonical_edges {
        hasher.write_u32(edge.index());
        hasher.write_u8(u8::from(
            mesh.edge_seam(edge)
                .expect("canonical edge should have readable seam state"),
        ));
        hasher.write_f32_bits(
            mesh.edge_sharpness(edge)
                .expect("canonical edge should have readable sharpness state"),
        );
    }

    hasher.finish().value()
}

fn write_optional_vec2(hasher: &mut PlanHasher, value: Option<[f32; 2]>) {
    match value {
        Some([x, y]) => {
            hasher.write_u8(1);
            hasher.write_f32_bits(x);
            hasher.write_f32_bits(y);
        }
        None => hasher.write_u8(0),
    }
}

fn write_optional_vec3(hasher: &mut PlanHasher, value: Option<[f32; 3]>) {
    match value {
        Some([x, y, z]) => {
            hasher.write_u8(1);
            hasher.write_f32_bits(x);
            hasher.write_f32_bits(y);
            hasher.write_f32_bits(z);
        }
        None => hasher.write_u8(0),
    }
}

#[cfg(test)]
mod tests {
    use exedra::{Mesh, op};

    use super::{mesh_signature, mesh_topology_signature};

    fn quad_mesh() -> Mesh {
        Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .expect("quad mesh should build")
    }

    #[test]
    fn mesh_signature_changes_when_face_region_changes() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let topology_before = mesh_topology_signature(&mesh);
        let signature_before = mesh_signature(&mesh);

        let mut edit = mesh.edit();
        op::set_face_region(&mut edit, face, 7).expect("face region write should succeed");
        let _: () = edit.finish();

        assert_eq!(mesh_topology_signature(&mesh), topology_before);
        assert_ne!(mesh_signature(&mesh), signature_before);
    }

    #[test]
    fn mesh_signature_changes_when_corner_uv_changes() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");
        let topology_before = mesh_topology_signature(&mesh);
        let signature_before = mesh_signature(&mesh);

        let mut edit = mesh.edit();
        op::set_corner_uv(&mut edit, corner, [0.25, 0.75]).expect("corner uv write should work");
        let _: () = edit.finish();

        assert_eq!(mesh_topology_signature(&mesh), topology_before);
        assert_ne!(mesh_signature(&mesh), signature_before);
    }

    #[test]
    fn mesh_signature_changes_when_corner_normal_override_changes() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let corner = mesh.face_loop(face).next().expect("corner should exist");
        let topology_before = mesh_topology_signature(&mesh);
        let signature_before = mesh_signature(&mesh);

        let mut edit = mesh.edit();
        op::set_corner_normal_override(&mut edit, corner, Some([0.0, 0.0, 1.0]))
            .expect("corner normal write should work");
        let _: () = edit.finish();

        assert_eq!(mesh_topology_signature(&mesh), topology_before);
        assert_ne!(mesh_signature(&mesh), signature_before);
    }

    #[test]
    fn mesh_signature_changes_when_edge_attributes_change() {
        let mut mesh = quad_mesh();
        let face = mesh.faces().next().expect("face should exist");
        let edge = mesh.face_loop(face).next().expect("edge should exist");
        let topology_before = mesh_topology_signature(&mesh);
        let signature_before = mesh_signature(&mesh);

        let mut edit = mesh.edit();
        op::set_edge_seam(&mut edit, edge, true).expect("edge seam write should work");
        op::set_edge_sharpness(&mut edit, edge, 2.5).expect("edge sharpness write should work");
        let _: () = edit.finish();

        assert_eq!(mesh_topology_signature(&mesh), topology_before);
        assert_ne!(mesh_signature(&mesh), signature_before);
    }
}
