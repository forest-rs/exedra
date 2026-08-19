// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Content-addressed, memoized part compilation.
//!
//! [`PartCompiler::compile_parts`] tessellates each distinct part once per
//! `(content fingerprint, policy fingerprint)` pair; any number of
//! instances share the compiled result. Compilation is deterministic, so
//! the cache is pure memoization: hits and misses can never change output,
//! only work.
//!
//! Dirty tracking runs through the `invalidation` crate: part-content
//! edits are marked on a parts channel, and the next compile drains the
//! channel and evicts exactly the marked parts' entries. Binding and
//! metadata edits never touch this layer.

use alloc::rc::Rc;
use alloc::vec::Vec;

use exedra::{ExtractParams, FaceTriangulation, TriMesh};
use exedra_constructive::EVAL_SCHEMA_VERSION;
use exedra_constructive::evaluate::{EvalError, evaluate};
use exedra_constructive::tessellate::EvalPolicy;
use hashbrown::HashMap;
use invalidation::{Channel, InvalidationSet};

use crate::assembly::{Assembly, PartId, PartSource};

/// The single invalidation channel this layer uses: part content.
const PARTS_CHANNEL: Channel = Channel::new(0);

/// Content identity of a compiled part (recipe fingerprint, or canonical
/// mesh bytes for baked parts).
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartFingerprint(pub u128);

/// Identity of everything in the evaluation policy that can change
/// tessellation output, folded with [`EVAL_SCHEMA_VERSION`].
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PolicyFingerprint(pub u64);

/// Computes the policy fingerprint for `policy`.
///
/// Folds every policy field's exact bits plus [`EVAL_SCHEMA_VERSION`], so
/// schema bumps invalidate compiled caches explicitly. Must be extended
/// whenever [`EvalPolicy`] grows a field.
#[must_use]
pub fn policy_fingerprint(policy: &EvalPolicy) -> PolicyFingerprint {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(&EVAL_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.chord_tolerance.to_bits().to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.max_segment_edges.to_le_bytes());
    bytes.extend_from_slice(&policy.discretize.min_arc_edges.to_le_bytes());
    bytes.extend_from_slice(&policy.sharp_sin_threshold.to_bits().to_le_bytes());
    let h = fnv128(&bytes);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "xor-folding a 128-bit hash to 64 bits is the intended narrowing"
    )]
    PolicyFingerprint((h ^ (h >> 64)) as u64)
}

/// One contiguous run of triangle indices sharing a `FACE_REGION` value.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RegionRange {
    /// The `FACE_REGION` value.
    pub region: u32,
    /// First index (multiple of 3) in the body's index buffer.
    pub start: u32,
    /// Number of indices (multiple of 3).
    pub count: u32,
}

/// One tessellated body of a compiled part, region-grouped for rendering.
///
/// The index buffer is reordered so that each distinct `FACE_REGION`
/// value occupies exactly one contiguous range; triangle order within a
/// region preserves extraction order, so output is deterministic.
#[derive(Clone, Debug)]
pub struct CompiledBody {
    /// Extracted render buffers (indices region-grouped).
    pub tri: TriMesh,
    /// Contiguous per-region ranges covering the whole index buffer, in
    /// ascending region order.
    pub regions: Vec<RegionRange>,
}

/// A compiled part: identity plus its tessellated bodies.
#[derive(Clone, Debug)]
pub struct CompiledPart {
    /// Content identity this compilation was keyed on.
    pub fingerprint: PartFingerprint,
    /// Bodies in deterministic evaluation order (one for baked parts).
    pub bodies: Vec<CompiledBody>,
}

impl CompiledPart {
    /// Total triangle count across all bodies.
    #[must_use]
    pub fn triangle_count(&self) -> u64 {
        self.bodies
            .iter()
            .map(|b| (b.tri.indices.len() / 3) as u64)
            .sum()
    }
}

/// Introspection counters accumulated across a compiler's lifetime.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct CompileCounters {
    /// Parts tessellated (cache misses).
    pub parts_compiled: u64,
    /// Cache hits (a compiled entry was reused).
    pub cache_hits: u64,
    /// Entries evicted through the parts invalidation channel.
    pub cache_evictions: u64,
    /// Triangles emitted by cache-miss compilations.
    pub triangles_emitted: u64,
}

/// Typed compilation failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompileError {
    /// A recipe part failed to evaluate.
    Evaluate {
        /// The failing part.
        part: PartId,
        /// The underlying evaluation failure.
        error: EvalError,
    },
}

impl core::fmt::Display for CompileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Evaluate { part, error } => {
                write!(f, "part {part:?} failed to evaluate: {error}")
            }
        }
    }
}

