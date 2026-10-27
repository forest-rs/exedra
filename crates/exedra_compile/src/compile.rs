// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Geometry compilation, extraction and explicit cache retention.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use alloc::collections::BTreeSet;
use exedra_constructive::EVAL_SCHEMA_VERSION;
use exedra_constructive::evaluate::{Aabb3, EvalError, GeometryReport, Severity, evaluate};
use exedra_constructive::tessellate::EvalPolicy;
use exedra_mesh::attributes::Domain;
use exedra_mesh::{
    AttributeValue, ExtractAttribute, ExtractParams, ExtractStats, FaceTriangulation,
    NormalsSource, TangentUv, TriMesh, UvSource,
};
pub use exedra_mesh_ops::measure::{SignedVolume, VolumeError};
use hashbrown::HashMap;

use crate::{PartId, PartInput, PartSource, SlotIndex};
mod evaluated;
mod snapshot;
use evaluated::EvaluatedPart;
pub use evaluated::{
    BodyLookupError, GeometrySnapshot, PartSnapshot, SnapshotBody, SnapshotSurfaceInventory,
    SnapshotWorkplane, StaleSelection,
};
pub use snapshot::CompilationMismatch;

/// Evaluation and render extraction policy for recipe and baked parts.
///
/// Defaults to derived normals and authored-only UVs. Select
/// [`NormalsSource::CustomOrDerived`] to preserve authored corner normals and
/// derive any missing overrides; select [`UvSource::CustomOrBoxProjected`] to
/// box-project every corner that has no authored UV. List extra layers in
/// [`Self::attributes`] to carry them into compiled bodies. Every
/// output-affecting setting participates in [`policy_fingerprint`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompilePolicy {
    /// Constructive evaluation settings; baked parts skip evaluation.
    pub evaluation: EvalPolicy,
    /// Explicit native body realization accuracy and work budgets. Recipes and
    /// baked sources do not use these settings.
    pub native: exedra_analytic::realize::RealizationPolicy,
    /// Normal selection for every emitted body, including imported meshes.
    ///
    /// [`NormalsSource::CustomOnly`] emits zero normals for missing overrides.
    /// Use [`NormalsSource::CustomOrDerived`] for partially authored meshes or
    /// parts that combine imported and generated geometry.
    pub normals: NormalsSource,
    /// UV selection for every emitted body, including imported meshes.
    ///
    /// [`UvSource::CustomOnly`] emits zero UVs for corners without an authored
    /// value and leaves [`RegionRange::has_uvs`] false for them. Under
    /// [`UvSource::CustomOrBoxProjected`] those corners project their position
    /// on the face's dominant-axis plane and count toward coverage whenever
    /// the emitted coordinates are finite. Authored UVs are never overwritten.
    pub uvs: UvSource,
    /// Attribute layers emitted as extra streams on every compiled body, in
    /// this order. See [`ExtractAttribute`] for resolution and splitting.
    pub attributes: Vec<ExtractAttribute>,
    /// MikkTSpace tangents for every compiled body, from the selected UV set.
    /// See [`ExtractParams::tangents`].
    ///
    /// [`TangentUv::Attribute`] reads a carried stream: list its key in
    /// [`Self::attributes`] too, or every tangent is a fallback (counted in
    /// [`CompileCounters::missing_tangent_uv_sets`]).
    pub tangents: Option<TangentUv>,
}

impl CompilePolicy {
    /// Returns the mesh extraction settings used by part compilation.
    ///
    /// Use these settings when inspecting or extracting the same stored mesh:
    /// compilation uses robust polygon triangulation rather than the kernel's
    /// default fan. Attribute descriptors are cloned; no mesh is evaluated.
    /// UV quality inspection of stored coordinates does not resolve the missing
    /// coordinate fallback in `uvs`; account for that policy separately.
    #[must_use]
    pub fn extraction_params(&self) -> ExtractParams {
        ExtractParams {
            normals: self.normals,
            uvs: self.uvs,
            face_triangulation: FaceTriangulation::Robust,
            attributes: self.attributes.clone(),
            tangents: self.tangents,
            ..ExtractParams::default()
        }
    }
}

impl From<EvalPolicy> for CompilePolicy {
    fn from(evaluation: EvalPolicy) -> Self {
        Self {
            evaluation,
            ..Self::default()
        }
    }
}

/// Content identity of a compiled part (recipe fingerprint, or canonical
/// mesh bytes for baked parts).
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PartFingerprint(pub u128);

/// Identity of evaluation and extraction settings that can change compiled
/// output, including [`EVAL_SCHEMA_VERSION`].
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PolicyFingerprint(pub u64);

