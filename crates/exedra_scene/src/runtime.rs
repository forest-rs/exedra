// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    string::String,
    sync::Arc,
    vec::Vec,
};
use exedra_compile::{
    BodyLookupError, CompileError, CompilePolicy, CompiledPart, PartCompiler, PartId, PartInput,
    PartSnapshot, PartSource, PreparedSource, SnapshotBody,
};
use exedra_constructive::{
    evaluate::{GeometryReport, Severity},
    ir::Recipe,
};
use layerstack::{Address, PropertyKind, PropertySpec, PropertyType};
use layerstack::{
    ChangeCursor, ChangeHistoryError, EditTarget, LayerStore, LiveStage, Path, PathId, PrimSpec,
    PropertyPath, StoreIdentity, Transaction, TypedArray, Value,
};
use layerstack_schemas::{
    Scene, Time,
    procedural::{Procedural, ProceduralError, ProceduralWork},
};

use crate::{
    BOX_SCHEMA, CYLINDER_SCHEMA, SceneError, SceneErrorKind, geometry::Polygon, primitive,
};

/// Reserved output child below each supported construction prim.
///
/// The evaluator owns this child's published geometry properties. Stronger layers
/// may override appearance; they must maintain their own indexed attribute
/// correspondence after topology changes.
pub const GENERATED_CHILD: &str = "__exedra_geometry";

type ContentKey = (u128, u64);

#[derive(Debug)]
struct Cached {
    encoding: Rc<Polygon>,
    used: u64,
}

#[derive(Clone, Debug)]
struct Published {
    output: PathId,
    properties: Vec<String>,
    created_prim: bool,
}

#[derive(Debug)]
struct Binding {
    key: String,
    kind: primitive::Lowering,
    lowering: Procedural<primitive::Lowering>,
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
    intent: Arc<PreparedSource>,
}

impl SourceGeometry {
    /// Ordinary immutable recipe lowered from composed construction inputs.
    #[must_use]
    pub fn recipe(&self) -> &Recipe {
        match self.intent.source() {
            PartSource::Recipe(recipe) => recipe,
            PartSource::Baked(_) => unreachable!("scene schemas lower recipes"),
        }
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
    /// Input recipes lowered after consumed composed values changed.
    pub recipe_evaluations: usize,
    /// Composed attribute resolutions, including input revalidation.
    pub input_resolutions: usize,
    /// Input reads skipped through retained composition evidence.
    pub input_reused: usize,
    /// Producer-owned output properties inspected during preparation.
    pub publication_properties: usize,
    /// Full polygon snapshot validations, performed once per encoding.
    pub polygon_validations: usize,
    /// Output meshes with authored property or declaration changes.
    pub meshes_written: usize,
    /// Outputs retired after their source disappeared; partner opinions survive.
    pub meshes_removed: usize,
    /// Point values authored by changed output specs.
    pub points_written: usize,
    /// Boundable recipe extents authored or repaired.
    pub source_bounds_written: usize,
}

impl SyncStats {
    fn record_inputs(&mut self, before: ProceduralWork, after: ProceduralWork) {
        let count = |delta| usize::try_from(delta).expect("work bounded by primitive input count");
        self.recipe_evaluations += count(after.evaluations - before.evaluations);
        self.input_resolutions += count(after.input_reads - before.input_reads);
        self.input_reused += count(after.query_cache_hits - before.query_cache_hits);
    }
}

/// A successful source-geometry snapshot; all scene authoring has completed.
#[derive(Clone, Debug)]
pub struct SceneSnapshot {
    /// Supported sources in native stage traversal order.
    pub sources: Vec<SourceGeometry>,
    /// Measured work from this sync.
    pub stats: SyncStats,
    /// Native changes authored by this publication. Deliver prior source-edit
    /// reports separately when updating native bounds, transforms or other caches.
    pub changes: layerstack::Changes,
}

/// Explicit geometry evaluation and publication on a caller-owned live stage.
///
/// Retains tracked recipe inputs, producer-owned output manifests and a bounded
/// cache of validated polygon snapshots. The host owns the stage, store, edit
/// target, scheduling, layer order and shared `PartCompiler`. Partners may author
/// unrelated opinions in the same layer or output prim.
///
/// Bind to the store's token/path domains at construction. Replacing the live
/// stage within that store refreshes discovery; another store is refused before
/// interpreting handles. Structural changes beneath `root` refresh membership;
/// ordinary edits reuse the inventory and composed input evidence.
#[derive(Debug)]
pub struct SceneEvaluator {
    store: StoreIdentity,
    root: PathId,
    target: EditTarget,
    cursor: Option<ChangeCursor>,
    discovery_dirty: bool,
    bindings: BTreeMap<PathId, Binding>,
    order: Vec<PathId>,
    published: BTreeMap<PathId, Published>,
    cache: BTreeMap<ContentKey, Cached>,
    capacity: usize,
    clock: u64,
}

impl SceneEvaluator {
    /// Binds one subtree and explicit publication target to this store.
    /// Retains at most 128 validated native polygon snapshots.
    #[must_use]
    pub fn new(store: &dyn LayerStore, root: PathId, target: EditTarget) -> Self {
        Self::with_capacity(store, root, target, 128)
    }

