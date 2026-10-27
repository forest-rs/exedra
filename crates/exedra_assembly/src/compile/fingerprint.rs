// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::PartFingerprint;
use crate::{Assembly, PartSource};
use alloc::vec::Vec;
use exedra_constructive::EVAL_SCHEMA_VERSION;

// The v1 assembly interchange encodes baked geometry without its attributes.
// Keep its existing structural identity separate from compilation identity.
fn baked_geometry_fingerprint(mesh: &exedra_mesh::Mesh) -> PartFingerprint {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EVAL_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(b"baked-mesh");
    for vertex in mesh.vertices() {
        let p = mesh.vertex_position(vertex).copied().unwrap_or([0.0; 3]);
        for c in p {
            bytes.extend_from_slice(&c.to_bits().to_le_bytes());
        }
    }
    bytes.push(0xFF);
    for face in mesh.faces() {
        let mut loop_vertices: Vec<u32> = mesh
            .face_loop(face)
            .filter_map(|he| mesh.to_vertex(he))
            .map(exedra_mesh::VertexId::index)
            .collect();
        if let Some(min_pos) = loop_vertices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| **v)
            .map(|(i, _)| i)
        {
            loop_vertices.rotate_left(min_pos);
        }
        bytes.extend_from_slice(&crate::len_u32(loop_vertices.len()).to_le_bytes());
        for v in loop_vertices {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    PartFingerprint(fnv128(&bytes))
}

/// Structural fingerprint of an assembly: recipe contents, baked geometry,
/// slot tables and mappings, placements, bindings, metadata, placement sets
/// (their placements, seeds, tints, bindings and metadata), and part
/// level-of-detail chains.
///
/// Baked attributes are absent from the v1 JSON interchange and are excluded
/// here. Rendering caches must use [`super::CompiledPart::fingerprint`] together with
/// [`super::policy_fingerprint`], which include the consumed attributes and settings.
///
/// This is the round-trip oracle for the `exedra-assembly-v1` interchange:
/// serialize, rebuild, and the fingerprints must match bit-for-bit.
#[must_use]
pub fn assembly_fingerprint(assembly: &Assembly) -> u128 {
    fn push_str(bytes: &mut Vec<u8>, s: &str) {
        bytes.extend_from_slice(&crate::len_u32(s.len()).to_le_bytes());
        bytes.extend_from_slice(s.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"exedra-assembly-fp");
    bytes.extend_from_slice(&EVAL_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&crate::len_u32(assembly.parts().len()).to_le_bytes());
    for def in assembly.parts() {
        push_str(&mut bytes, def.key());
        let content = match def.source() {
            PartSource::Recipe(recipe) => recipe.recipe_fingerprint().0,
            PartSource::Baked(mesh) => baked_geometry_fingerprint(mesh).0,
            PartSource::Native(source) => source.fingerprint().0,
        };
        bytes.extend_from_slice(&content.to_le_bytes());
        bytes.extend_from_slice(&crate::len_u32(def.slots().len()).to_le_bytes());
        for slot in def.slots() {
            push_str(&mut bytes, slot);
        }
        for (region, slot) in def.region_slots() {
            bytes.extend_from_slice(&region.to_le_bytes());
            bytes.extend_from_slice(&slot.0.to_le_bytes());
        }
        bytes.extend_from_slice(&def.default_slot().map_or(u32::MAX, |s| s.0).to_le_bytes());
        for material in def.default_materials() {
            match material {
                Some(m) => push_str(&mut bytes, m),
                None => bytes.push(0),
            }
        }
    }
    bytes.extend_from_slice(&crate::len_u32(assembly.instances().len()).to_le_bytes());
    for inst in assembly.instances() {
        bytes.extend_from_slice(&inst.parent().map_or(u32::MAX, |p| p.0).to_le_bytes());
        push_str(&mut bytes, inst.key());
        match inst.part() {
            Some(part) => {
                bytes.push(1);
                bytes.extend_from_slice(&part.0.to_le_bytes());
            }
            None => bytes.push(0),
        }
        for row in inst.placement().rows {
            for v in row {
                bytes.extend_from_slice(&v.to_bits().to_le_bytes());
            }
        }
        bytes.extend_from_slice(&crate::len_u32(inst.bindings().len()).to_le_bytes());
        for (slot, material) in inst.bindings() {
            bytes.extend_from_slice(&slot.0.to_le_bytes());
            push_str(&mut bytes, material);
        }
        bytes.extend_from_slice(&crate::len_u32(inst.metadata().len()).to_le_bytes());
        for (key, value) in inst.metadata() {
            push_str(&mut bytes, key);
            push_str(&mut bytes, value);
        }
    }
    // Placement sets extend the encoding only when present, so assemblies
    // without sets keep their fingerprints.
    if !assembly.placement_sets().is_empty() {
        bytes.extend_from_slice(b"placement-sets");
        bytes.extend_from_slice(&crate::len_u32(assembly.placement_sets().len()).to_le_bytes());
        for set in assembly.placement_sets() {
            bytes.extend_from_slice(&set.parent().map_or(u32::MAX, |p| p.0).to_le_bytes());
            push_str(&mut bytes, set.key());
            bytes.extend_from_slice(&set.part().0.to_le_bytes());
            bytes.extend_from_slice(&crate::len_u32(set.len()).to_le_bytes());
            for placement in set.placements() {
                for v in placement.rows.iter().flatten() {
                    bytes.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            match set.seeds() {
                Some(seeds) => {
                    bytes.push(1);
                    for seed in seeds {
                        bytes.extend_from_slice(&seed.to_le_bytes());
                    }
                }
                None => bytes.push(0),
            }
            match set.tints() {
                Some(tints) => {
                    bytes.push(1);
                    for v in tints.iter().flatten() {
                        bytes.extend_from_slice(&v.to_bits().to_le_bytes());
                    }
                }
                None => bytes.push(0),
            }
            bytes.extend_from_slice(&crate::len_u32(set.bindings().len()).to_le_bytes());
            for (slot, material) in set.bindings() {
                bytes.extend_from_slice(&slot.0.to_le_bytes());
                push_str(&mut bytes, material);
            }
            bytes.extend_from_slice(&crate::len_u32(set.metadata().len()).to_le_bytes());
            for (key, value) in set.metadata() {
                push_str(&mut bytes, key);
                push_str(&mut bytes, value);
            }
        }
    }
    // Level-of-detail chains, likewise only when some part has one.
    if assembly.parts().iter().any(|def| !def.lods().is_empty()) {
        bytes.extend_from_slice(b"part-lods");
        for def in assembly.parts() {
            bytes.extend_from_slice(&crate::len_u32(def.lods().len()).to_le_bytes());
            for level in def.lods() {
                bytes.extend_from_slice(&level.part.0.to_le_bytes());
                bytes.extend_from_slice(&level.min_coverage.to_bits().to_le_bytes());
                bytes.extend_from_slice(&level.crossfade.to_bits().to_le_bytes());
            }
        }
    }
    fnv128(&bytes)
}

/// FNV-1a 128-bit over `bytes`.
fn fnv128(bytes: &[u8]) -> u128 {
    const FNV128_OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV128_PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut hash = FNV128_OFFSET;
    for &b in bytes {
        hash ^= u128::from(b);
        hash = hash.wrapping_mul(FNV128_PRIME);
    }
    hash
}