/// Computes the policy fingerprint for `policy`.
///
/// Combines the constructive policy fingerprint with the normal and UV
/// sources, the native realization policy, carried attributes, and the tangent request. Constructive
/// evaluation owns fingerprinting its settings, including refinement budgets
/// and [`EVAL_SCHEMA_VERSION`]. The prefix advanced to `v4` when tangents
/// joined the identity; v5 includes native realization. Earlier versions never
/// collide with current ones.
#[must_use]
pub fn policy_fingerprint(policy: &CompilePolicy) -> PolicyFingerprint {
    let mut bytes = Vec::from(&b"assembly-compile-v5"[..]);
    bytes.extend_from_slice(&policy.native.tolerance.to_bits().to_le_bytes());
    bytes.extend_from_slice(&policy.native.geometry_gap.to_bits().to_le_bytes());
    for limit in [
        policy.native.min_arc_segments,
        policy.native.max_edge_segments,
        policy.native.max_vertices,
        policy.native.max_boundary_samples,
    ] {
        bytes.extend_from_slice(&limit.to_le_bytes());
    }
    bytes.extend_from_slice(
        &exedra_constructive::cache::policy_fingerprint(&policy.evaluation).to_le_bytes(),
    );
    bytes.push(match policy.normals {
        NormalsSource::Derived => 0,
        NormalsSource::CustomOrDerived => 1,
        NormalsSource::CustomOnly => 2,
    });
    match policy.uvs {
        UvSource::CustomOnly => bytes.push(0),
        UvSource::CustomOrBoxProjected { scale } => {
            bytes.push(1);
            bytes.extend_from_slice(&scale.to_bits().to_le_bytes());
        }
    }
    bytes.extend_from_slice(&(policy.attributes.len() as u64).to_le_bytes());
    for attribute in &policy.attributes {
        bytes.push(match attribute.domain() {
            Domain::Vertex => 0,
            Domain::Face => 1,
            Domain::HalfEdge => 2,
        });
        let name = attribute.name().as_bytes();
        bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(name);
        let mut push = |tag: u8, words: &[u32]| {
            bytes.push(tag);
            for word in words {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
        };
        match attribute.missing() {
            AttributeValue::F32(v) => push(0, &[v.to_bits()]),
            AttributeValue::Vec2(v) => push(1, &v.map(f32::to_bits)),
            AttributeValue::Vec3(v) => push(2, &v.map(f32::to_bits)),
            AttributeValue::Vec4(v) => push(3, &v.map(f32::to_bits)),
            AttributeValue::U32(v) => push(4, &[v]),
        }
    }
    match policy.tangents {
        None => bytes.push(0),
        Some(TangentUv::Primary) => bytes.push(1),
        Some(TangentUv::Attribute(key)) => {
            bytes.push(2);
            bytes.push(match key.domain() {
                Domain::Vertex => 0,
                Domain::Face => 1,
                Domain::HalfEdge => 2,
            });
            let name = key.name().as_bytes();
            bytes.extend_from_slice(&(name.len() as u64).to_le_bytes());
            bytes.extend_from_slice(name);
        }
    }
    let h = fnv128(&bytes);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "xor-folding a 128-bit hash to 64 bits is the intended narrowing"
    )]
    PolicyFingerprint((h ^ (h >> 64)) as u64)
}

/// One contiguous run of triangle indices sharing a geometric region and slot.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RegionRange {
    /// The `FACE_REGION` value.
    pub region: u32,
    /// Authored slot for these triangles, independent of their region ID.
    /// `None` permits caller region/default mappings as fallback.
    pub material_slot: Option<SlotIndex>,
    /// First index (multiple of 3) in the body's index buffer.
    pub start: u32,
    /// Number of indices (multiple of 3).
    pub count: u32,
    /// Whether every emitted triangle corner in this range has a finite UV
    /// after [`CompilePolicy::uvs`] is applied.
    ///
    /// Under [`UvSource::CustomOnly`] only finite authored UVs count;
    /// extraction's zero fallback for missing attributes does not. Under
    /// [`UvSource::CustomOrBoxProjected`] a corner without an authored UV
    /// counts when its emitted projected coordinates are finite. That is
    /// checked on the emitted values, not assumed: a finite position times a
    /// finite scale can still overflow, and an authored non-finite UV is left
    /// as authored.
    pub has_uvs: bool,
}

/// One tessellated body of a compiled part, region-grouped for rendering.
///
/// Ranges are ordered by `(region, material_slot)` with unassigned first;
/// triangle order within each range preserves extraction order. One region
/// may occur in several ranges when Boolean operands use different slots.
#[derive(Clone, Debug)]
pub struct CompiledBody {
    /// Extracted render buffers (indices region-grouped).
    pub tri: TriMesh,
    /// Contiguous region/slot ranges covering the whole index buffer.
    pub regions: Vec<RegionRange>,
    /// Counters from extracting this body, including carried-attribute
    /// fallbacks and missing layers.
    pub extraction: ExtractStats,
}

impl CompiledBody {
    /// Measures signed volume from the existing part-local render triangles.
    ///
    /// No evaluation, tessellation or topology retention is required. This is
    /// an algebraic measurement, not a closed-solid certificate. A positive sum
    /// does not prove every connected component faces outward. Occurrence
    /// placement is separate: call [`SignedVolume::transformed`] with a render
    /// item's world placement to include its scale and handedness.
    ///
    /// # Errors
    /// Identifies malformed triangle indices, nonfinite referenced positions or
    /// nonrepresentable arithmetic. No partial measurement is returned.
    pub fn signed_volume(&self) -> Result<SignedVolume, VolumeError> {
        exedra_mesh_ops::measure::signed_volume(&self.tri)
    }