    /// Creates an evaluator with an explicit encoding-entry budget, minimum one.
    #[must_use]
    pub fn with_capacity(
        store: &dyn LayerStore,
        root: PathId,
        target: EditTarget,
        capacity: usize,
    ) -> Self {
        Self {
            store: store.identity(),
            root,
            target,
            cursor: None,
            discovery_dirty: true,
            bindings: BTreeMap::new(),
            order: Vec::new(),
            published: BTreeMap::new(),
            cache: BTreeMap::new(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    /// Caller-selected native publication target; this evaluator never changes it.
    #[must_use]
    pub fn edit_target(&self) -> &EditTarget {
        &self.target
    }

    /// Validated content/policy encodings retained in the bounded cache.
    #[must_use]
    pub fn cached_entries(&self) -> usize {
        self.cache.len()
    }

    /// Approximate numeric buffer bytes retained by the encoding cache.
    /// Excludes allocator overhead and owners held by native layers or snapshots.
    #[must_use]
    pub fn cached_payload_bytes(&self) -> usize {
        self.cache
            .values()
            .map(|entry| entry.encoding.payload_bytes)
            .sum()
    }

    /// Releases native encodings without clearing input bindings or the shared compiler.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Samples tracked recipe inputs and atomically updates native polygon outputs.
    ///
    /// Equal composed inputs reuse lowered recipes, even after masked edits.
    /// Geometry identity includes the complete compilation policy. Publication
    /// checks actual authored output on every call, repairing deleted or replaced
    /// owned values without reevaluating unchanged construction. Other properties,
    /// prim metadata, children and stronger layers survive updates and retirement.
    /// Native instances consume authored-source geometry without proxy authoring.
    ///
    /// # Errors
    /// Refuses foreign stores, invalid parameters, incomplete composition,
    /// unmappable targets and incompatible output declarations. Evaluation and
    /// preparation failures preserve all previously published opinions. Consumed
    /// input evidence is verified before one guarded atomic transaction is applied.
    pub fn sync(
        &mut self,
        live: &mut LiveStage,
        store: &mut dyn LayerStore,
        compiler: &mut PartCompiler,
        time: Time,
        policy: &CompilePolicy,
    ) -> Result<SceneSnapshot, SceneError> {
        let identity = store.identity();
        if self.store != identity || live.stage().store_identity() != Some(&identity) {
            return Err(SceneError {
                path: String::new(),
                kind: SceneErrorKind::ForeignStore,
            });
        }
        if let Time::At { code, .. } = time
            && !code.is_finite()
        {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::InvalidTime,
            ));
        }
        live.synchronize(store);
        let mut discover = self.cursor.is_none();
        if let Some(cursor) = self.cursor.as_mut() {
            match live.changes_since(cursor) {
                Ok(changes) => {
                    for change in changes {
                        for path in change
                            .created
                            .iter()
                            .chain(&change.removed)
                            .chain(&change.resynced)
                        {
                            let paths = store.paths();
                            if paths.resolve(*path).is_prefix_of(paths.resolve(self.root))
                                || paths.resolve(self.root).is_prefix_of(paths.resolve(*path))
                            {
                                discover = true;
                            }
                        }
                    }
                }
                Err(ChangeHistoryError::Expired | ChangeHistoryError::DifferentStage) => {
                    discover = true;
                }
            }
        }
        self.discovery_dirty |= discover;
        self.cursor = Some(live.change_cursor());
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
        if ["Mesh", "GenerativeProcedural", BOX_SCHEMA, CYLINDER_SCHEMA]
            .iter()
            .any(|name| {
                store
                    .tokens()
                    .lookup(name)
                    .and_then(|token| registry.schema(token))
                    .is_none()
            })
        {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::MissingSchemas,
            ));
        }
        if store.layer(self.target.layer()).is_none() {
            return Err(SceneError::new(
                store,
                self.root,
                SceneErrorKind::PublicationTarget,
            ));
        }
        let scene = Scene::new(stage, store);
        let mut stats = SyncStats::default();
        if self.discovery_dirty {
            let mut order = Vec::new();
            let mut kinds = Vec::new();
            for path in stage.traverse(self.root) {
                stats.discovered_prims += 1;
                if !stage.is_defined(path, store) || stage.is_abstract(path, store) {
                    continue;
                }
                let kind = if scene.is_a(path, BOX_SCHEMA) {
                    primitive::Lowering::Box
                } else if scene.is_a(path, CYLINDER_SCHEMA) {
                    primitive::Lowering::Cylinder
                } else {
                    continue;
                };
                let mut ancestor = Some(path);
                let mut instance = false;
                while let Some(parent) = ancestor {
                    if store
                        .paths()
                        .resolve(parent)
                        .leaf()
                        .is_some_and(|name| store.tokens().resolve(name) == GENERATED_CHILD)
                    {
                        return Err(SceneError::new(store, path, SceneErrorKind::GeneratedInput));
                    }
                    instance |= stage.is_instance(parent);
                    ancestor = store.paths().parent(parent);
                }
                if instance {
                    continue;
                }
                order.push(path);
                kinds.push(kind);
            }
            let members: BTreeSet<_> = order.iter().copied().collect();
            self.bindings.retain(|path, _| members.contains(path));
            for (&path, kind) in order.iter().zip(kinds) {
                let binding = self.bindings.entry(path).or_insert_with(|| Binding {
                    key: store.paths().display(path, store.tokens()),
                    kind,
                    lowering: Procedural::new(store, path, kind),
                });
                if binding.kind != kind {
                    binding.kind = kind;
                    *binding.lowering.evaluator_mut() = kind;
                }
            }
            self.order = order;
            self.discovery_dirty = false;
        }
        let mut inputs = Vec::new();
        for &path in &self.order {
            let binding = self.bindings.get_mut(&path).expect("discovered binding");
            let before = binding.lowering.work();
            let input = binding.lowering.snapshot(&scene, time).map_err(|error| {
                let kind = match error {
                    ProceduralError::DifferentStore => SceneErrorKind::ForeignStore,
                    ProceduralError::NotProcedural(_) => SceneErrorKind::UnsupportedPrimitive,
                    ProceduralError::System { expected, actual } => {
                        SceneErrorKind::ProceduralSystem { expected, actual }
                    }
                    ProceduralError::Input(error) => SceneErrorKind::Input(error),
                    ProceduralError::Evaluation(error) => error,
                };
                SceneError::new(store, path, kind)
            })?;
            let after = binding.lowering.work();
            stats.record_inputs(before, after);
            inputs.push((path, binding.key.clone(), input));
        }
        let compilation_inputs: Vec<_> = inputs
            .iter()
            .map(|(_, key, source)| PartInput::prepared(key, source.output()))
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
            .expect("cache hits bounded by input count");
        let mut transaction = Transaction::new();
        let mut published = BTreeMap::new();
        let mut sources = Vec::new();
        for (index, (path, source, input)) in inputs.iter().enumerate() {
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
                    *path,
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
                    Polygon::new(&body.geometry().mesh, policy)
                        .map_err(|kind| SceneError::new(store, *path, kind))?,
                );
                stats.polygon_encodings += 1;
                stats.polygon_validations += 1;
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
            let previous = self.published.get(path);
            let output = previous.map_or_else(
                || {
                    let spelling = alloc::format!("{source}/{GENERATED_CHILD}");
                    let output = Path::parse_absolute(&spelling, store.tokens_mut())
                        .expect("source paths are absolute");
                    store.paths_mut().intern(output)
                },
                |previous| previous.output,
            );
            if previous.is_none()
                && stage.has_prim(output)
                && !stage.prim_stack(output).is_some_and(|stack| {
                    stack.iter().any(|(layer, _)| *layer == self.target.layer())
                })
            {
                return Err(SceneError::new(
                    store,
                    output,
                    SceneErrorKind::OutputCollision,
                ));
            }
            let site = self
                .target
                .map_to_spec_path(output, store.paths_mut())
                .ok_or_else(|| SceneError::new(store, output, SceneErrorKind::PublicationTarget))?;
            let local = store
                .layer(self.target.layer())
                .and_then(|layer| layer.prim_at(&site, store.paths()));
            let created_prim = previous.map_or(local.is_none(), |previous| previous.created_prim);
            let points_changed = local.and_then(|spec| store.tokens().lookup("points").and_then(|token| spec.property(token)))
                .and_then(|spec| spec.default.as_ref()).is_none_or(|value| !matches!(value,
                    Value::TypedArray(TypedArray::Vec3f(points)) if Arc::ptr_eq(points, &encoding.mesh.mesh().points)));
            let update = encoding
                .mesh
                .prepare(
                    stage,
                    store,
                    &self.target,
                    output,
                    previous.map_or(&[], |previous| previous.properties.as_slice()),
                )
                .map_err(|kind| SceneError::new(store, output, SceneErrorKind::Mesh(kind)))?;
            if previous.is_none()
                && update.properties.iter().any(|name| {
                    store.tokens().lookup(name).is_some_and(|token| {
                        store
                            .layer(self.target.layer())
                            .and_then(|layer| layer.prim_at(&site, store.paths()))
                            .and_then(|spec| spec.property(token))
                            .is_some()
                    })
                })
            {
                return Err(SceneError::new(
                    store,
                    output,
                    SceneErrorKind::OutputCollision,
                ));
            }
            stats.publication_properties += update.work.authored_properties;
            if !update.transaction.is_empty() {
                stats.meshes_written += 1;
                if points_changed {
                    stats.points_written += encoding.points;
                }
            }
            if prepare_extent(
                store,
                &self.target,
                *path,
                &encoding.extent,
                previous.is_some(),
                &mut transaction,
            )
            .map_err(|kind| SceneError::new(store, *path, kind))?
            {
                stats.source_bounds_written += 1;
            }
            stats.publication_properties += 1;
            transaction.append(update.transaction);
            published.insert(
                *path,
                Published {
                    output,
                    properties: update.properties,
                    created_prim,
                },
            );
            sources.push(SourceGeometry {
                path: *path,
                source: source.clone(),
                output,
                geometry: geometry.part(part).expect("retained scene input"),
                intent: input.shared_output(),
            });
        }
        for (path, previous) in &self.published {
            if !published.contains_key(path) {
                retire(store, &self.target, previous, &mut transaction)
                    .map_err(|kind| SceneError::new(store, previous.output, kind))?;
                let extent = store.tokens_mut().intern("extent");
                transaction.remove_spec(self.target.property(PropertyPath::new(*path, extent)));
                stats.meshes_removed += 1;
            }
        }
        for (path, _, input) in &inputs {
            input
                .evidence()
                .verify(&Scene::new(live.stage(), store))
                .map_err(|error| {
                    SceneError::new(
                        store,
                        *path,
                        SceneErrorKind::StaleInputs(alloc::format!("{error}")),
                    )
                })?;
        }
        let changes = if transaction.is_empty() {
            layerstack::Changes::default()
        } else {
            live.apply(store, &transaction)
                .map_err(|error| SceneError::new(store, self.root, SceneErrorKind::Publish(error)))?
                .changes
        };