impl core::error::Error for CompileError {}

/// The compiled view of an assembly's parts.
#[derive(Clone, Debug)]
pub struct CompiledParts {
    /// Compiled entry per part, indexed by [`PartId`].
    parts: Vec<Rc<CompiledPart>>,
}

impl CompiledParts {
    /// The compiled entry for a part.
    #[must_use]
    pub fn part(&self, id: PartId) -> Option<&Rc<CompiledPart>> {
        self.parts.get(id.0 as usize)
    }

    /// All compiled entries in [`PartId`] order.
    #[must_use]
    pub fn parts(&self) -> &[Rc<CompiledPart>] {
        &self.parts
    }
}

/// Memoizing part compiler with invalidation-channel eviction.
#[derive(Debug, Default)]
pub struct PartCompiler {
    cache: HashMap<(PartFingerprint, PolicyFingerprint), Rc<CompiledPart>>,
    /// Cache keys last produced for each part, so channel-driven eviction
    /// can find them without scanning.
    part_keys: HashMap<PartId, Vec<(PartFingerprint, PolicyFingerprint)>>,
    /// Baked-mesh content fingerprints, valid for `baked_generation`.
    baked_fingerprints: HashMap<PartId, PartFingerprint>,
    baked_generation: u64,
    dirty: InvalidationSet<PartId>,
    counters: CompileCounters,
}

impl PartCompiler {
    /// Creates an empty compiler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Lifetime counters.
    #[must_use]
    pub fn counters(&self) -> CompileCounters {
        self.counters
    }

    /// Number of live cache entries.
    #[must_use]
    pub fn cached_entries(&self) -> usize {
        self.cache.len()
    }

    /// Marks a part's content as changed on the parts channel.
    ///
    /// Call after [`Assembly::replace_part_source`]; the next
    /// [`Self::compile_parts`] evicts exactly this part's entries. Content
    /// addressing keeps results correct even without a mark — marking
    /// controls memory, not correctness.
    pub fn mark_part_changed(&mut self, part: PartId) {
        self.dirty.mark(part, PARTS_CHANNEL);
    }

    /// Compiles every part of `assembly` under `policy`, reusing cached
    /// entries whenever content and policy fingerprints match.
    ///
    /// # Errors
    ///
    /// Fails when a recipe part fails to evaluate; the cache keeps all
    /// other entries.
    pub fn compile_parts(
        &mut self,
        assembly: &Assembly,
        policy: &EvalPolicy,
    ) -> Result<CompiledParts, CompileError> {
        self.drain_dirty();
        if self.baked_generation != assembly.content_generation() {
            self.baked_fingerprints.clear();
            self.baked_generation = assembly.content_generation();
        }
        let policy_fp = policy_fingerprint(policy);
        let mut out = Vec::with_capacity(assembly.parts().len());
        for (index, def) in assembly.parts().iter().enumerate() {
            let id = PartId(crate::len_u32(index));
            let content_fp = self.part_fingerprint(id, def.source());
            let key = (content_fp, policy_fp);
            if let Some(hit) = self.cache.get(&key) {
                self.counters.cache_hits += 1;
                out.push(Rc::clone(hit));
                continue;
            }
            let compiled = Rc::new(compile_source(id, def.source(), policy, content_fp)?);
            self.counters.parts_compiled += 1;
            self.counters.triangles_emitted += compiled.triangle_count();
            self.cache.insert(key, Rc::clone(&compiled));
            self.part_keys.entry(id).or_default().push(key);
            out.push(compiled);
        }
        Ok(CompiledParts { parts: out })
    }

    fn drain_dirty(&mut self) {
        if !self.dirty.has_invalidated(PARTS_CHANNEL) {
            return;
        }
        // Drain to a sorted list: eviction is order-independent, but the
        // counters and map states must not depend on hash order.
        let mut marked: Vec<PartId> = self.dirty.drain(PARTS_CHANNEL).collect();
        marked.sort_unstable();
        for part in marked {
            self.baked_fingerprints.remove(&part);
            if let Some(keys) = self.part_keys.remove(&part) {
                for key in keys {
                    if self.cache.remove(&key).is_some() {
                        self.counters.cache_evictions += 1;
                    }
                }
            }
        }
    }

    fn part_fingerprint(&mut self, id: PartId, source: &PartSource) -> PartFingerprint {
        match source {
            PartSource::Recipe(recipe) => PartFingerprint(recipe.recipe_fingerprint().0),
            PartSource::Baked(mesh) => {
                if let Some(fp) = self.baked_fingerprints.get(&id) {
                    return *fp;
                }
                let fp = baked_mesh_fingerprint(mesh);
                self.baked_fingerprints.insert(id, fp);
                fp
            }
        }
    }
}