    /// Axis-aligned bounds of the emitted render positions in part-local space.
    ///
    /// Bounds are derived on demand from [`TriMesh::positions`], after
    /// constructive evaluation and its `f64`-to-`f32` emission boundary. They
    /// therefore describe exactly the geometry a renderer receives. Occurrence
    /// placement has not been applied. Returns `None` for an empty
    /// body.
    ///
    /// # Example
    ///
    /// ```
    /// use exedra_mesh::TriMesh;
    /// use exedra_compile::CompiledBody;
    ///
    /// let body = CompiledBody {
    ///     tri: TriMesh {
    ///         positions: vec![[-2.0, 1.0, 4.0], [3.0, 5.0, -1.0]],
    ///         ..TriMesh::default()
    ///     },
    ///     regions: vec![],
    ///     extraction: Default::default(),
    /// };
    /// let bounds = body.bounds().expect("two emitted positions");
    /// assert_eq!(bounds.min, [-2.0, 1.0, -1.0]);
    /// assert_eq!(bounds.max, [3.0, 5.0, 4.0]);
    /// ```
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb3> {
        bounds_of_positions(&self.tri.positions)
    }
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
    /// Total compiled triangle count across all bodies in this part.
    ///
    /// Each triangle is counted once, regardless of how many assembly
    /// instances place the part. Placed counting belongs to the caller.
    #[must_use]
    pub fn triangle_count(&self) -> u64 {
        self.bodies
            .iter()
            .map(|b| (b.tri.indices.len() / 3) as u64)
            .sum()
    }

    /// Axis-aligned union of every body bound in part-local space.
    ///
    /// Like [`CompiledBody::bounds`], this is derived on demand from emitted
    /// render positions and excludes occurrence placement. Empty bodies
    /// do not affect the union; a part with no render positions returns `None`.
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb3> {
        let mut bounds = Aabb3::EMPTY;
        for body in self.bodies.iter().filter_map(CompiledBody::bounds) {
            bounds.union(&body);
        }
        (!bounds.is_empty()).then_some(bounds)
    }
}

fn bounds_of_positions(positions: &[[f32; 3]]) -> Option<Aabb3> {
    let mut positions = positions.iter();
    let first = positions.next()?.map(f64::from);
    let mut bounds = Aabb3 {
        min: first,
        max: first,
    };
    for position in positions {
        for (axis, component) in position.iter().copied().map(f64::from).enumerate() {
            bounds.min[axis] = bounds.min[axis].min(component);
            bounds.max[axis] = bounds.max[axis].max(component);
        }
    }
    Some(bounds)
}

/// Introspection counters accumulated across a compiler's lifetime.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct CompileCounters {
    /// Parts tessellated (cache misses).
    pub parts_compiled: u64,
    /// Explicit native body realizations performed on cache misses/upgrades.
    pub native_realizations: u64,
    /// Cache hits (a compiled entry was reused).
    pub cache_hits: u64,
    /// Cache entries evicted explicitly or to satisfy the entry budget.
    pub cache_evictions: u64,
    /// Triangles emitted by cache-miss compilations.
    pub triangles_emitted: u64,
    /// Carried attribute values that used their missing value, summed over
    /// cache-miss compilations ([`ExtractStats::attribute_fallback_count`]).
    pub attribute_fallbacks: u64,
    /// Carried attributes with no layer of the requested type, summed per
    /// body over cache-miss compilations
    /// ([`ExtractStats::missing_attribute_layers`]).
    pub missing_attribute_layers: u64,
    /// Render vertices given the fallback tangent, summed over cache-miss
    /// compilations ([`ExtractStats::tangent_fallback_count`]).
    pub tangent_fallbacks: u64,
    /// Bodies whose tangent UV stream was not carried, summed over cache-miss
    /// compilations ([`ExtractStats::missing_tangent_uv_sets`]). Nonzero
    /// means [`CompilePolicy::tangents`] names a key that
    /// [`CompilePolicy::attributes`] does not carry.
    pub missing_tangent_uv_sets: u64,
}

/// Native source retention owned by this compiler cache, independently of
/// display buffers and native installations held by external snapshots.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeCacheStorage {
    /// Distinct installed source allocations retained by query cache entries.
    /// Multiple tolerances sharing one source count it once.
    pub sources: usize,
    /// Cached realizations retaining native mesh/source correspondence.
    pub realizations: usize,
    /// Combined live native boundary-table payload for distinct source allocations.
    /// Excludes constructor records, spare capacity, allocator overhead and all
    /// realization/render buffers; see [`crate::NativeBoundaryStorage`].
    pub boundary_payload_bytes: usize,
}

/// Typed compilation failure.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CompileError {
    /// Native body realization was refused; the installed source is unchanged.
    Native {
        /// Failing input-local part.
        part: PartId,
        /// Owned caller diagnostic key.
        part_key: String,
        /// Original analytic realization diagnosis.
        error: Box<exedra_analytic::realize::RealizationError>,
    },
    /// A recipe part failed to evaluate.
    Evaluate {
        /// The failing part.
        part: PartId,
        /// Owned authored key, retained independently of the input list.
        part_key: String,
        /// The underlying evaluation failure.
        error: Box<EvalError>,
    },
    /// Evaluation explicitly refused a recipe and produced no geometry.
    NoGeometry {
        /// The rejected part.
        part: PartId,
        /// Owned authored key, retained independently of the input list.
        part_key: String,
        /// The complete constructive report, including the refusal reason.
        report: Arc<GeometryReport>,
    },
}

