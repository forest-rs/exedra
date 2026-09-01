// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Element-level provenance: which feature of which body produced each mesh
//! element.
//!
//! A [`SourceMap`] is built alongside tessellation and pinned to the mesh's
//! [`exedra::MeshRevision`]: editing the mesh afterwards invalidates the map
//! *explicitly* — lookups fail with [`StaleSourceMap`] instead of silently
//! describing geometry that no longer exists.
//!
//! Forward lookups are O(1) (dense by element index); the reverse index
//! (feature → faces) is built once at construction and queried by binary
//! search.

use alloc::vec::Vec;

use exedra::{FaceId, Mesh, MeshRevision, VertexId};

use crate::tessellate::Feature;

/// The source map was built for an earlier revision of the mesh.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StaleSourceMap {
    /// Revision the map was built against.
    pub built_for: MeshRevision,
    /// The mesh's current revision.
    pub current: MeshRevision,
}

impl core::fmt::Display for StaleSourceMap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "source map is stale: built for {:?}, mesh is at {:?}",
            self.built_for, self.current
        )
    }
}

impl core::error::Error for StaleSourceMap {}

/// Size and shape of one source map.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMapStats {
    /// Dense face-feature entries.
    pub face_entries: usize,
    /// Dense vertex-feature entries.
    pub vertex_entries: usize,
    /// Reverse-index entries.
    pub reverse_entries: usize,
    /// Approximate retained bytes across all tables.
    pub approx_bytes: usize,
}

/// Per-element provenance for one tessellated body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    face_features: Vec<Feature>,
    vertex_features: Vec<Feature>,
    /// `(feature, face index)` sorted by feature then index: the reverse
    /// lookup table.
    by_feature: Vec<(Feature, u32)>,
    revision: MeshRevision,
}

impl SourceMap {
    /// Builds a map from dense per-element features, pinned to `mesh`'s
    /// current revision.
    #[must_use]
    pub fn new(mesh: &Mesh, face_features: Vec<Feature>, vertex_features: Vec<Feature>) -> Self {
        let mut by_feature: Vec<(Feature, u32)> = face_features
            .iter()
            .enumerate()
            .map(|(i, &f)| (f, crate::len_u32(i + 1) - 1))
            .collect();
        by_feature.sort_unstable();
        Self {
            face_features,
            vertex_features,
            by_feature,
            revision: mesh.revision(),
        }
    }

    /// Verifies the map still describes `mesh`.
    ///
    /// # Errors
    ///
    /// Fails with [`StaleSourceMap`] when the mesh has been edited since
    /// the map was built.
    pub fn check(&self, mesh: &Mesh) -> Result<(), StaleSourceMap> {
        let current = mesh.revision();
        if current == self.revision {
            Ok(())
        } else {
            Err(StaleSourceMap {
                built_for: self.revision,
                current,
            })
        }
    }

    /// The revision this map was built against.
    #[must_use]
    pub fn revision(&self) -> MeshRevision {
        self.revision
    }

    /// The feature that produced a face (O(1)).
    #[must_use]
    pub fn face_feature(&self, face: FaceId) -> Option<Feature> {
        self.face_features.get(face.index() as usize).copied()
    }

    /// The feature whose surface a vertex lies on (O(1)).
    ///
    /// A vertex generally borders several features; the recorded one is the
    /// deterministic generating feature chosen by its tessellator. Consumers
    /// must not interpret that choice as exclusive ownership at boundaries.
    #[must_use]
    pub fn vertex_feature(&self, vertex: VertexId) -> Option<Feature> {
        self.vertex_features.get(vertex.index() as usize).copied()
    }

    /// All face indices produced by `feature`, ascending (O(log n) + k).
    #[must_use]
    pub fn faces_for(&self, feature: Feature) -> &[(Feature, u32)] {
        let start = self.by_feature.partition_point(|(f, _)| *f < feature);
        let end = self.by_feature[start..]
            .iter()
            .position(|(f, _)| *f != feature)
            .map_or(self.by_feature.len(), |p| start + p);
        &self.by_feature[start..end]
    }