fn compile_source(
    part: PartId,
    source: &PartSource,
    policy: &EvalPolicy,
    fingerprint: PartFingerprint,
) -> Result<CompiledPart, CompileError> {
    let bodies = match source {
        PartSource::Recipe(recipe) => {
            let evaluation =
                evaluate(recipe, policy).map_err(|error| CompileError::Evaluate { part, error })?;
            evaluation
                .bodies
                .iter()
                .map(|placed| compile_body(&placed.body.mesh))
                .collect()
        }
        PartSource::Baked(mesh) => alloc::vec![compile_body(mesh)],
    };
    Ok(CompiledPart {
        fingerprint,
        bodies,
    })
}

/// Extracts render buffers and regroups the index buffer so each
/// `FACE_REGION` value is one contiguous range.
fn compile_body(mesh: &exedra::Mesh) -> CompiledBody {
    let params = ExtractParams {
        face_triangulation: FaceTriangulation::Robust,
        ..ExtractParams::default()
    };
    let (tri, _stats) = mesh.to_trimesh(&params);
    // Per-triangle regions in extraction order: to_trimesh emits faces in
    // face-id order, so per-face triangle counts line up exactly.
    let regions_layer = mesh.attrs().dense(exedra::attr::FACE_REGION);
    let triangle_count = tri.indices.len() / 3;
    let mut tri_regions = Vec::with_capacity(triangle_count);
    for face in mesh.faces() {
        let (triangles, _) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        let region = regions_layer
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        for _ in 0..triangles.len() {
            tri_regions.push(region);
        }
    }
    debug_assert_eq!(
        tri_regions.len(),
        triangle_count,
        "per-face counts must match extraction emission"
    );
    // Stable regroup: order triangles by (region, extraction order).
    let mut order: Vec<u32> = (0..crate::len_u32(triangle_count)).collect();
    order.sort_by_key(|&t| (tri_regions[t as usize], t));
    let mut indices = Vec::with_capacity(tri.indices.len());
    let mut regions: Vec<RegionRange> = Vec::new();
    for &t in &order {
        let region = tri_regions[t as usize];
        let start = crate::len_u32(indices.len());
        match regions.last_mut() {
            Some(last) if last.region == region => last.count += 3,
            _ => regions.push(RegionRange {
                region,
                start,
                count: 3,
            }),
        }
        let base = t as usize * 3;
        indices.extend_from_slice(&tri.indices[base..base + 3]);
    }
    CompiledBody {
        tri: TriMesh { indices, ..tri },
        regions,
    }
}