impl core::fmt::Display for CompileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Native {
                part,
                part_key,
                error,
            } => write!(
                f,
                "native part {part_key:?} ({part:?}) failed to realize: {error}"
            ),
            Self::Evaluate {
                part,
                part_key,
                error,
            } => {
                write!(
                    f,
                    "part {part_key:?} ({part:?}) failed to evaluate: {error}"
                )
            }
            Self::NoGeometry {
                part,
                part_key,
                report,
            } => {
                let diagnostic = report
                    .diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic.severity >= Severity::Error);
                if let Some(diagnostic) = diagnostic {
                    write!(
                        f,
                        "part {part_key:?} ({part:?}) produced no geometry: {}: {}",
                        diagnostic.code, diagnostic.message
                    )
                } else {
                    write!(f, "part {part_key:?} ({part:?}) produced no geometry")
                }
            }
        }
    }
}

impl core::error::Error for CompileError {}

/// The compiled view of an explicit geometry input list.
///
/// Geometry, reports and handle tables share immutable [`Arc`] ownership.
/// Cloning this snapshot takes constant time without copying buffers or diagnostics. Snapshots
/// are [`Send`] and [`Sync`] and remain usable after their compiler is dropped
/// or its cache is cleared.
#[derive(Clone, Debug)]
pub struct CompiledParts {
    /// Compiled entry per part, indexed by [`PartId`].
    parts: Arc<[Arc<CompiledPart>]>,
    /// Constructive report per recipe part, parallel to `parts`.
    reports: Arc<[Option<Arc<GeometryReport>>]>,
    policy: PolicyFingerprint,
}

impl CompiledParts {
    /// The compiled entry for a part.
    #[must_use]
    pub fn part(&self, id: PartId) -> Option<&Arc<CompiledPart>> {
        self.parts.get(id.0 as usize)
    }

    /// All compiled entries in [`PartId`] order.
    #[must_use]
    pub fn parts(&self) -> &[Arc<CompiledPart>] {
        &self.parts
    }

    /// The constructive evaluation report for a recipe part.
    ///
    /// Baked parts have no constructive evaluation and return `None`, as do
    /// unknown part ids. A cache hit returns the content-addressed report
    /// captured with the compiled part; its evaluation counters therefore
    /// describe the original compilation. Use [`PartCompiler::counters`] to
    /// measure work performed by the current compiler call.
    #[must_use]
    pub fn report(&self, id: PartId) -> Option<&GeometryReport> {
        self.reports.get(id.0 as usize).and_then(Option::as_deref)
    }
}

/// One successful content-addressed cache entry.
///
/// The report is cached beside the geometry rather than recomputed on a hit:
/// diagnostics are part of the compilation result, not incidental logging.
#[derive(Clone, Debug)]
struct CachedCompilation {
    used: u64,
    evaluated: Option<Arc<EvaluatedPart>>,
    part: Arc<CompiledPart>,
    report: Option<Arc<GeometryReport>>,
    // Keeps the original immutable native installation when retaining queries.
    // Equal-content new installations bind their own scope in snapshots.
    native_source: Option<crate::NativeSource>,
}

/// Memoizing compiler for explicit geometry inputs, independent of scene structure.
///
/// The compiler is [`Send`] and [`Sync`]; mutation requires exclusive access.
/// A worker can own it and publish [`CompiledParts`] to other threads without
/// copying geometry. Shared ownership uses [`alloc::sync::Arc`], requiring
/// pointer-width atomic operations even when the `std` feature is disabled.
#[derive(Debug)]
pub struct PartCompiler {
    cache: HashMap<(PartFingerprint, PolicyFingerprint), CachedCompilation>,
    /// Keys in the most recent successful compilation, including cache hits.
    /// Part IDs identify only that snapshot; older variants remain content keyed.
    part_keys: Vec<(PartFingerprint, PolicyFingerprint)>,
    dirty: BTreeSet<PartId>,
    capacity: usize,
    clock: u64,
    counters: CompileCounters,
}

impl Default for PartCompiler {
    fn default() -> Self {
        Self::with_capacity(usize::MAX)
    }
}