        // Creating output specs can replace ancestor composition records. Refresh
        // retained input evidence against the committed snapshot so the next
        // unchanged sync need not resolve those inputs again. This is cache
        // maintenance: failure retires the binding, not the committed output.
        if !transaction.is_empty() {
            let scene = Scene::new(live.stage(), store);
            for binding in self.bindings.values_mut() {
                let before = binding.lowering.work();
                if binding.lowering.evaluate(&scene, time).is_err() {
                    binding.lowering.invalidate();
                }
                let after = binding.lowering.work();
                stats.record_inputs(before, after);
            }
        }
        self.published = published;
        self.cursor = Some(live.change_cursor());
        Ok(SceneSnapshot {
            sources,
            stats,
            changes,
        })
    }
}

// Remove the declaration only when this producer created it and no partner
// opinions remain. Otherwise retire owned properties at the exact authored site.
fn retire(
    store: &mut dyn LayerStore,
    target: &EditTarget,
    previous: &Published,
    transaction: &mut Transaction,
) -> Result<(), SceneErrorKind> {
    let site = target
        .map_to_spec_path(previous.output, store.paths_mut())
        .ok_or(SceneErrorKind::PublicationTarget)?;
    let layer = store
        .layer(target.layer())
        .ok_or(SceneErrorKind::PublicationTarget)?;
    let generation = layer.generation();
    if let Some(local) = layer.prim_at(&site, store.paths()) {
        let mut remaining = local.clone();
        remaining.properties.retain(|property| {
            !previous
                .properties
                .iter()
                .any(|name| name == store.tokens().resolve(property.name))
        });
        let mut empty = PrimSpec::def();
        empty.type_name = store.tokens().lookup("Mesh");
        empty
            .outer_variant_sites
            .clone_from(&remaining.outer_variant_sites);
        if previous.created_prim && remaining == empty {
            transaction.remove_spec(target.prim(previous.output));
        } else {
            for name in &previous.properties {
                if let Some(token) = store.tokens().lookup(name) {
                    transaction
                        .remove_spec(target.property(PropertyPath::new(previous.output, token)));
                }
            }
        }
        transaction.expect_generation(target.layer(), generation);
    }
    Ok(())
}