/// Canonical content fingerprint of a baked mesh: vertex positions in id
/// order plus face loops rotated to their minimum vertex index.
fn baked_mesh_fingerprint(mesh: &exedra::Mesh) -> PartFingerprint {
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
        bytes.extend_from_slice(&crate::len_u32(loop_vertices.len()).to_le_bytes());
        for v in loop_vertices {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    PartFingerprint(fnv128(&bytes))
}

/// Structural fingerprint of an assembly: part contents, slot tables and
/// mappings, the instance tree with placements, bindings, and metadata.
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
            PartSource::Baked(mesh) => baked_mesh_fingerprint(mesh).0,
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
        bytes.extend_from_slice(&inst.part().0.to_le_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::{Assembly, InstanceId};
    use exedra_constructive::builders;
    use exedra_constructive::ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder};

    fn prism_recipe(width: f64) -> Recipe {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect(width, 20.0).unwrap());
        let node = b
            .add(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 10.0,
                caps: CapMode::Both,
            })
            .unwrap();
        b.finish(node).unwrap()
    }

    fn n_instance_assembly(n: u32) -> Assembly {
        let mut asm = Assembly::new();
        let part = asm.add_recipe_part("panel", prism_recipe(40.0)).unwrap();
        for i in 0..n {
            asm.add_instance(
                None,
                &alloc::format!("p{i}"),
                part,
                Placement3::translate(f64::from(i) * 50.0, 0.0, 0.0),
            )
            .unwrap();
        }
        asm
    }

    #[test]
    fn n_instances_compile_once() {
        let asm = n_instance_assembly(8);
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        assert_eq!(compiled.parts().len(), 1);
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 0);
        // Re-compiling the same assembly is a pure hit.
        let again = compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 1);
        assert!(Rc::ptr_eq(
            compiled.part(PartId(0)).unwrap(),
            again.part(PartId(0)).unwrap()
        ));
    }

    #[test]
    fn double_run_determinism() {
        let asm = n_instance_assembly(2);
        let mut c1 = PartCompiler::new();
        let mut c2 = PartCompiler::new();
        let a = c1.compile_parts(&asm, &EvalPolicy::default()).unwrap();
        let b = c2.compile_parts(&asm, &EvalPolicy::default()).unwrap();
        let (pa, pb) = (a.part(PartId(0)).unwrap(), b.part(PartId(0)).unwrap());
        assert_eq!(pa.fingerprint, pb.fingerprint);
        assert_eq!(pa.bodies.len(), pb.bodies.len());
        for (body_a, body_b) in pa.bodies.iter().zip(&pb.bodies) {
            assert_eq!(body_a.tri, body_b.tri);
            assert_eq!(body_a.regions, body_b.regions);
        }
    }

    #[test]
    fn region_ranges_cover_all_triangles_contiguously() {
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        let body = &compiled.part(PartId(0)).unwrap().bodies[0];
        let mut cursor = 0;
        let mut prev_region = None;
        for range in &body.regions {
            assert_eq!(range.start, cursor, "ranges must be contiguous");
            assert_eq!(range.count % 3, 0, "ranges must be whole triangles");
            if let Some(prev) = prev_region {
                assert!(range.region > prev, "regions must be ascending and unique");
            }
            prev_region = Some(range.region);
            cursor += range.count;
        }
        assert_eq!(cursor as usize, body.tri.indices.len());
        // A rect prism has both caps and four wall segments.
        assert!(body.regions.len() >= 6, "expected caps plus four walls");
    }

    #[test]
    fn policy_change_misses_cache() {
        let asm = n_instance_assembly(1);
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        let mut coarse = EvalPolicy::default();
        coarse.discretize.chord_tolerance = 0.5;
        compiler.compile_parts(&asm, &coarse).unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);
        assert_ne!(
            policy_fingerprint(&EvalPolicy::default()),
            policy_fingerprint(&coarse)
        );
    }

    #[test]
    fn slot_rebinding_compiles_nothing() {
        let mut asm = n_instance_assembly(3);
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        let compiled_before = compiler.counters().parts_compiled;
        // Rebind materials on every instance: pure structure.
        let part = asm.part_by_key("panel").unwrap();
        let _ = part;
        for id in 0..3 {
            // The test recipe declares no slots, so bind_material errors —
            // metadata stands in for a structure-only edit here; slot
            // rebinding is covered in the flatten tests with real slots.
            asm.set_metadata(InstanceId(id), "note", "edited").unwrap();
        }
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, compiled_before);
        assert_eq!(compiler.counters().cache_hits, 1);
    }

    #[test]
    fn replacing_one_part_evicts_only_it() {
        let mut asm = Assembly::new();
        let a = asm.add_recipe_part("a", prism_recipe(40.0)).unwrap();
        let b = asm.add_recipe_part("b", prism_recipe(60.0)).unwrap();
        asm.add_instance(None, "ia", a, Placement3::IDENTITY)
            .unwrap();
        asm.add_instance(None, "ib", b, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 2);

        asm.replace_part_source(a, PartSource::Recipe(prism_recipe(45.0)))
            .unwrap();
        compiler.mark_part_changed(a);
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        // Part a: evicted + recompiled (content changed). Part b: pure hit.
        assert_eq!(compiler.counters().cache_evictions, 1);
        assert_eq!(compiler.counters().parts_compiled, 3);
        assert_eq!(compiler.counters().cache_hits, 1);
    }

    #[test]
    fn baked_parts_fingerprint_and_compile() {
        let recipe = prism_recipe(40.0);
        let evaluation = evaluate(&recipe, &EvalPolicy::default()).unwrap();
        let mesh = evaluation
            .bodies
            .into_iter()
            .next()
            .unwrap()
            .body
            .mesh
            .clone();
        let mut asm = Assembly::new();
        let part = asm.add_baked_part("baked", mesh, &["shell"]).unwrap();
        asm.add_instance(None, "i", part, Placement3::IDENTITY)
            .unwrap();
        let mut compiler = PartCompiler::new();
        let compiled = compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        let entry = compiled.part(part).unwrap();
        assert_eq!(entry.bodies.len(), 1);
        assert!(entry.triangle_count() > 0);
        // Second run reuses the memoized baked fingerprint and hits.
        compiler
            .compile_parts(&asm, &EvalPolicy::default())
            .unwrap();
        assert_eq!(compiler.counters().parts_compiled, 1);
        assert_eq!(compiler.counters().cache_hits, 1);
    }
}