    /// Number of mapped faces.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.face_features.len()
    }

    /// The dense per-face feature table, indexed by face index.
    #[must_use]
    pub fn face_features(&self) -> &[Feature] {
        &self.face_features
    }

    /// Size and shape counters for introspection (tenet: measurable).
    #[must_use]
    pub fn stats(&self) -> SourceMapStats {
        let entry = size_of::<Feature>();
        let reverse = size_of::<(Feature, u32)>();
        SourceMapStats {
            face_entries: self.face_features.len(),
            vertex_entries: self.vertex_features.len(),
            reverse_entries: self.by_feature.len(),
            approx_bytes: self.face_features.len() * entry
                + self.vertex_features.len() * entry
                + self.by_feature.len() * reverse,
        }
    }

    /// The same map re-pinned to `mesh`'s current revision.
    ///
    /// Only valid when the mesh's topology is unchanged since the map was
    /// built (for example after pure vertex-position edits, as instancing
    /// performs); the caller asserts that by calling this.
    #[must_use]
    pub fn repinned(&self, mesh: &Mesh) -> Self {
        Self {
            face_features: self.face_features.clone(),
            vertex_features: self.vertex_features.clone(),
            by_feature: self.by_feature.clone(),
            revision: mesh.revision(),
        }
    }

    /// Renders the map as deterministic text lines for goldens: one
    /// `face <index> <feature>` line per face in index order.
    #[must_use]
    pub fn dump(&self) -> alloc::string::String {
        use core::fmt::Write;
        let mut out = alloc::string::String::new();
        for (i, feature) in self.face_features.iter().enumerate() {
            let _ = writeln!(out, "face {i} {}", FeatureLabel(*feature));
        }
        out
    }
}

struct FeatureLabel(Feature);

impl core::fmt::Display for FeatureLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Feature::CapStart => write!(f, "cap_start"),
            Feature::CapEnd => write!(f, "cap_end"),
            Feature::Wall { loop_index, seg } => write!(f, "wall {loop_index} {seg}"),
            Feature::LoftWall {
                band,
                loop_index,
                seg,
            } => write!(f, "loft_wall {band} {loop_index} {seg}"),
            Feature::Imported => write!(f, "imported"),
            Feature::PrimitiveRegion { region } => write!(f, "primitive_region {region}"),
            Feature::BooleanFace { operand } => write!(f, "boolean_face {operand}"),
            Feature::SweepWall {
                band,
                loop_index,
                seg,
            } => write!(f, "sweep_wall {band} {loop_index} {seg}"),
            Feature::GridPatch { row, col } => write!(f, "grid_patch {row} {col}"),
            Feature::StretchSeam { rim } => write!(f, "stretch_seam {rim}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::ir::{CapMode, Placement3};
    use crate::tessellate::{EvalPolicy, tessellate_extrude};

    #[test]
    fn forward_and_reverse_agree() {
        let profile = builders::l_profile(1.0, 1.0, 0.5, 0.5).expect("L");
        let body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            2.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        let map = &body.source_map;
        map.check(&body.mesh).expect("fresh map is valid");

        for face in body.mesh.faces() {
            let feature = map.face_feature(face).expect("every face is mapped");
            assert!(
                map.faces_for(feature)
                    .iter()
                    .any(|&(_, i)| i == face.index()),
                "reverse lookup must contain {face:?}"
            );
        }
        // Every reverse entry maps forward consistently.
        for feature in [
            Feature::CapStart,
            Feature::CapEnd,
            Feature::Wall {
                loop_index: 0,
                seg: 0,
            },
        ] {
            for &(f, index) in map.faces_for(feature) {
                assert_eq!(f, feature);
                let face = body
                    .mesh
                    .faces()
                    .find(|face| face.index() == index)
                    .expect("face exists");
                assert_eq!(map.face_feature(face), Some(feature));
            }
        }
    }

    #[test]
    fn stale_maps_are_rejected() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let mut body = tessellate_extrude(
            &profile,
            &Placement3::IDENTITY,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("tessellates");
        body.source_map.check(&body.mesh).expect("fresh");
        // Any edit bumps the revision and orphans the map.
        let session = body.mesh.edit();
        #[expect(unused_must_use, reason = "the unit sink output is irrelevant here")]
        {
            session.finish();
        }
        assert!(body.source_map.check(&body.mesh).is_err());
    }

    #[test]
    fn dump_is_deterministic() {
        let profile = builders::rect(1.0, 1.0).expect("rect");
        let make = || {
            tessellate_extrude(
                &profile,
                &Placement3::IDENTITY,
                1.0,
                CapMode::Both,
                &EvalPolicy::default(),
            )
            .expect("tessellates")
        };
        let a = make();
        let b = make();
        assert_eq!(a.source_map.dump(), b.source_map.dump());
        assert!(a.source_map.dump().starts_with("face 0 wall 0 0\n"));
    }
}