// The native procedural is Boundable: its extent describes the generated local
// geometry and is published alongside the Mesh. Compare only two cached points.
fn prepare_extent(
    store: &mut dyn LayerStore,
    target: &EditTarget,
    path: PathId,
    extent: &Arc<Vec<[f32; 3]>>,
    owned: bool,
    transaction: &mut Transaction,
) -> Result<bool, SceneErrorKind> {
    let name = store.tokens_mut().intern("extent");
    let property = PropertyPath::new(path, name);
    let site = target
        .map_property_to_spec_path(property, store.paths_mut())
        .ok_or(SceneErrorKind::PublicationTarget)?;
    let layer = store
        .layer(target.layer())
        .ok_or(SceneErrorKind::PublicationTarget)?;
    let generation = layer.generation();
    let local = layer.property_at(&site, store.paths());
    if !owned && local.is_some() {
        return Err(SceneErrorKind::OutputCollision);
    }
    let value = Value::TypedArray(TypedArray::Vec3f(Arc::clone(extent)));
    let mut changed = false;
    if let Some(local) = local {
        if local.kind != PropertyKind::Attribute
            || local
                .type_name
                .as_ref()
                .is_none_or(|ty| ty.type_name.as_ref() != "float3" || !ty.is_array)
        {
            return Err(SceneErrorKind::Mesh(
                layerstack_schemas::MeshPublicationError::PropertyConflict("extent".into()),
            ));
        }
        if local
            .default
            .as_ref()
            .is_none_or(|current| !current.same_representation(&value))
        {
            transaction.set_default(target.property(property), value);
            changed = true;
        }
        if let Some(samples) = &local.time_samples {
            for (time, _) in samples.as_slice() {
                transaction.remove_time_sample(Address::spec(target.layer(), site.clone()), *time);
                changed = true;
            }
        }
    } else {
        transaction.create_property(
            target.property(property),
            PropertySpec::typed_attribute(PropertyType::new(
                "float3",
                true,
                Value::Vec3f([0.0; 3]),
            ))
            .with_default(value),
        );
        changed = true;
    }
    transaction.expect_generation(target.layer(), generation);
    Ok(changed)
}
