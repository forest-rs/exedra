// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `exedra-assembly-v1`: the JSON interchange schema for assemblies.
//!
//! Mirrors the `exedra-recipe-v1` policy (see that module's ADR-0003):
//! dedicated DTO types — never derives on internal types — additive-only
//! evolution within version 1, unknown *fields* tolerated, unknown part
//! *source kinds* hard errors (an assembly is executable content).
//! Deserialization rebuilds through the validated [`Assembly`] API,
//! re-running every check; the correctness oracle is
//! [`assembly_fingerprint`] equality across a round trip.
//!
//! Recipe parts embed their full `exedra-recipe-v1` document (the recipe
//! DTO is the one source of truth for recipe encoding); baked parts embed
//! positions plus canonical face loops. Instances serialize as a flat
//! parent-indexed list in insertion order, so rebuild is a single forward
//! pass and parents always precede children.

#![cfg(feature = "serde")]

use alloc::string::String;
use alloc::vec::Vec;

use exedra_constructive::interchange::{InterchangeError, MeshDto, RecipeDto};
use serde::{Deserialize, Serialize};

use crate::assembly::{Assembly, AssemblyError, InstanceId, PartId, PartSource};
use crate::compile::assembly_fingerprint;

/// Format name in the header.
pub const FORMAT: &str = "exedra-assembly";
/// Format version this module reads and writes.
pub const VERSION: u32 = 1;

/// Top-level interchange document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssemblyDto {
    /// Format name; always [`FORMAT`].
    pub format: String,
    /// Format version; always [`VERSION`].
    pub version: u32,
    /// Parts in registration order.
    pub parts: Vec<PartDto>,
    /// Instances in insertion order; parents precede children.
    pub instances: Vec<InstanceDto>,
}

/// One part record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartDto {
    /// The opaque frontend registration key.
    pub key: String,
    /// The geometry source.
    pub source: PartSourceDto,
    /// Explicit region-to-slot mappings.
    #[serde(default)]
    pub region_slots: Vec<RegionSlotDto>,
    /// Part-wide default slot name for unmapped regions.
    #[serde(default)]
    pub default_slot: Option<String>,
    /// Part-default material keys per slot.
    #[serde(default)]
    pub default_materials: Vec<SlotMaterialDto>,
}

/// The geometry source of a part.
///
/// Unknown kinds are hard errors: skipping one would silently drop
/// geometry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PartSourceDto {
    /// A constructive recipe, embedded as a full `exedra-recipe-v1`
    /// document.
    Recipe {
        /// The embedded recipe document.
        recipe: RecipeDto,
    },
    /// A baked mesh with explicitly declared slots.
    Baked {
        /// The mesh: positions plus canonical face loops.
        mesh: MeshDto,
        /// Declared slot names in slot order.
        slots: Vec<String>,
    },
}

/// A region-to-slot mapping entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionSlotDto {
    /// The `FACE_REGION` value.
    pub region: u32,
    /// The slot name it maps to.
    pub slot: String,
}

/// A slot-to-material binding entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlotMaterialDto {
    /// The slot name.
    pub slot: String,
    /// The opaque material key.
    pub material: String,
}

/// One instance record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstanceDto {
    /// Index of the parent instance, or `None` for roots. Must be smaller
    /// than this record's own index.
    pub parent: Option<u32>,
    /// The sibling-unique instance key.
    pub key: String,
    /// Index into `parts`.
    pub part: u32,
    /// Placement rows (3x4 row-major).
    pub placement: [[f64; 4]; 3],
    /// Slot-to-material bindings.
    #[serde(default)]
    pub bindings: Vec<SlotMaterialDto>,
    /// Opaque metadata pairs.
    #[serde(default)]
    pub metadata: Vec<MetadataDto>,
}

/// One opaque metadata pair.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetadataDto {
    /// The metadata key.
    pub key: String,
    /// The metadata value.
    pub value: String,
}

/// Typed assembly interchange failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AssemblyInterchangeError {
    /// Wrong format name or unsupported version.
    UnsupportedFormat,
    /// An embedded recipe failed to rebuild.
    Recipe(InterchangeError),
    /// A baked mesh failed to rebuild.
    InvalidMesh,
    /// An instance referenced a part index out of range.
    DanglingPart(u32),
    /// An instance referenced a parent at or after its own index.
    DanglingParent(u32),
    /// Assembly validation failed on rebuild.
    Assembly(AssemblyError),
}

