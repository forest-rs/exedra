// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Optional glTF preparation from assembly structure.
//!
//! Assembly owns path identity, placement structure and binding resolution.
//! The exporter receives ordinary one-shot records and knows none of those APIs.
//! Enable `gltf` here, or select both facade features `assembly,gltf`.

use crate::{Assembly, CompilationMismatch, Occurrence, PartId, PlacementSetId};
use alloc::{borrow::Cow, string::ToString};
use exedra_compile::CompiledParts;
use exedra_gltf::{
    GltfError, GltfExtras, GltfGeometry, GltfLod, GltfMesh, GltfNode, GltfPlacementSet, GltfScene,
    GltfSource, JsonValue,
};

impl GltfSource for Assembly {
    fn gltf_scene<'a>(&'a self, compiled: &CompiledParts) -> Result<Cow<'a, GltfScene>, GltfError> {
        if self.parts().len() != compiled.parts().len() {
            return Err(GltfError::CompilationMismatch(
                CompilationMismatch::PartCount {
                    expected: self.parts().len(),
                    actual: compiled.parts().len(),
                },
            ));
        }
        for (index, (def, entry)) in self.parts().iter().zip(compiled.parts()).enumerate() {
            if def.source_fingerprint() != entry.fingerprint {
                return Err(GltfError::CompilationMismatch(
                    CompilationMismatch::PartContent {
                        part: PartId(crate::len_u32(index)),
                        expected: def.source_fingerprint(),
                        actual: entry.fingerprint,
                    },
                ));
            }
        }
        let nodes = self
            .instances_with_ids()
            .map(|(id, node)| {
                let path = self.path_of(id).expect("assembly path").to_string();
                let mut extras: GltfExtras = node
                    .metadata()
                    .iter()
                    .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
                    .collect();
                extras.insert("instancePath".into(), JsonValue::String(path.clone()));
                extras.remove("body");
                if let Some(part) = node.part() {
                    extras.insert(
                        "partKey".into(),
                        JsonValue::String(self.part(part).expect("assembly part").key().into()),
                    );
                } else {
                    extras.remove("partKey");
                }
                GltfNode {
                    name: path,
                    parent: node.parent().map(|id| id.0 as usize),
                    transform: *node.placement(),
                    geometry: node
                        .part()
                        .map(|part| geometry(self, compiled, Occurrence::Instance(id), part)),
                    extras,
                }
            })
            .collect();
        let placement_sets = self
            .placement_sets()
            .iter()
            .enumerate()
            .map(|(index, set)| {
                let id = PlacementSetId(crate::len_u32(index));
                GltfPlacementSet {
                    name: self
                        .placement_set_path(id)
                        .expect("assembly set path")
                        .to_string(),
                    parent: set.parent().map(|id| id.0 as usize),
                    geometry: geometry(self, compiled, Occurrence::PlacementSet(id), set.part()),
                    transforms: set.placements().into(),
                    seeds: set.seeds.clone(),
                    tints: set.tints.clone(),
                    extras: set
                        .metadata()
                        .iter()
                        .map(|(key, value)| (key.clone(), JsonValue::String(value.clone())))
                        .collect(),
                }
            })
            .collect();
        Ok(Cow::Owned(GltfScene {
            nodes,
            placement_sets,
        }))
    }
}
fn mesh(
    assembly: &Assembly,
    compiled: &CompiledParts,
    occurrence: Occurrence,
    part: PartId,
) -> GltfMesh {
    let def = assembly.part(part).expect("assembly part");
    let entry = compiled.part(part).expect("validated compilation");
    let materials = entry
        .bodies
        .iter()
        .map(|body| {
            body.regions
                .iter()
                .map(|range| {
                    range
                        .material_slot
                        .or_else(|| def.region_slot(range.region))
                        .and_then(|slot| assembly.resolved_level_material(occurrence, part, slot))
                        .map(ToString::to_string)
                })
                .collect()
        })
        .collect();
    GltfMesh {
        part,
        fingerprint: entry.fingerprint,
        name: def.key().into(),
        materials,
    }
}
fn geometry(
    assembly: &Assembly,
    compiled: &CompiledParts,
    occurrence: Occurrence,
    part: PartId,
) -> GltfGeometry {
    GltfGeometry {
        mesh: mesh(assembly, compiled, occurrence, part),
        lods: assembly
            .part(part)
            .expect("assembly part")
            .lods()
            .iter()
            .map(|level| GltfLod {
                mesh: mesh(assembly, compiled, occurrence, level.part),
                min_coverage: level.min_coverage,
                crossfade: level.crossfade,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Assembly, CompilationMismatch, CompiledParts, PartId};
    use alloc::{borrow::ToOwned, string::ToString, vec::Vec};
    use exedra_constructive::evaluate::Severity;
    use exedra_constructive::ir::Placement3;
    use exedra_gltf::*;
    use serde_json::{Value, json};
    use std::{format, vec};
    #[path = "attribute_tests.rs"]
    mod attribute_tests;
    #[path = "export_tests.rs"]
    mod export_tests;
    #[path = "extension_tests.rs"]
    mod extension_tests;
    #[path = "hierarchy_tests.rs"]
    mod hierarchy_tests;
    #[path = "instancing_tests.rs"]
    mod instancing_tests;
    #[path = "normal_tests.rs"]
    mod normal_tests;
    #[path = "set_lod_tests.rs"]
    mod set_lod_tests;
    #[path = "slot_tests.rs"]
    mod slot_tests;
    #[path = "texture_tests.rs"]
    mod texture_tests;
}
