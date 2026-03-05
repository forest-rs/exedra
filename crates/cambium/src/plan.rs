// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic operator plans and fingerprints.

use alloc::vec::Vec;

use exedra::{FaceId, Mesh};

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
    /// Deterministic plan fingerprint.
    pub fingerprint: PlanFingerprint,
    /// Operator-specific compiled payload.
    pub payload: P,
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

/// Stable mesh signature used by determinism tests.
///
/// This intentionally walks topology and attributes in sorted ID order so test
/// cases can assert "identical mesh state" before comparing compiled plans.
#[must_use]
pub fn mesh_signature(mesh: &Mesh) -> u64 {
    let mut hasher = PlanHasher::new();
    hasher.write_str("mesh-signature/v1");

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
        let mut loop_edges: Vec<_> = mesh.face_loop(face).collect();
        loop_edges.sort_unstable();
        hasher.write_len(loop_edges.len());
        for edge in loop_edges {
            hasher.write_u32(edge.index());
        }
    }

    hasher.finish().value()
}