impl core::fmt::Display for AssemblyInterchangeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported format or version"),
            Self::Recipe(e) => write!(f, "embedded recipe failed to rebuild: {e}"),
            Self::InvalidMesh => write!(f, "baked mesh failed to rebuild"),
            Self::DanglingPart(i) => write!(f, "instance references unknown part index {i}"),
            Self::DanglingParent(i) => {
                write!(f, "instance references parent index {i} not yet built")
            }
            Self::Assembly(e) => write!(f, "assembly validation failed: {e}"),
        }
    }
}

impl core::error::Error for AssemblyInterchangeError {}

impl From<AssemblyError> for AssemblyInterchangeError {
    fn from(e: AssemblyError) -> Self {
        Self::Assembly(e)
    }
}

/// Serializes an assembly to its interchange document.
#[must_use]
pub fn to_dto(assembly: &Assembly) -> AssemblyDto {
    let parts = assembly
        .parts()
        .iter()
        .map(|def| {
            let source = match def.source() {
                PartSource::Recipe(recipe) => PartSourceDto::Recipe {
                    recipe: exedra_constructive::interchange::to_dto(recipe),
                },
                PartSource::Baked(mesh) => PartSourceDto::Baked {
                    mesh: mesh_to_dto(mesh),
                    slots: def.slots().to_vec(),
                },
            };
            let slot_name = |index: crate::assembly::SlotIndex| -> String {
                def.slots()[index.0 as usize].clone()
            };
            PartDto {
                key: String::from(def.key()),
                source,
                region_slots: def
                    .region_slots()
                    .iter()
                    .map(|(region, slot)| RegionSlotDto {
                        region: *region,
                        slot: slot_name(*slot),
                    })
                    .collect(),
                default_slot: def.default_slot().map(slot_name),
                default_materials: def
                    .default_materials()
                    .iter()
                    .enumerate()
                    .filter_map(|(i, material)| {
                        material.as_ref().map(|m| SlotMaterialDto {
                            slot: def.slots()[i].clone(),
                            material: m.clone(),
                        })
                    })
                    .collect(),
            }
        })
        .collect();
    let instances = assembly
        .instances()
        .iter()
        .map(|inst| {
            let def = assembly
                .part(inst.part())
                .expect("instances always reference registered parts");
            InstanceDto {
                parent: inst.parent().map(|p| p.0),
                key: String::from(inst.key()),
                part: inst.part().0,
                placement: inst.placement().rows,
                bindings: inst
                    .bindings()
                    .iter()
                    .map(|(slot, material)| SlotMaterialDto {
                        slot: def.slots()[slot.0 as usize].clone(),
                        material: material.clone(),
                    })
                    .collect(),
                metadata: inst
                    .metadata()
                    .iter()
                    .map(|(key, value)| MetadataDto {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    AssemblyDto {
        format: String::from(FORMAT),
        version: VERSION,
        parts,
        instances,
    }
}

/// Rebuilds an assembly from its interchange document, re-running every
/// validation.
///
/// # Errors
///
/// Fails on a bad header, an embedded recipe/mesh that does not rebuild,
/// dangling indices, or any assembly validation failure.
pub fn from_dto(dto: &AssemblyDto) -> Result<Assembly, AssemblyInterchangeError> {
    if dto.format != FORMAT || dto.version != VERSION {
        return Err(AssemblyInterchangeError::UnsupportedFormat);
    }
    let mut assembly = Assembly::new();
    for part in &dto.parts {
        let id = match &part.source {
            PartSourceDto::Recipe { recipe } => {
                let recipe = exedra_constructive::interchange::from_dto(recipe)
                    .map_err(AssemblyInterchangeError::Recipe)?;
                assembly.add_recipe_part(&part.key, recipe)?
            }
            PartSourceDto::Baked { mesh, slots } => {
                let mesh = mesh_from_dto(mesh)?;
                let slot_refs: Vec<&str> = slots.iter().map(String::as_str).collect();
                assembly.add_baked_part(&part.key, mesh, &slot_refs)?
            }
        };
        for entry in &part.region_slots {
            assembly.bind_region_slot(id, entry.region, &entry.slot)?;
        }
        if let Some(slot) = &part.default_slot {
            assembly.set_default_slot(id, slot)?;
        }
        for entry in &part.default_materials {
            assembly.set_part_material(id, &entry.slot, &entry.material)?;
        }
    }
    for (index, inst) in dto.instances.iter().enumerate() {
        if inst.part as usize >= dto.parts.len() {
            return Err(AssemblyInterchangeError::DanglingPart(inst.part));
        }
        let parent = match inst.parent {
            Some(p) => {
                if p as usize >= index {
                    return Err(AssemblyInterchangeError::DanglingParent(p));
                }
                Some(InstanceId(p))
            }
            None => None,
        };
        let id = assembly.add_instance(
            parent,
            &inst.key,
            PartId(inst.part),
            exedra_constructive::ir::Placement3 {
                rows: inst.placement,
            },
        )?;
        for binding in &inst.bindings {
            assembly.bind_material(id, &binding.slot, &binding.material)?;
        }
        for meta in &inst.metadata {
            assembly.set_metadata(id, &meta.key, &meta.value)?;
        }
    }
    Ok(assembly)
}

fn mesh_to_dto(mesh: &exedra::Mesh) -> MeshDto {
    MeshDto {
        positions: mesh
            .vertices()
            .filter_map(|v| mesh.vertex_position(v))
            .copied()
            .collect(),
        faces: mesh
            .faces()
            .map(|face| {
                let mut loop_vertices: Vec<u32> = mesh
                    .face_loop(face)
                    .filter_map(|he| mesh.to_vertex(he))
                    .map(exedra::VertexId::index)
                    .collect();
                if let Some(min_pos) = loop_vertices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| **v)
                    .map(|(i, _)| i)
                {
                    loop_vertices.rotate_left(min_pos);
                }
                loop_vertices
            })
            .collect(),
    }
}

fn mesh_from_dto(dto: &MeshDto) -> Result<exedra::Mesh, AssemblyInterchangeError> {
    let loops: Vec<&[u32]> = dto.faces.iter().map(Vec::as_slice).collect();
    exedra::Mesh::from_polygons(&dto.positions, &loops)
        .map_err(|_| AssemblyInterchangeError::InvalidMesh)
}

/// The round-trip oracle: serialize, rebuild, compare fingerprints.
///
/// Exposed for hosts that want the same check this crate's tests run.
#[must_use]
pub fn round_trips(assembly: &Assembly) -> bool {
    let dto = to_dto(assembly);
    match from_dto(&dto) {
        Ok(rebuilt) => assembly_fingerprint(&rebuilt) == assembly_fingerprint(assembly),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};
    use exedra_constructive::tessellate::EvalPolicy;

    fn slotted_recipe() -> Recipe {
        let mut b = RecipeBuilder::new();
        let front = b.material_slot("front");
        let profile = b.add_profile(builders::rounded_rect(60.0, 40.0, 8.0).unwrap());
        let node = b
            .with_material(front)
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 18.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    fn corpus_assembly() -> Assembly {
        let mut asm = Assembly::new();
        let panel = asm.add_recipe_part("panel", slotted_recipe()).unwrap();
        asm.bind_region_slot(panel, 0, "front").unwrap();
        asm.set_default_slot(panel, "front").unwrap();
        asm.set_part_material(panel, "front", "oak").unwrap();

        let evaluation =
            exedra_constructive::evaluate::evaluate(&slotted_recipe(), &EvalPolicy::default())
                .unwrap();
        let mesh = evaluation
            .bodies
            .into_iter()
            .next()
            .unwrap()
            .body
            .mesh
            .clone();
        let baked = asm.add_baked_part("trim", mesh, &["shell"]).unwrap();
        asm.set_default_slot(baked, "shell").unwrap();

        let root = asm
            .add_instance(
                None,
                "unit",
                panel,
                Placement3::rotate_z_then_translate(0.25, 10.0, 0.0, 0.0),
            )
            .unwrap();
        let shelf = asm
            .add_instance(
                Some(root),
                "shelf",
                panel,
                Placement3::translate(0.0, 0.0, 20.0),
            )
            .unwrap();
        asm.bind_material(shelf, "front", "walnut").unwrap();
        asm.set_metadata(shelf, "position", "upper").unwrap();
        asm.add_instance(
            Some(root),
            "edge",
            baked,
            Placement3::translate(0.0, 5.0, 0.0),
        )
        .unwrap();
        asm
    }

    #[test]
    fn round_trip_fingerprint_equality() {
        let asm = corpus_assembly();
        assert!(round_trips(&asm), "corpus must round-trip bit-exactly");
        // And through actual JSON text, not just DTOs.
        let json = serde_json::to_string(&to_dto(&asm)).unwrap();
        let dto: AssemblyDto = serde_json::from_str(&json).unwrap();
        let rebuilt = from_dto(&dto).unwrap();
        assert_eq!(
            assembly_fingerprint(&rebuilt),
            assembly_fingerprint(&asm),
            "JSON text round trip must preserve the fingerprint"
        );
    }

    #[test]
    fn header_and_reference_validation() {
        let asm = corpus_assembly();
        let mut dto = to_dto(&asm);
        dto.format = String::from("other");
        assert!(matches!(
            from_dto(&dto),
            Err(AssemblyInterchangeError::UnsupportedFormat)
        ));
        let mut dto = to_dto(&asm);
        dto.version = 2;
        assert!(matches!(
            from_dto(&dto),
            Err(AssemblyInterchangeError::UnsupportedFormat)
        ));
        let mut dto = to_dto(&asm);
        dto.instances[0].part = 99;
        assert!(matches!(
            from_dto(&dto),
            Err(AssemblyInterchangeError::DanglingPart(99))
        ));
        let mut dto = to_dto(&asm);
        dto.instances[0].parent = Some(5);
        assert!(matches!(
            from_dto(&dto),
            Err(AssemblyInterchangeError::DanglingParent(5))
        ));
    }

    #[test]
    fn unknown_fields_tolerated_unknown_kinds_rejected() {
        let asm = corpus_assembly();
        let json = serde_json::to_string(&to_dto(&asm)).unwrap();
        // Additive tolerance: inject an unknown field.
        let extended = json.replacen(
            "\"format\":",
            "\"future_field\": {\"x\": 1}, \"format\":",
            1,
        );
        let dto: AssemblyDto = serde_json::from_str(&extended).expect("unknown fields tolerated");
        assert!(from_dto(&dto).is_ok());
        // Unknown source kinds are hard errors at parse time.
        let broken = json.replacen("\"kind\":\"recipe\"", "\"kind\":\"future\"", 1);
        assert!(
            serde_json::from_str::<AssemblyDto>(&broken).is_err(),
            "unknown part-source kinds must not parse"
        );
    }

    #[test]
    fn frozen_schema_corpus_still_parses() {
        let json = include_str!("../goldens/assembly_v1.frozen.json");
        let dto: AssemblyDto = serde_json::from_str(json).expect("frozen corpus parses");
        let rebuilt = from_dto(&dto).expect("frozen corpus rebuilds");
        assert_eq!(
            alloc::format!("{:#034x}", assembly_fingerprint(&rebuilt)),
            include_str!("../goldens/assembly_v1.frozen.fingerprint").trim(),
            "frozen corpus fingerprint changed; the schema or encoding drifted"
        );
    }

    /// Regenerates the frozen corpus. Run deliberately after a reviewed,
    /// additive schema change:
    /// `cargo test -p exedra_assembly --all-features -- --ignored bless_frozen_schema`
    #[test]
    #[ignore = "regenerates the frozen schema corpus; run deliberately"]
    fn bless_frozen_schema() {
        let asm = corpus_assembly();
        let json = serde_json::to_string_pretty(&to_dto(&asm)).unwrap();
        let fingerprint = alloc::format!("{:#034x}\n", assembly_fingerprint(&asm));
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens");
        std::fs::create_dir_all(&dir).expect("goldens dir");
        std::fs::write(dir.join("assembly_v1.frozen.json"), json).expect("write corpus");
        std::fs::write(dir.join("assembly_v1.frozen.fingerprint"), fingerprint)
            .expect("write fingerprint");
    }
}
