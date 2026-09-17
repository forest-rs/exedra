// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Element-level provenance: which feature of which body produced each mesh
//! element.
//!
//! A [`SourceMap`] is built alongside tessellation and pinned to the mesh's
//! [`exedra_mesh::MeshRevision`]: editing the mesh afterwards invalidates the map
//! *explicitly*: [`SourceMap::check`] returns [`StaleSourceMap`]. Check the map
//! before using its lookups after a mesh edit.
//!
//! Forward lookups are O(log n) by live element ID; the reverse index
//! (feature → faces) is built once at construction and queried by binary
//! search.

use alloc::{string::String, sync::Arc, vec::Vec};

use exedra_mesh::{FaceId, Mesh, MeshRevision, VertexId};

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
    /// Approximate retained bytes across all tables. Shared source strings are
    /// conservatively counted once per referencing face.
    pub approx_bytes: usize,
}

/// Original generating surface, independent of the latest Boolean operand.
/// Placement and Boolean face splits preserve this record. A difference may
/// reverse its winding: the feature describes ancestry, not the current normal.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceOrigin {
    /// Feature on the generating body.
    pub feature: Feature,
    /// Opaque label on the generating recipe node, when authored.
    /// Wrapper labels do not rename surfaces inherited from their children.
    pub source: Option<Arc<str>>,
}

/// Per-element provenance for one tessellated body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    face_ids: Vec<FaceId>,
    face_features: Vec<Feature>,
    origins: Vec<Option<SurfaceOrigin>>,
    ambiguous_sources: Vec<String>,
    vertex_ids: Vec<VertexId>,
    vertex_features: Vec<Feature>,
    /// `(feature, face index)` sorted by feature then index: the reverse
    /// lookup table.
    by_feature: Vec<(Feature, u32)>,
    revision: MeshRevision,
}

