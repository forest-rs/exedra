// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded deterministic artifact payloads for operator introspection.

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use exedra::{CornerId, Domain, FaceId, HalfEdgeId, Mesh};

/// Default artifact item cap for [`Artifacts`].
pub const DEFAULT_MAX_ARTIFACT_ITEMS: usize = 16;
/// Default artifact byte cap for [`Artifacts`] (v0.1 estimate model).
pub const DEFAULT_MAX_ARTIFACT_BYTES: usize = 1 << 20;

/// One operator-emitted debug artifact.
#[derive(Clone, Debug)]
pub enum Artifact {
    /// Full mesh snapshot.
    Mesh {
        /// Artifact label.
        name: String,
        /// Mesh payload.
        mesh: Mesh,
    },
    /// Face ID set.
    FaceSet {
        /// Artifact label.
        name: String,
        /// Retained face IDs.
        faces: Vec<FaceId>,
    },
    /// Half-edge ID set.
    EdgeSet {
        /// Artifact label.
        name: String,
        /// Retained half-edge IDs.
        half_edges: Vec<HalfEdgeId>,
    },
    /// Corner ID set.
    CornerSet {
        /// Artifact label.
        name: String,
        /// Retained corner IDs.
        corners: Vec<CornerId>,
    },
    /// 3D polyline.
    Polyline3 {
        /// Artifact label.
        name: String,
        /// Polyline points.
        points: Vec<[f32; 3]>,
    },
    /// 2D polyline.
    Polyline2 {
        /// Artifact label.
        name: String,
        /// Polyline points.
        points: Vec<[f32; 2]>,
    },
    /// Scalar field over a domain.
    FieldF32 {
        /// Artifact label.
        name: String,
        /// Domain of field values.
        domain: Domain,
        /// Scalar field values.
        values: Vec<f32>,
    },
}

impl Artifact {
    /// Returns artifact name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Mesh { name, .. }
            | Self::FaceSet { name, .. }
            | Self::EdgeSet { name, .. }
            | Self::CornerSet { name, .. }
            | Self::Polyline3 { name, .. }
            | Self::Polyline2 { name, .. }
            | Self::FieldF32 { name, .. } => name,
        }
    }

    /// Returns v0.1 byte estimate used for bounded retention.
    #[must_use]
    pub fn estimate_bytes(&self) -> usize {
        match self {
            // v0.1 posture: mesh payload size estimation is deferred.
            Self::Mesh { .. } => 0,
            Self::FaceSet { faces, .. } => faces.len() * size_of::<FaceId>(),
            Self::EdgeSet { half_edges, .. } => half_edges.len() * size_of::<HalfEdgeId>(),
            Self::CornerSet { corners, .. } => corners.len() * size_of::<CornerId>(),
            Self::Polyline3 { points, .. } => points.len() * size_of::<[f32; 3]>(),
            Self::Polyline2 { points, .. } => points.len() * size_of::<[f32; 2]>(),
            Self::FieldF32 { values, .. } => values.len() * size_of::<f32>(),
        }
    }
}

/// Bounded artifact list with deterministic overflow policy.
///
/// On overflow (item count or bytes), new entries are dropped. Earlier entries
/// are always retained.
#[derive(Clone, Debug)]
pub struct Artifacts {
    items: Vec<Artifact>,
    max_items: usize,
    max_bytes: usize,
    used_bytes: usize,
}

impl Default for Artifacts {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ARTIFACT_ITEMS, DEFAULT_MAX_ARTIFACT_BYTES)
    }
}

impl Artifacts {
    /// Creates a bounded artifact container.
    #[must_use]
    pub fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            items: Vec::new(),
            max_items,
            max_bytes,
            used_bytes: 0,
        }
    }

    /// Returns retained artifact count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns true when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns retained byte estimate.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    /// Returns retained artifacts in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Artifact> {
        self.items.iter()
    }

    /// Attempts to push one artifact.
    ///
    /// Returns `true` when retained and `false` when deterministically dropped.
    pub fn push(&mut self, artifact: Artifact) -> bool {
        if self.items.len() >= self.max_items {
            return false;
        }
        let estimate = artifact.estimate_bytes();
        let Some(next_bytes) = self.used_bytes.checked_add(estimate) else {
            return false;
        };
        if next_bytes > self.max_bytes {
            return false;
        }
        self.items.push(artifact);
        self.used_bytes = next_bytes;
        true
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroU32;

    use exedra::{Domain, FaceId, Id, Mesh};

    use super::{Artifact, Artifacts, DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_ARTIFACT_ITEMS};

    #[test]
    fn item_limit_keeps_earliest_artifacts() {
        let mut artifacts = Artifacts::new(2, usize::MAX);
        assert!(artifacts.push(Artifact::FieldF32 {
            name: "a".to_string(),
            domain: Domain::Face,
            values: vec![1.0],
        }));
        assert!(artifacts.push(Artifact::FieldF32 {
            name: "b".to_string(),
            domain: Domain::Face,
            values: vec![2.0],
        }));
        assert!(!artifacts.push(Artifact::FieldF32 {
            name: "c".to_string(),
            domain: Domain::Face,
            values: vec![3.0],
        }));

        let names = artifacts.iter().map(Artifact::name).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn byte_limit_enforces_v0_1_accounting() {
        let mut artifacts = Artifacts::new(4, 8);
        assert!(artifacts.push(Artifact::FieldF32 {
            name: "small".to_string(),
            domain: Domain::Face,
            values: vec![1.0, 2.0],
        }));
        assert_eq!(artifacts.used_bytes(), 8);
        assert!(!artifacts.push(Artifact::FieldF32 {
            name: "too_big".to_string(),
            domain: Domain::Face,
            values: vec![3.0],
        }));
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts.used_bytes(), 8);
    }

    #[test]
    fn mesh_variant_counts_as_zero_bytes_in_v0_1() {
        let mut artifacts = Artifacts::new(2, 0);
        assert!(artifacts.push(Artifact::Mesh {
            name: "snapshot".to_string(),
            mesh: Mesh::new(),
        }));
        assert_eq!(artifacts.used_bytes(), 0);
    }

    #[test]
    fn id_set_accounting_uses_element_size() {
        let mut artifacts = Artifacts::new(3, usize::MAX);
        let faces = (0_u32..3)
            .map(|i| FaceId::from(Id::new(i, NonZeroU32::MIN)))
            .collect::<Vec<_>>();
        assert!(artifacts.push(Artifact::FaceSet {
            name: "faces".to_string(),
            faces: faces.clone(),
        }));
        let expected = faces.len() * size_of::<FaceId>();
        assert_eq!(artifacts.used_bytes(), expected);
    }

    #[test]
    fn insertion_order_is_preserved() {
        let mut artifacts = Artifacts::new(8, usize::MAX);
        for i in 0..4 {
            assert!(artifacts.push(Artifact::Polyline2 {
                name: format!("line-{i}"),
                points: vec![[i as f32, i as f32]],
            }));
        }
        let names = artifacts.iter().map(Artifact::name).collect::<Vec<_>>();
        assert_eq!(names, vec!["line-0", "line-1", "line-2", "line-3"]);
    }

    #[test]
    fn default_uses_non_zero_limits() {
        let artifacts = Artifacts::default();
        assert_eq!(artifacts.max_items, DEFAULT_MAX_ARTIFACT_ITEMS);
        assert_eq!(artifacts.max_bytes, DEFAULT_MAX_ARTIFACT_BYTES);
    }
}
