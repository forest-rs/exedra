// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive attribution for direct mesh stretch.

use super::{MeshStretchStats, StretchRefusal};
use crate::ir::{Placement3, Plane3};
use crate::tessellate::{EvalPolicy, Feature, TessellatedBody};
use exedra_mesh_ops::stretch::{self, StretchError, StretchPolicy, StretchVertexSource};

pub(crate) fn stretch_mesh(
    source: &TessellatedBody,
    plane: &Plane3,
    length: f64,
    world: &Placement3,
    policy: &EvalPolicy,
) -> Result<(TessellatedBody, MeshStretchStats), StretchRefusal> {
    source
        .source_map
        .check(&source.mesh)
        .map_err(|_| StretchRefusal::BuildFailed)?;
    let result = stretch::stretch_mesh(
        &source.mesh,
        plane,
        length,
        world,
        &StretchPolicy {
            sharp_sin_threshold: policy.sharp_sin_threshold,
        },
    )
    .map_err(StretchRefusal::from)?;
    let source_map = if result.topology_rebuilt {
        crate::source_map::SourceMap::new(
            &result.mesh,
            result
                .mesh
                .faces()
                .map(|face| {
                    source
                        .source_map
                        .face_feature(result.face_sources[&face].face())
                        .unwrap_or(Feature::Imported)
                })
                .collect(),
            result
                .mesh
                .vertices()
                .map(|vertex| match result.vertex_sources[&vertex] {
                    StretchVertexSource::Original(source_vertex) => source
                        .source_map
                        .vertex_feature(source_vertex)
                        .unwrap_or(Feature::Imported),
                    StretchVertexSource::Seam { rim } => Feature::StretchSeam { rim },
                })
                .collect(),
        )
    } else {
        source.source_map.repinned(&result.mesh)
    };
    let face_materials = result
        .face_sources
        .iter()
        .filter_map(|(&face, from)| {
            source
                .face_materials
                .get(&from.face())
                .map(|&slot| (face, slot))
        })
        .collect();
    Ok((
        TessellatedBody {
            mesh: result.mesh,
            source_map,
            face_materials,
            sweep_checks: None,
            path_sampling: None,
            loft_sampling: None,
            refinement: None,
        },
        result.stats,
    ))
}

impl From<StretchError> for StretchRefusal {
    fn from(error: StretchError) -> Self {
        match error {
            StretchError::InvalidInput
            | StretchError::InvalidMesh
            | StretchError::NumericLimit
            | StretchError::BuildFailed => Self::BuildFailed,
            StretchError::ContractionConsumesHalf => Self::ContractionConsumesHalf,
            StretchError::SingularTransform => Self::SingularTransform,
            StretchError::AmbiguousContact => Self::AmbiguousContact,
            StretchError::DisconnectedFaceSection => Self::DisconnectedFaceSection,
            StretchError::OpenShell => Self::OpenShell,
            StretchError::NonManifoldSection => Self::NonManifoldSection,
            StretchError::IncompatibleContraction => Self::IncompatibleContraction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builders,
        ir::{CapMode, SlotId},
        tessellate::tessellate_extrude,
    };

    #[test]
    fn direct_stretch_and_constructive_binding_agree_on_geometry_materials_and_features() {
        let policy = EvalPolicy::default();
        let mut source = tessellate_extrude(
            &builders::rect_from_corner(4.0, 3.0).unwrap(),
            &Placement3::IDENTITY,
            2.0,
            CapMode::Both,
            &policy,
        )
        .unwrap();
        source.face_materials = source
            .mesh
            .faces()
            .map(|face| (face, SlotId(face.index())))
            .collect();
        let plane = Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 1.3,
        };
        let direct = stretch::stretch_mesh(
            &source.mesh,
            &plane,
            1.0,
            &Placement3::IDENTITY,
            &StretchPolicy::default(),
        )
        .unwrap();
        let (bound, stats) =
            stretch_mesh(&source, &plane, 1.0, &Placement3::IDENTITY, &policy).unwrap();
        assert_eq!(
            bound
                .mesh
                .to_trimesh(&exedra_mesh::ExtractParams::default()),
            direct
                .mesh
                .to_trimesh(&exedra_mesh::ExtractParams::default())
        );
        assert_eq!(stats, direct.stats);
        for (face, from) in direct.face_sources {
            assert_eq!(
                bound.face_materials[&face],
                source.face_materials[&from.face()]
            );
            assert_eq!(
                bound.source_map.face_feature(face),
                source.source_map.face_feature(from.face())
            );
        }
        for (vertex, from) in direct.vertex_sources {
            let expected = match from {
                StretchVertexSource::Original(from) => {
                    source.source_map.vertex_feature(from).unwrap()
                }
                StretchVertexSource::Seam { rim } => Feature::StretchSeam { rim },
            };
            assert_eq!(bound.source_map.vertex_feature(vertex), Some(expected));
        }
    }
}
