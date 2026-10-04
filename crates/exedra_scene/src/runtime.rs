// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, string::String, sync::Arc, vec::Vec};
use exedra_compile::{
    BodyLookupError, CompileError, CompilePolicy, CompiledPart, PartCompiler, PartId, PartInput,
    PartSnapshot, PartSource, SnapshotBody,
};
use exedra_constructive::{
    evaluate::{GeometryReport, Severity},
    ir::Recipe,
};
use layerstack::{
    ChangeCursor, ChangeHistoryError, EditTarget, LayerId, LayerOffset, LayerStack, LayerStore,
    LiveStage, Path, PathId, PropertyPath, Specifier, Transaction,
};
use layerstack_schemas::{Scene, Time};

use crate::{
    BOX_SCHEMA, CYLINDER_SCHEMA, SceneError, SceneErrorKind, geometry::Polygon, primitive,
};

/// Reserved output child below each supported construction prim.
///
/// The generated layer owns this child's geometry properties. Stronger layers
/// may override appearance; they must maintain their own indexed attribute
/// correspondence after topology changes.
pub const GENERATED_CHILD: &str = "__exedra_geometry";

type ContentKey = (u128, u64);

#[derive(Debug)]
struct Cached {
    encoding: Rc<Polygon>,
    used: u64,
}

#[derive(Clone, Copy, Debug)]
struct Published {
    output: PathId,
    content: ContentKey,
}

/// One source's part-local evaluated geometry, independent of scene placement.
///
/// Clones share immutable topology, source maps, buffers and reports. They
/// survive regeneration, cache eviction and evaluator destruction. Array indices
/// and mesh handles are snapshot-local; retain construction intent for selections.
#[derive(Clone, Debug)]
pub struct SourceGeometry {
    /// Store-local source prim handle; use `source` outside this store.
    pub path: PathId,
    /// Owned source path spelling.
    pub source: String,
    /// Store-local conventional output mesh path.
    pub output: PathId,
    geometry: PartSnapshot,
    recipe: Recipe,
}

impl SourceGeometry {
    /// Ordinary immutable recipe lowered from composed construction inputs.
    #[must_use]
    pub fn recipe(&self) -> &Recipe {
        &self.recipe
    }

    /// Shared extraction buffers from the scene-independent compiler.
    #[must_use]
    pub fn compiled(&self) -> &Arc<CompiledPart> {
        self.geometry.compiled()
    }

    /// Complete constructive report, including original measurements on cache hits.
    #[must_use]
    pub fn report(&self) -> &GeometryReport {
        self.geometry.report().expect("scene sources are recipes")
    }

    /// Shared immutable body with the same query APIs as standalone compilation.
    /// Indices and selection scopes belong to this synchronization's snapshot.
    #[must_use]
    pub fn body(&self, index: u32) -> Option<SnapshotBody> {
        self.geometry.body(index)
    }

    /// Resolves a producing source label without retaining transient body indices.
    /// # Errors
    /// Reports missing or ambiguous source labels without choosing one.
    pub fn body_by_source(&self, source: &str) -> Result<SnapshotBody, BodyLookupError> {
        self.geometry.body_by_source(source)
    }

    /// Whether another source shares exactly the same immutable compiled geometry.
    #[must_use]
    pub fn shares_geometry(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.compiled(), other.compiled())
    }
}

/// Actual work performed by one successful explicit synchronization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Composed prims inspected for supported construction types.
    pub discovered_prims: usize,
    /// Geometry compilations performed by the caller-owned shared service.
    pub geometry_evaluations: usize,
    /// Source occurrences that reused a cached geometry result.
    pub geometry_reused: usize,
    /// Native polygon property encodings, independent of shared compilation work.
    pub polygon_encodings: usize,
    /// Output mesh specs created or replaced.
    pub meshes_written: usize,
    /// Output mesh specs removed after their source disappeared.
    pub meshes_removed: usize,
    /// Point values authored by changed output specs.
    pub points_written: usize,
}

