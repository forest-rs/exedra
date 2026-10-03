// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use exedra_constructive::{
    cache::policy_fingerprint,
    evaluate::{Evaluation, Severity, evaluate},
    ir::Recipe,
    tessellate::EvalPolicy,
};
use layerstack::{
    ChangeCursor, ChangeHistoryError, EditTarget, LayerId, LayerOffset, LayerStack, LayerStore,
    LiveStage, Path, PathId, PropertyPath, Specifier, Transaction,
};
use layerstack_schemas::{Scene, Time};

use crate::{
    BOX_SCHEMA, CYLINDER_SCHEMA, SceneError, SceneErrorKind, geometry::Geometry, primitive,
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
    geometry: Rc<Geometry>,
    used: u64,
}

#[derive(Clone, Copy, Debug)]
struct Published {
    output: PathId,
    content: ContentKey,
}

/// One source's part-local evaluated geometry, independent of scene placement.
///
/// Clones share immutable recipes, topology, source maps and reports. They
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
    geometry: Rc<Geometry>,
}

impl SourceGeometry {
    /// The ordinary immutable recipe lowered from composed construction inputs.
    #[must_use]
    pub fn recipe(&self) -> &Recipe {
        &self.geometry.recipe
    }

    /// Geometry and complete constructive report from this content's evaluation.
    ///
    /// Reports describe the original evaluation on a cache hit. Use the sync
    /// statistics to measure work in the current call. Geometry stays source-local;
    /// query native transforms separately for placed measurements.
    #[must_use]
    pub fn evaluation(&self) -> &Evaluation {
        &self.geometry.evaluation
    }

    /// Whether another source shares exactly this immutable evaluated result.
    #[must_use]
    pub fn shares_geometry(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.geometry, &other.geometry)
    }
}

/// Actual work performed by one successful explicit synchronization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Composed prims inspected for supported construction types.
    pub discovered_prims: usize,
    /// Geometry evaluations and polygon encodings performed on cache misses.
    pub geometry_evaluations: usize,
    /// Source occurrences that reused a cached geometry result.
    pub geometry_reused: usize,
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
/// Owns only the generated layer and content cache. Source authoring, transforms,
/// materials, composition and undo remain native `LayerStack` operations. No scene
/// tree, scheduling, callbacks or renderer resources are retained.
///
/// Bind to the same live stage/store for this evaluator's lifetime. Its generated
/// layer must start empty and be the weakest local sublayer, with identity time
/// offset. Do not edit that layer externally; author overrides in stronger layers.
/// Each sync scans the requested subtree, but geometry and output publication are
/// content incremental. The cache is bounded by entry count; held snapshots may
/// retain evicted entries. Current scene snapshots use single-threaded `Rc`,
/// matching direct constructive evaluation.
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
    /// Creates an evaluator with room for 128 geometry content/policy pairs.
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

    /// Content/policy pairs retained in the evaluation cache.
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
            .map(|entry| entry.geometry.payload_bytes)
            .sum()
    }

    /// Releases the content cache; existing snapshots and published geometry survive.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Samples construction inputs and atomically updates changed generated meshes.
    ///
    /// Uses the ordinary constructive evaluator and never bakes scene transforms
    /// into its inputs. Geometry keys include the recipe and evaluation policy.
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
        time: Time,
        policy: &EvalPolicy,
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
        if stack.layers.last() != Some(&self.generated)
            || stack
                .layers
                .iter()
                .filter(|layer| **layer == self.generated)
                .count()
                != 1
            || stack.offsets.last() != Some(&LayerOffset::IDENTITY)
            || stack.layers.len() < 2
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
            inputs.push((path, store.paths().display(path, store.tokens()), recipe));
        }
        let target = EditTarget::for_layer(self.generated);
        let mut transaction = Transaction::new();
        let mut published = BTreeMap::new();
        let mut sources = Vec::new();
        for (path, source, recipe) in inputs {
            let key = (recipe.recipe_fingerprint().0, policy_fingerprint(policy));
            self.clock = self
                .clock
                .checked_add(1)
                .expect("scene cache clock overflow");
            let geometry = if let Some(cached) = self.cache.get_mut(&key) {
                cached.used = self.clock;
                stats.geometry_reused += 1;
                Rc::clone(&cached.geometry)
            } else {
                let evaluation = evaluate(&recipe, policy).map_err(|e| {
                    SceneError::new(store, path, SceneErrorKind::Evaluate(Box::new(e)))
                })?;
                if !evaluation.report.clean_at(Severity::Error) {
                    return Err(SceneError::new(
                        store,
                        path,
                        SceneErrorKind::Refused(Rc::new(evaluation.report)),
                    ));
                }
                let geometry = Rc::new(
                    Geometry::new(recipe, evaluation, store.tokens_mut())
                        .map_err(|kind| SceneError::new(store, path, kind))?,
                );
                stats.geometry_evaluations += 1;
                self.cache.insert(
                    key,
                    Cached {
                        geometry: Rc::clone(&geometry),
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
                geometry
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
                for (name, spec) in &geometry.properties {
                    let name = store.tokens_mut().intern(name);
                    transaction.create_property(
                        target.property(PropertyPath::new(output, name)),
                        spec.clone(),
                    );
                }
                stats.meshes_written += 1;
                stats.points_written += geometry.points;
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
                geometry,
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