impl SourceMap {
    /// Builds a map from features in live face/vertex iteration order,
    /// pinned to `mesh`'s current revision. Element IDs may contain gaps.
    ///
    /// # Panics
    ///
    /// Panics unless each feature vector has one entry per live element.
    #[must_use]
    pub fn new(mesh: &Mesh, face_features: Vec<Feature>, vertex_features: Vec<Feature>) -> Self {
        let face_ids: Vec<_> = mesh.faces().collect();
        let vertex_ids: Vec<_> = mesh.vertices().collect();
        assert_eq!(
            face_ids.len(),
            face_features.len(),
            "one feature per live face"
        );
        assert_eq!(
            vertex_ids.len(),
            vertex_features.len(),
            "one feature per live vertex"
        );
        let mut by_feature: Vec<(Feature, u32)> = face_features
            .iter()
            .zip(&face_ids)
            .map(|(&feature, face)| (feature, face.index()))
            .collect();
        by_feature.sort_unstable();
        Self {
            face_ids,
            origins: face_features
                .iter()
                .map(|&feature| match feature {
                    Feature::BooleanFace { .. } | Feature::BooleanSeam => None,
                    _ => Some(SurfaceOrigin {
                        feature,
                        source: None,
                    }),
                })
                .collect(),
            ambiguous_sources: Vec::new(),
            face_features,
            vertex_ids,
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

    /// The feature that produced a live face (O(log n)).
    #[must_use]
    pub fn face_feature(&self, face: FaceId) -> Option<Feature> {
        let index = self
            .face_ids
            .binary_search_by_key(&face.index(), |id| id.index())
            .ok()?;
        (self.face_ids[index] == face).then(|| self.face_features[index])
    }

    /// Original surface of a live face, when correspondence was retained.
    /// `None` means unknown ancestry; callers must not infer it from regions.
    #[must_use]
    pub fn surface_origin(&self, face: FaceId) -> Option<&SurfaceOrigin> {
        let index = self
            .face_ids
            .binary_search_by_key(&face.index(), |id| id.index())
            .ok()?;
        (self.face_ids[index] == face)
            .then(|| self.origins[index].as_ref())
            .flatten()
    }

    /// Whether a label names multiple reachable nodes in the evaluated recipe.
    #[must_use]
    pub fn source_is_ambiguous(&self, source: &str) -> bool {
        self.ambiguous_sources.iter().any(|label| label == source)
    }

    pub(crate) fn with_origins(mut self, origins: Vec<Option<SurfaceOrigin>>) -> Self {
        assert_eq!(
            origins.len(),
            self.face_ids.len(),
            "one origin entry per live face"
        );
        self.origins = origins;
        self
    }

    pub(crate) fn bind_source(&mut self, source: &str) {
        let source: Arc<str> = Arc::from(source);
        for origin in self.origins.iter_mut().flatten() {
            if origin.source.is_none() {
                origin.source = Some(Arc::clone(&source));
            }
        }
    }

    pub(crate) fn set_ambiguous_sources(&mut self, sources: &[String]) {
        self.ambiguous_sources = sources.to_vec();
    }

    /// The feature whose surface a live vertex lies on (O(log n)).
    ///
    /// A vertex generally borders several features; the recorded one is the
    /// deterministic generating feature chosen by its tessellator. Consumers
    /// must not interpret that choice as exclusive ownership at boundaries.
    #[must_use]
    pub fn vertex_feature(&self, vertex: VertexId) -> Option<Feature> {
        let index = self
            .vertex_ids
            .binary_search_by_key(&vertex.index(), |id| id.index())
            .ok()?;
        (self.vertex_ids[index] == vertex).then(|| self.vertex_features[index])
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

    /// The per-face feature table in live mesh-face iteration order.
    /// Use [`Self::face_feature`] for lookup by an element ID.
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
                + self.face_ids.len() * size_of::<FaceId>()
                + self.vertex_ids.len() * size_of::<VertexId>()
                + self.by_feature.len() * reverse
                + self.origins.len() * size_of::<Option<SurfaceOrigin>>()
                + self
                    .origins
                    .iter()
                    .flatten()
                    .filter_map(|o| o.source.as_ref())
                    .map(|s| s.len())
                    .sum::<usize>()
                + self
                    .ambiguous_sources
                    .iter()
                    .map(|s| size_of::<String>() + s.len())
                    .sum::<usize>(),
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
            face_ids: self.face_ids.clone(),
            face_features: self.face_features.clone(),
            origins: self.origins.clone(),
            ambiguous_sources: self.ambiguous_sources.clone(),
            vertex_ids: self.vertex_ids.clone(),
            vertex_features: self.vertex_features.clone(),
            by_feature: self.by_feature.clone(),
            revision: mesh.revision(),
        }
    }

    /// Renders the map as deterministic text lines for goldens: one
    /// `face <index> <feature>` line per face in index order.
    #[must_use]
    pub fn dump(&self) -> String {
        use core::fmt::Write;
        let mut out = String::new();
        for (face, feature) in self.face_ids.iter().zip(&self.face_features) {
            let _ = writeln!(out, "face {} {}", face.index(), FeatureLabel(*feature));
        }
        out
    }
}

struct FeatureLabel(Feature);

impl core::fmt::Display for FeatureLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Feature::PlanarFace => write!(f, "planar_face"),
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
            Feature::BooleanSeam => write!(f, "boolean_seam"),
            Feature::SweepWall {
                band,
                loop_index,
                seg,
            } => write!(f, "sweep_wall {band} {loop_index} {seg}"),
            Feature::GridPatch { row, col } => write!(f, "grid_patch {row} {col}"),
            Feature::StretchSeam { rim } => write!(f, "stretch_seam {rim}"),
            Feature::PlaneCutCap => write!(f, "plane_cut_cap"),
            Feature::PlaneCutSeam => write!(f, "plane_cut_seam"),
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
    fn sparse_live_ids_keep_forward_reverse_and_dump_lookups_attached() {
        let mut builder = exedra_mesh::MeshBuilder::new();
        for point in [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ] {
            builder.push_vertex(point);
        }
        builder.add_face(&[0, 1, 2]).unwrap();
        builder.add_face(&[3, 4, 5]).unwrap();
        let mut mesh = builder.build().unwrap().mesh;
        let deleted_face = mesh.faces().next().unwrap();
        let deleted_vertex = mesh.vertices().next().unwrap();
        let mut edit = mesh.edit();
        exedra_mesh::op::delete_faces(
            &mut edit,
            &[deleted_face],
            exedra_mesh::DeletePolicy::CleanupIsolated,
        )
        .unwrap();
        let _: () = edit.finish();
        let features = [Feature::Imported, Feature::CapStart, Feature::BooleanSeam];
        let map = SourceMap::new(&mesh, alloc::vec![Feature::CapEnd], features.to_vec());
        let face = mesh.faces().next().unwrap();
        assert_eq!(face.index(), 1);
        assert_eq!(map.face_feature(face), Some(Feature::CapEnd));
        assert_eq!(
            map.faces_for(Feature::CapEnd),
            [(Feature::CapEnd, face.index())]
        );
        assert_eq!(map.dump(), "face 1 cap_end\n");
        assert_eq!(map.face_feature(deleted_face), None);
        assert_eq!(map.vertex_feature(deleted_vertex), None);
        for (vertex, feature) in mesh.vertices().zip(features) {
            assert_eq!(map.vertex_feature(vertex), Some(feature));
        }
    }

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