/// A successful source-geometry snapshot; all scene authoring has completed.
#[derive(Clone, Debug)]
pub struct SceneSnapshot {
    /// Supported sources in native stage traversal order.
    pub sources: Vec<SourceGeometry>,
    /// Measured work from this sync.
    pub stats: SyncStats,
}

/// Explicit geometry evaluation and publication for one caller-owned live stage.
///
/// Owns only generated publication and native property encoding retention. Source authoring, transforms,
/// materials, composition and undo remain native `LayerStack` operations. No scene
/// tree, scheduling, callbacks or renderer resources are retained.
///
/// Bind to the same live stage/store for this evaluator's lifetime. Its generated
/// layer must start empty and be a dedicated local sublayer, with identity time
/// offset. Do not edit that layer externally; author overrides in stronger layers.
/// Each sync scans the requested subtree, but geometry and output publication are
/// content incremental. The native encoding cache is bounded by entry count.
/// Geometry retention and work belong to an explicitly supplied `PartCompiler`;
/// it can also serve standalone callers or Assembly adapters.
#[derive(Debug)]
pub struct SceneEvaluator {
    root: PathId,
    generated: LayerId,
    generation: Option<u64>,
    cursor: Option<ChangeCursor>,
    published: BTreeMap<PathId, Published>,
    cache: BTreeMap<ContentKey, Cached>,
    capacity: usize,
    clock: u64,
}

impl SceneEvaluator {
    /// Creates an evaluator with room for 128 native property encodings.
    #[must_use]
    pub fn new(root: PathId, generated: LayerId) -> Self {
        Self::with_capacity(root, generated, 128)
    }