impl PartCompiler {
    /// Creates an empty compiler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a compiler with an explicit content/policy entry budget, minimum one.
    /// Held snapshots can retain entries after eviction; this bounds only cache retention.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: HashMap::new(),
            part_keys: Vec::new(),
            dirty: BTreeSet::new(),
            counters: CompileCounters::default(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    /// Cache entries retaining topology and source correspondence for queries.
    #[must_use]
    pub fn retained_entries(&self) -> usize {
        self.cache
            .values()
            .filter(|entry| entry.evaluated.is_some())
            .count()
    }

    /// Surveys cache-owned native sources without counting externally held
    /// snapshots or deduplicated source storage once per display tolerance.
    #[must_use]
    pub fn native_storage(&self) -> NativeCacheStorage {
        let mut sources = hashbrown::HashSet::new();
        let mut result = NativeCacheStorage::default();
        for entry in self.cache.values() {
            if let Some(source) = &entry.native_source {
                result.realizations += 1;
                if sources.insert(source.installation_key()) {
                    result.sources += 1;
                    result.boundary_payload_bytes += source.boundary_storage().payload_bytes;
                }
            }
        }
        result
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

    /// Marks a part from the most recent successful compilation for eviction.
    ///
    /// Call to evict a previously compiled geometry input. The next compilation evicts
    /// its previous content/policy entry, including entries first obtained on a
    /// cache hit. Equal-content parts share an entry, so evicting one releases
    /// the cached entry for all of its aliases. Older variants remain cached.
    ///
    /// Handles belong to the last successful compilation, not to a persistent
    /// occurrence. When rebuilding input lists, content addressing alone keeps
    /// results correct; do not reinterpret their new IDs as old invalidations.
    /// Marking controls cache retention, not geometry correctness.
    pub fn mark_part_changed(&mut self, part: PartId) {
        self.dirty.insert(part);
    }

    /// Releases all cached geometry and reports while retaining lifetime counters.
    ///
    /// Existing [`CompiledParts`] snapshots keep their own shared ownership and
    /// remain usable. This also clears pending invalidations and local handle
    /// associations. Callers can use [`Self::cached_entries`] to choose when to
    /// release cache retention; this does not bound memory held by old snapshots.
    pub fn clear_cache(&mut self) {
        self.counters.cache_evictions += self.cache.len() as u64;
        self.cache = HashMap::new();
        self.part_keys = Vec::new();
        self.dirty.clear();
    }

    /// Compiles every geometry input under `policy`, reusing cached
    /// entries whenever content and policy fingerprints match.
    ///
    /// Inputs with equal content in one call share one compilation and its
    /// buffers, whatever the cache capacity.
    ///
    /// # Errors
    ///
    /// Fails when a recipe part fails to evaluate, or when its evaluation
    /// reports an error and emits no bodies. The cache keeps all other
    /// entries.
    pub fn compile_parts(
        &mut self,
        inputs: &[PartInput<'_>],
        policy: &CompilePolicy,
    ) -> Result<CompiledParts, CompileError> {
        self.compile_parts_inner(inputs, policy, false)
            .map(|(parts, _)| parts)
    }

    /// Compiles render buffers and retains their evaluated bodies for queries.
    ///
    /// The returned snapshot contains no placement or appearance state.
    /// Cached topology is shared; upgrading a render-only
    /// cache entry evaluates it once. Subsequent queries never reevaluate it.
    /// Dropping or clearing this compiler does not invalidate a live snapshot.
    ///
    /// # Errors
    ///
    /// Same checked evaluation failures as [`Self::compile_parts`].
    pub fn compile_snapshot(
        &mut self,
        inputs: &[PartInput<'_>],
        policy: &CompilePolicy,
    ) -> Result<GeometrySnapshot, CompileError> {
        let (compiled, evaluated) = self.compile_parts_inner(inputs, policy, true)?;
        Ok(GeometrySnapshot::new(
            compiled,
            evaluated,
            inputs
                .iter()
                .map(|input| match input.source() {
                    PartSource::Native(source) => Some(source.clone()),
                    _ => None,
                })
                .collect(),
        ))
    }

    fn compile_parts_inner(
        &mut self,
        inputs: &[PartInput<'_>],
        policy: &CompilePolicy,
        retain: bool,
    ) -> Result<(CompiledParts, Vec<Arc<EvaluatedPart>>), CompileError> {
        self.drain_dirty();
        let policy_fp = policy_fingerprint(policy);
        let mut out = Vec::with_capacity(inputs.len());
        let mut reports: Vec<Option<Arc<GeometryReport>>> = Vec::with_capacity(inputs.len());
        let mut part_keys = Vec::with_capacity(inputs.len());
        let mut evaluated = Vec::new();
        // Repeated content within one call shares its first result, so
        // eviction under a small capacity never compiles an input twice.
        let mut first_of = HashMap::<PartFingerprint, usize>::new();
        for (index, def) in inputs.iter().enumerate() {
            let id = PartId(crate::len_u32(index));
            let content_fp = def.fingerprint();
            let key = (content_fp, policy_fp);
            part_keys.push(key);
            self.clock = self.clock.checked_add(1).expect("compiler clock overflow");
            if let Some(&first) = first_of.get(&content_fp) {
                self.counters.cache_hits += 1;
                out.push(Arc::clone(&out[first]));
                reports.push(reports[first].as_ref().map(Arc::clone));
                if retain {
                    evaluated.push(Arc::clone(&evaluated[first]));
                }
                continue;
            }
            first_of.insert(content_fp, index);
            if let Some(hit) = self.cache.get_mut(&key)
                && (!retain || hit.evaluated.is_some())
            {
                hit.used = self.clock;
                self.counters.cache_hits += 1;
                out.push(Arc::clone(&hit.part));
                reports.push(hit.report.as_ref().map(Arc::clone));
                if retain {
                    evaluated.push(Arc::clone(hit.evaluated.as_ref().expect("retained hit")));
                }
                continue;
            }
            let (part, report, geometry) =
                compile_source(id, def.key(), def.source(), policy, content_fp, retain)?;
            let compiled = CachedCompilation {
                used: self.clock,
                evaluated: geometry.map(Arc::new),
                part: Arc::new(part),
                report: report.map(Arc::new),
                native_source: if retain {
                    match def.source() {
                        PartSource::Native(source) => Some(source.clone()),
                        _ => None,
                    }
                } else {
                    None
                },
            };
            self.counters.parts_compiled += 1;
            if matches!(def.source(), PartSource::Native(_)) {
                self.counters.native_realizations += 1;
            }
            self.counters.triangles_emitted += compiled.part.triangle_count();
            for body in &compiled.part.bodies {
                self.counters.attribute_fallbacks += body.extraction.attribute_fallback_count;
                self.counters.missing_attribute_layers += body.extraction.missing_attribute_layers;
                self.counters.tangent_fallbacks += body.extraction.tangent_fallback_count;
                self.counters.missing_tangent_uv_sets += body.extraction.missing_tangent_uv_sets;
            }
            self.cache.insert(key, compiled.clone());
            while self.cache.len() > self.capacity {
                let victim = *self
                    .cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .expect("nonempty cache")
                    .0;
                self.cache.remove(&victim);
                self.counters.cache_evictions += 1;
            }
            if retain {
                evaluated.push(compiled.evaluated.expect("retained compilation"));
            }
            out.push(compiled.part);
            reports.push(compiled.report);
        }
        self.part_keys = part_keys;
        Ok((
            CompiledParts {
                parts: out.into(),
                reports: reports.into(),
                policy: policy_fp,
            },
            evaluated,
        ))
    }

    fn drain_dirty(&mut self) {
        // A single explicit set is enough: only geometry content can be evicted.
        let marked = core::mem::take(&mut self.dirty);
        for part in marked {
            if let Some(key) = self.part_keys.get(part.0 as usize)
                && self.cache.remove(key).is_some()
            {
                self.counters.cache_evictions += 1;
            }
        }
    }
}

/// Computes source-content identity, including baked mesh attributes.
#[must_use]
pub fn part_fingerprint(source: &PartSource) -> PartFingerprint {
    match source {
        PartSource::Recipe(recipe) => PartFingerprint(recipe.recipe_fingerprint().0),
        // Assemblies can be rebuilt with the same local ids and generation.
        // Recompute content identity; those counters cannot identify a mesh.
        PartSource::Baked(mesh) => baked_mesh_fingerprint(mesh),
        PartSource::Native(source) => source.fingerprint(),
    }
}

fn compile_source(
    part: PartId,
    part_key: &str,
    source: &PartSource,
    policy: &CompilePolicy,
    fingerprint: PartFingerprint,
    retain: bool,
) -> Result<(CompiledPart, Option<GeometryReport>, Option<EvaluatedPart>), CompileError> {
    let (bodies, report, evaluated) = match source {
        PartSource::Recipe(recipe) => {
            let evaluation =
                evaluate(recipe, &policy.evaluation).map_err(|error| CompileError::Evaluate {
                    part,
                    part_key: String::from(part_key),
                    error: Box::new(error),
                })?;
            // Constructive refusals are represented as reports rather than
            // `EvalError`s. Reject only when an Error leaves the entire part
            // empty: partial geometry stays usable, but its complete report is
            // retained below so the limitation remains visible to callers.
            if evaluation.bodies.is_empty() && !evaluation.report.clean_at(Severity::Error) {
                return Err(CompileError::NoGeometry {
                    part,
                    part_key: String::from(part_key),
                    report: Arc::new(evaluation.report),
                });
            }
            let bodies = evaluation
                .bodies
                .iter()
                .map(|placed| {
                    compile_body(
                        &placed.body.mesh,
                        |face| placed.material_for_face(face).map(|slot| SlotIndex(slot.0)),
                        policy,
                    )
                })
                .collect();
            let geometry =
                retain.then(|| EvaluatedPart::from_evaluation(evaluation.bodies, recipe));
            (bodies, Some(evaluation.report), geometry)
        }
        PartSource::Baked(mesh) => (
            alloc::vec![compile_body(mesh, |_| None, policy)],
            None,
            retain.then(|| EvaluatedPart::from_baked(mesh)),
        ),
        PartSource::Native(source) => {
            let realized =
                source
                    .realize(&policy.native)
                    .map_err(|error| CompileError::Native {
                        part,
                        part_key: String::from(part_key),
                        error: Box::new(error),
                    })?;
            let (compiled, render_mesh_faces) =
                compile_body_with_faces(&realized.mesh, |_| None, policy, retain);
            let geometry = if retain {
                let faces = render_mesh_faces
                    .into_iter()
                    .map(|face| {
                        let at = realized
                            .mesh_faces
                            .binary_search_by_key(&face.index(), |(f, _)| f.index())
                            .expect("native mesh-face correspondence is complete");
                        realized.mesh_faces[at].1
                    })
                    .collect();
                let (mesh, correspondence) =
                    crate::NativeRealization::from_realized(realized, faces);
                Some(EvaluatedPart::from_native(mesh, correspondence))
            } else {
                None
            };
            (alloc::vec![compiled], None, geometry)
        }
    };
    Ok((
        CompiledPart {
            fingerprint,
            bodies,
        },
        report,
        evaluated,
    ))
}

/// Extracts render buffers and regroups the index buffer so each
/// `(FACE_REGION, material slot)` pair is one contiguous range.
fn compile_body(
    mesh: &exedra_mesh::Mesh,
    material_for_face: impl Fn(exedra_mesh::FaceId) -> Option<SlotIndex>,
    policy: &CompilePolicy,
) -> CompiledBody {
    compile_body_with_faces(mesh, material_for_face, policy, false).0
}

fn compile_body_with_faces(
    mesh: &exedra_mesh::Mesh,
    material_for_face: impl Fn(exedra_mesh::FaceId) -> Option<SlotIndex>,
    policy: &CompilePolicy,
    retain_faces: bool,
) -> (CompiledBody, Vec<exedra_mesh::FaceId>) {
    let params = policy.extraction_params();
    let (tri, extraction) = mesh.to_trimesh(&params);
    // Per-triangle regions in extraction order: to_trimesh emits faces in
    // face-id order, so per-face triangle counts line up exactly.
    let regions_layer = mesh.attrs().dense(exedra_mesh::attr::FACE_REGION);
    let triangle_count = tri.indices.len() / 3;
    let mut tri_regions = Vec::with_capacity(triangle_count);
    let mut tri_uvs = Vec::with_capacity(triangle_count);
    let mut tri_faces = if retain_faces {
        Vec::with_capacity(triangle_count)
    } else {
        Vec::new()
    };
    let uv_layer = mesh.attrs().sparse(exedra_mesh::attr::CORNER_UV);
    // A corner counts as textured when the policy resolves a finite UV for
    // it. Authored values are checked at the source; a projected fallback is
    // checked on the emitted buffer, where extraction-order triangle `t`
    // occupies indices `3t..3t + 3`. The zero fallback of `CustomOnly` is not
    // a UV and never counts.
    let is_finite_uv = |uv: &[f32; 2]| uv.iter().all(|v| v.is_finite());
    let corner_has_uv = |corner: exedra_mesh::CornerId, emitted: usize| match uv_layer
        .and_then(|layer| layer.get(corner.into()))
    {
        Some(uv) => is_finite_uv(uv),
        None => match policy.uvs {
            UvSource::CustomOnly => false,
            UvSource::CustomOrBoxProjected { .. } => {
                is_finite_uv(&tri.uvs[tri.indices[emitted] as usize])
            }
        },
    };
    for face in mesh.faces() {
        let (triangles, _) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        let region = regions_layer
            .and_then(|layer| layer.get(face.as_id()).copied())
            .unwrap_or(0);
        let slot = material_for_face(face);
        for triangle in &triangles {
            let base = tri_regions.len() * 3;
            tri_regions.push((region, slot));
            if retain_faces {
                tri_faces.push(face);
            }
            tri_uvs.push(
                triangle
                    .iter()
                    .enumerate()
                    .all(|(k, corner)| corner_has_uv(*corner, base + k)),
            );
        }
    }
    debug_assert_eq!(
        tri_regions.len(),
        triangle_count,
        "per-face counts must match extraction emission"
    );
    // Stable regroup: order triangles by (region, slot, extraction order).
    let mut order: Vec<u32> = (0..crate::len_u32(triangle_count)).collect();
    order.sort_by_key(|&t| (tri_regions[t as usize], t));
    let mut indices = Vec::with_capacity(tri.indices.len());
    let mut regions: Vec<RegionRange> = Vec::new();
    for &t in &order {
        let (region, material_slot) = tri_regions[t as usize];
        let start = crate::len_u32(indices.len());
        match regions.last_mut() {
            Some(last) if last.region == region && last.material_slot == material_slot => {
                last.count += 3;
                last.has_uvs &= tri_uvs[t as usize];
            }
            _ => regions.push(RegionRange {
                region,
                material_slot,
                start,
                count: 3,
                has_uvs: tri_uvs[t as usize],
            }),
        }
        let base = t as usize * 3;
        indices.extend_from_slice(&tri.indices[base..base + 3]);
    }
    let faces = if retain_faces {
        order.iter().map(|&t| tri_faces[t as usize]).collect()
    } else {
        Vec::new()
    };
    (
        CompiledBody {
            tri: TriMesh { indices, ..tri },
            regions,
            extraction,
        },
        faces,
    )
}

/// Canonical baked-mesh identity including the built-in render attributes.
/// Corner attributes rotate together with their owning face loop.
fn baked_mesh_fingerprint(mesh: &exedra_mesh::Mesh) -> PartFingerprint {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&EVAL_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(b"baked-mesh-v3");
    bytes.extend_from_slice(&crate::len_u32(mesh.vertices().count()).to_le_bytes());
    for vertex in mesh.vertices() {
        // Face loops reference slot indices, which need not be contiguous.
        bytes.extend_from_slice(&vertex.index().to_le_bytes());
        let p = mesh.vertex_position(vertex).copied().unwrap_or([0.0; 3]);
        for c in p {
            bytes.extend_from_slice(&c.to_bits().to_le_bytes());
        }
        put_optional_floats(&mut bytes, mesh.vertex_sharpness(vertex).map(|v| [v]));
    }
    bytes.extend_from_slice(&crate::len_u32(mesh.faces().count()).to_le_bytes());
    for face in mesh.faces() {
        let mut corners: Vec<_> = mesh.face_loop(face).collect();
        if let Some(min_pos) = corners
            .iter()
            .enumerate()
            .min_by_key(|(_, he)| mesh.to_vertex(**he).map(exedra_mesh::VertexId::index))
            .map(|(i, _)| i)
        {
            corners.rotate_left(min_pos);
        }
        bytes.extend_from_slice(&crate::len_u32(corners.len()).to_le_bytes());
        let region = mesh
            .attrs()
            .dense(exedra_mesh::attr::FACE_REGION)
            .and_then(|layer| layer.get(face.into()))
            .copied()
            .unwrap_or(0);
        bytes.extend_from_slice(&region.to_le_bytes());
        for he in corners {
            bytes.extend_from_slice(
                &mesh
                    .to_vertex(he)
                    .expect("live face corner")
                    .index()
                    .to_le_bytes(),
            );
            let uv = mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_UV)
                .and_then(|layer| layer.get(he.into()))
                .copied();
            let normal = mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                .and_then(|layer| layer.get(he.into()))
                .copied();
            put_optional_floats(&mut bytes, uv);
            put_optional_floats(&mut bytes, normal);
            put_optional_floats(&mut bytes, mesh.edge_sharpness(he).map(|v| [v]));
            bytes.push(match mesh.edge_seam(he) {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
    }
    put_custom_layers(&mut bytes, mesh);
    PartFingerprint(fnv128(&bytes))
}

/// Appends every caller-defined layer: carried attribute streams read them, so
/// they are compiled content. Layers are ordered by domain and name; values
/// follow live elements in slot order, each with a presence byte so dense
/// defaults and sparse gaps stay distinct.
fn put_custom_layers(bytes: &mut Vec<u8>, mesh: &exedra_mesh::Mesh) {
    use exedra_mesh::attributes::{Domain, LayerKind};
    let domain_tag = |domain: Domain| match domain {
        Domain::Vertex => 0_u8,
        Domain::Face => 1,
        Domain::HalfEdge => 2,
    };
    let mut keys: Vec<_> = mesh
        .attrs()
        .keys()
        .filter(|(domain, name)| !exedra_mesh::attr::is_reserved(*domain, name))
        .collect();
    keys.sort_by_key(|(domain, name)| (domain_tag(*domain), *name));
    bytes.extend_from_slice(&crate::len_u32(keys.len()).to_le_bytes());
    for (domain, name) in keys {
        bytes.push(domain_tag(domain));
        bytes.extend_from_slice(&crate::len_u32(name.len()).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(match mesh.attrs().layer_kind(domain, name) {
            Some(LayerKind::F32) => 0,
            Some(LayerKind::Vec2) => 1,
            Some(LayerKind::Vec3) => 2,
            Some(LayerKind::Vec4) => 3,
            Some(LayerKind::U32) => 4,
            Some(LayerKind::Bool) => 5,
            None => 6,
        });
        let ids: Vec<exedra_mesh::Id> = match domain {
            Domain::Vertex => mesh.vertices().map(|v| v.as_id()).collect(),
            Domain::Face => mesh.faces().map(|f| f.as_id()).collect(),
            Domain::HalfEdge => mesh.half_edges().map(|h| h.as_id()).collect(),
        };
        for id in ids {
            bytes.extend_from_slice(&id.index().to_le_bytes());
            match mesh.attrs().value_words(domain, name, id) {
                Some(value) => {
                    bytes.push(1);
                    for word in value.words() {
                        bytes.extend_from_slice(&word.to_le_bytes());
                    }
                }
                None => bytes.push(0),
            }
        }
    }
}

fn put_optional_floats<const N: usize>(bytes: &mut Vec<u8>, values: Option<[f32; N]>) {
    bytes.push(u8::from(values.is_some()));
    if let Some(values) = values {
        for value in values {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
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

impl AsRef<Self> for CompiledParts {
    fn as_ref(&self) -> &Self {
        self
    }
}

#[cfg(test)]
mod native_mapping_tests {
    use super::*;
    #[test]
    fn retained_face_order_matches_actual_regrouped_render_triangles() {
        let validation = exedra_profile::ProfilePolicy::new(1e-9);
        let profile = exedra_constructive::builders::rect_centered(4.0, 3.0).unwrap();
        let source: crate::NativeSource = exedra_analytic::extrusion::Extrusion::from_profile(
            &profile,
            exedra_math::Placement3::IDENTITY,
            2.0,
            &validation,
        )
        .unwrap()
        .into();
        let mut realized = source
            .realize(&exedra_analytic::realize::RealizationPolicy::default())
            .unwrap();
        let original: Vec<_> = realized.mesh.faces().collect();
        let mut edit = realized.mesh.edit();
        for (i, &face) in original.iter().enumerate() {
            exedra_mesh::op::set_face_region(
                &mut edit,
                face,
                crate::len_u32((original.len() - i) % 3),
            )
            .unwrap();
        }
        let _: () = edit.finish();
        let (compiled, faces) =
            compile_body_with_faces(&realized.mesh, |_| None, &CompilePolicy::default(), true);
        assert_eq!(compiled.regions.len(), 3);
        assert_ne!(faces, original);
        for (ids, &face) in compiled.tri.indices.as_chunks::<3>().0.iter().zip(&faces) {
            let mut actual: Vec<_> = ids
                .iter()
                .map(|&i| compiled.tri.positions[i as usize].map(f32::to_bits))
                .collect();
            let mut expected: Vec<_> = realized
                .mesh
                .face_loop(face)
                .map(|c| {
                    realized
                        .mesh
                        .vertex_position(realized.mesh.to_vertex(c).unwrap())
                        .unwrap()
                        .map(f32::to_bits)
                })
                .collect();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected);
        }
        let (_, omitted) =
            compile_body_with_faces(&realized.mesh, |_| None, &CompilePolicy::default(), false);
        assert!(omitted.is_empty());
    }
}