    /// Creates an evaluator with an explicit cache-entry budget, minimum one.
    #[must_use]
    pub fn with_capacity(root: PathId, generated: LayerId, capacity: usize) -> Self {
        Self {
            root,
            generated,
            generation: None,
            cursor: None,
            published: BTreeMap::new(),
            cache: BTreeMap::new(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    /// Content/policy pairs retained in the native property encoding cache.
    #[must_use]
    pub fn cached_entries(&self) -> usize {
        self.cache.len()
    }

    /// Approximate encoded geometry bytes retained by the cache.
    ///
    /// Counts numeric property buffers, excluding topology, reports, maps,
    /// allocation overhead and ownership retained by external snapshots.
    #[must_use]
    pub fn cached_payload_bytes(&self) -> usize {
        self.cache
            .values()
            .map(|entry| entry.encoding.payload_bytes)
            .sum()
    }

    /// Releases native encodings; geometry compiler retention is caller-owned.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Samples construction inputs and atomically updates changed generated meshes.
    ///
    /// Calls the supplied shared compiler without baking scene transforms into
    /// inputs. Geometry keys include source content and the full compilation policy.
    /// The compiler can also serve standalone callers or other native stages.
    /// Stronger opinions survive regeneration; source edit inverses can be applied
    /// normally, followed by another sync to regenerate the corresponding output.
    /// Source-local geometry and reports remain available in the returned snapshot.
    ///
    /// # Errors
    ///
    /// Refuses invalid parameters, incomplete composition, native instance
    /// descendants, output collisions and generated-layer ownership violations.
    /// Preflight/evaluation failures leave the previous generated layer intact;
    /// useful cache entries may remain. Publication uses an atomic transaction.
    pub fn sync(
        &mut self,
        live: &mut LiveStage,
        store: &mut dyn LayerStore,
        compiler: &mut PartCompiler,
        time: Time,
        policy: &CompilePolicy,
    ) -> Result<SceneSnapshot, SceneError> {
        let error = |kind| SceneError::new(store, self.root, kind);
        if let Time::At { code, .. } = time
            && !code.is_finite()
        {
            return Err(error(SceneErrorKind::InvalidTime));
        }
        if let Some(cursor) = self.cursor.as_mut() {
            // Discovery below is a complete refresh, so expired history is safe.
            match live.changes_since(cursor) {
                Ok(changes) => for _ in changes {},
                Err(ChangeHistoryError::Expired) => {}
                Err(e) => return Err(error(SceneErrorKind::Stage(e))),
            }
        }
        // Encoded properties contain store-local tokens. Bind even when a later
        // failure leaves useful cache entries, so they cannot escape this stage.
        self.cursor = Some(live.change_cursor());
        live.synchronize(store);
        let stage = live.stage();
        if !stage.composition_errors().is_empty() {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::Composition {
                    errors: stage.composition_errors().len(),
                },
            ));
        }
        let Some(registry) = stage.schemas() else {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::MissingSchemas,
            ));
        };
        if store
            .tokens()
            .lookup("Mesh")
            .and_then(|name| registry.schema(name))
            .is_none()
            || store
                .tokens()
                .lookup(BOX_SCHEMA)
                .and_then(|name| registry.schema(name))
                .is_none()
            || store
                .tokens()
                .lookup(CYLINDER_SCHEMA)
                .and_then(|name| registry.schema(name))
                .is_none()
        {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::MissingSchemas,
            ));
        }
        let layer = store
            .layer(self.generated)
            .ok_or_else(|| SceneError::new(store, self.root, SceneErrorKind::GeneratedLayer))?;
        if let Some(generation) = self.generation {
            if generation != layer.generation() {
                return Err(SceneError::new(
                    store,
                    self.root,
                    SceneErrorKind::GeneratedLayerEdited,
                ));
            }
        } else if !layer.prims.is_empty() {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::GeneratedLayer,
            ));
        }
        let stack = LayerStack::gather(store, stage.root_layer().expect("composed stage has root"));
        let generated_index = stack
            .layers
            .iter()
            .position(|layer| *layer == self.generated);
        if generated_index
            .is_none_or(|index| index == 0 || stack.offsets[index] != LayerOffset::IDENTITY)
            || stack
                .layers
                .iter()
                .filter(|layer| **layer == self.generated)
                .count()
                != 1
        {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::GeneratedLayer,
            ));
        }
        let scene = Scene::new(stage, store);
        let mut stats = SyncStats::default();
        let mut inputs = Vec::new();
        // All source checks precede evaluation and generated-layer authoring.
        for path in stage.traverse(self.root) {
            stats.discovered_prims += 1;
            if !stage.is_defined(path, store)
                || stage.is_abstract(path, store)
                || !(scene.is_a(path, BOX_SCHEMA) || scene.is_a(path, CYLINDER_SCHEMA))
            {
                continue;
            }
            let mut ancestor = Some(path);
            while let Some(parent) = ancestor {
                if store
                    .paths()
                    .resolve(parent)
                    .leaf()
                    .is_some_and(|name| store.tokens().resolve(name) == GENERATED_CHILD)
                {
                    return Err(SceneError::new(store, path, SceneErrorKind::GeneratedInput));
                }
                if stage.is_instance(parent) {
                    return Err(SceneError::new(store, path, SceneErrorKind::NativeInstance));
                }
                ancestor = store.paths().parent(parent);
            }
            let recipe = primitive::read(scene, path, time)?;
            inputs.push((
                path,
                store.paths().display(path, store.tokens()),
                PartSource::Recipe(recipe),
            ));
        }
        let compilation_inputs: Vec<_> = inputs
            .iter()
            .map(|(_, key, source)| PartInput::new(key, source))
            .collect();
        let before = compiler.counters();
        let geometry = compiler
            .compile_snapshot(&compilation_inputs, policy)
            .map_err(|error| {
                let part = match &error {
                    CompileError::Evaluate { part, .. } | CompileError::NoGeometry { part, .. } => {
                        *part
                    }
                    _ => PartId(0),
                };
                let path = inputs
                    .get(part.0 as usize)
                    .map_or(self.root, |input| input.0);
                SceneError::new(store, path, SceneErrorKind::Compile(Box::new(error)))
            })?;
        let after = compiler.counters();
        stats.geometry_evaluations = usize::try_from(after.parts_compiled - before.parts_compiled)
            .expect("compilations bounded by input count");
        stats.geometry_reused = usize::try_from(after.cache_hits - before.cache_hits)
            .expect("hits bounded by input count");
        let target = EditTarget::for_layer(self.generated);
        let mut transaction = Transaction::new();
        let mut published = BTreeMap::new();
        let mut sources = Vec::new();
        for (index, (path, source, input)) in inputs.into_iter().enumerate() {
            let part = PartId(u32::try_from(index).expect("compiler input count fits u32"));
            let compiled = geometry
                .compiled()
                .part(part)
                .expect("compiled scene input");
            let report = geometry
                .compiled()
                .report(part)
                .expect("scene inputs are recipes");
            if !report.clean_at(Severity::Error) || compiled.bodies.len() != 1 {
                return Err(SceneError::new(
                    store,
                    path,
                    SceneErrorKind::Refused(Arc::new(report.clone())),
                ));
            }
            let key = (
                compiled.fingerprint.0,
                geometry.compiled().policy_fingerprint().0,
            );
            self.clock = self
                .clock
                .checked_add(1)
                .expect("scene encoding cache clock overflow");
            let encoding = if let Some(cached) = self.cache.get_mut(&key) {
                cached.used = self.clock;
                Rc::clone(&cached.encoding)
            } else {
                let body = geometry.body(part, 0).expect("one retained scene body");
                let encoding = Rc::new(
                    Polygon::new(&body.geometry().mesh, policy, store.tokens_mut())
                        .map_err(|kind| SceneError::new(store, path, kind))?,
                );
                stats.polygon_encodings += 1;
                self.cache.insert(
                    key,
                    Cached {
                        encoding: Rc::clone(&encoding),
                        used: self.clock,
                    },
                );
                while self.cache.len() > self.capacity {
                    let victim = *self
                        .cache
                        .iter()
                        .min_by_key(|(_, value)| value.used)
                        .expect("nonempty cache")
                        .0;
                    self.cache.remove(&victim);
                }
                encoding
            };
            let previous = self.published.get(&path);
            let output = previous.map_or_else(
                || {
                    let spelling = alloc::format!("{source}/{GENERATED_CHILD}");
                    let output = Path::parse_absolute(&spelling, store.tokens_mut())
                        .expect("source paths are absolute");
                    store.paths_mut().intern(output)
                },
                |previous| previous.output,
            );
            // A reference can already compose output generated for another
            // source. It is safe to specialize that output locally. Unrelated
            // authored geometry at the reserved child is a collision.
            if previous.is_none()
                && stage.has_prim(output)
                && !stage
                    .prim_stack(output)
                    .is_some_and(|stack| stack.iter().any(|(layer, _)| *layer == self.generated))
            {
                return Err(SceneError::new(
                    store,
                    output,
                    SceneErrorKind::OutputCollision,
                ));
            }
            if previous.is_none_or(|previous| previous.content != key) {
                if previous.is_some() {
                    transaction.remove_spec(target.prim(output));
                }
                let mesh_type = store.tokens_mut().intern("Mesh");
                transaction.create_prim(target.prim(output), Specifier::Def, Some(mesh_type));
                for (name, spec) in &encoding.properties {
                    let name = store.tokens_mut().intern(name);
                    transaction.create_property(
                        target.property(PropertyPath::new(output, name)),
                        spec.clone(),
                    );
                }
                stats.meshes_written += 1;
                stats.points_written += encoding.points;
            }
            published.insert(
                path,
                Published {
                    output,
                    content: key,
                },
            );
            sources.push(SourceGeometry {
                path,
                source,
                output,
                geometry: geometry.part(part).expect("retained scene input"),
                recipe: match input {
                    PartSource::Recipe(recipe) => recipe,
                    PartSource::Baked(_) => unreachable!("scene schemas lower recipes"),
                },
            });
        }
        for (path, previous) in &self.published {
            if !published.contains_key(path) {
                transaction.remove_spec(target.prim(previous.output));
                stats.meshes_removed += 1;
            }
        }
        if !transaction.is_empty() {
            live.apply(store, &transaction)
                .map_err(|e| SceneError::new(store, self.root, SceneErrorKind::Publish(e)))?;
        }
        self.published = published;
        self.generation = Some(
            store
                .layer(self.generated)
                .expect("validated generated layer")
                .generation(),
        );
        self.cursor = Some(live.change_cursor());
        Ok(SceneSnapshot { sources, stats })
    }
}
