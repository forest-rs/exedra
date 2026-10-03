// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::{sync::Arc, vec, vec::Vec};
use composition::*;
use exedra_constructive::{evaluate::evaluate, ir::PrimitiveSpec, tessellate::EvalPolicy};
use schemas::{XformCache, usd_geom, value};

const GENERATED: LayerId = LayerId(1);
const BASE: LayerId = LayerId(2);
const LOCAL: LayerId = LayerId(3);

struct Fixture {
    store: InMemoryStore,
    live: LiveStage,
    root: PathId,
}

impl Fixture {
    fn new() -> Self {
        let mut store = InMemoryStore::default();
        let mut registry = SchemaRegistry::builder();
        schemas::register(
            &mut registry,
            &[schemas::Domain::UsdGeom, schemas::Domain::UsdShade],
            &mut store.tokens,
        )
        .unwrap();
        register_schemas(&mut registry, &mut store.tokens);
        let registry = registry.build(&mut store.tokens);
        assert!(
            registry.issues().is_empty(),
            "schemas must have resolved inheritance"
        );
        store.insert_layer(Layer::new(GENERATED));
        store.insert_layer(Layer::new(BASE));
        let root = store.path("/World");
        let mut local = Layer::new(LOCAL);
        local.sublayers = vec![SublayerEntry::new(BASE), SublayerEntry::new(GENERATED)];
        local.insert_prim(
            root,
            PrimSpec::def().with_type_name(store.tokens.intern("Xform")),
        );
        store.insert_layer(local);
        let live = LiveStage::compose(
            &mut store,
            LOCAL,
            StageOptions {
                schemas: Some(Arc::new(registry)),
                ..StageOptions::default()
            },
        );
        Self { store, live, root }
    }

    fn primitive(&mut self, name: &str, spec: PrimitiveSpec) -> PathId {
        let path = self.store.path(name);
        let transaction =
            define_primitive(&mut self.store, &EditTarget::for_layer(BASE), path, spec).unwrap();
        self.live.apply(&mut self.store, &transaction).unwrap();
        path
    }

    fn box_at(&mut self, name: &str, size: [f64; 3]) -> PathId {
        self.primitive(name, PrimitiveSpec::Box { size })
    }

    fn set(&mut self, path: PathId, name: &str, value: Value) -> Applied {
        let property = PropertyPath::new(path, self.store.tokens.intern(name));
        let mut transaction = Transaction::new();
        transaction.set_default(EditTarget::for_layer(LOCAL).property(property), value);
        self.live.apply(&mut self.store, &transaction).unwrap()
    }

    fn sync(&mut self, evaluator: &mut SceneEvaluator) -> SceneSnapshot {
        evaluator
            .sync(
                &mut self.live,
                &mut self.store,
                Time::Default,
                &EvalPolicy::default(),
            )
            .unwrap()
    }

    fn generated_points(&self, output: PathId) -> Vec<[f32; 3]> {
        let scene = Scene::new(self.live.stage(), &self.store);
        usd_geom::Mesh::new(&scene, output)
            .unwrap()
            .points()
            .unwrap()
    }
}

#[test]
fn published_polygons_match_direct_construction_and_keep_provenance() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [2.0, 3.0, 4.0]);
    fixture.primitive(
        "/World/Cylinder",
        PrimitiveSpec::Cylinder {
            radius: 2.0,
            height: 5.0,
            segments: 12,
        },
    );
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let snapshot = fixture.sync(&mut evaluator);
    assert_eq!(snapshot.sources.len(), 2);
    assert_eq!(snapshot.stats.geometry_evaluations, 2);
    for source in &snapshot.sources {
        let direct = evaluate(source.recipe(), &EvalPolicy::default()).unwrap();
        let body = &source.evaluation().bodies[0].body;
        let points: Vec<_> = body
            .mesh
            .vertices()
            .map(|v| *body.mesh.vertex_position(v).unwrap())
            .collect();
        assert_eq!(fixture.generated_points(source.output), points);
        assert_eq!(source.evaluation().report, direct.report);
        assert_eq!(
            body.source_map.stats(),
            direct.bodies[0].body.source_map.stats()
        );
        let scene = Scene::new(fixture.live.stage(), &fixture.store);
        let mesh = usd_geom::Mesh::new(&scene, source.output).unwrap();
        assert_eq!(
            mesh.subdivision_scheme(),
            Some(usd_geom::MeshSubdivisionScheme::None)
        );
        assert_eq!(
            mesh.face_vertex_counts().unwrap().len(),
            body.mesh.faces().count()
        );
        let indices = mesh.face_vertex_indices().unwrap();
        let published_corners: Vec<_> = indices
            .iter()
            .map(|&index| points[usize::try_from(index).unwrap()])
            .collect();
        let source_corners: Vec<_> = body
            .mesh
            .faces()
            .flat_map(|face| body.mesh.face_loop(face))
            .map(|edge| {
                *body
                    .mesh
                    .vertex_position(body.mesh.to_vertex(edge).unwrap())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            published_corners, source_corners,
            "USD corners must preserve source polygon winding and attribute correspondence"
        );
        assert_eq!(mesh.normals().unwrap().len(), indices.len());
        let normals = mesh.property_path("normals").unwrap();
        let interpolation = fixture.store.tokens.lookup("interpolation").unwrap();
        let resolved = fixture
            .live
            .stage()
            .resolve_property_metadata(source.output, normals.property(), interpolation)
            .unwrap();
        let ResolvedValue::Scalar(metadata) = resolved.value else {
            panic!("scalar metadata")
        };
        assert_eq!(
            value::read_token(&metadata, &fixture.store.tokens),
            Some("faceVarying")
        );
    }
    assert!(
        evaluator.cached_payload_bytes() > 0,
        "encoded memory must be measurable"
    );
}

#[test]
fn identical_sources_share_geometry_and_unchanged_sync_does_not_write() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Left", [1.0; 3]);
    fixture.box_at("/World/Right", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    assert!(
        first.sources[0].shares_geometry(&first.sources[1]),
        "same content must share topology and reports"
    );
    assert_eq!(first.stats.geometry_evaluations, 1);
    assert_eq!(first.stats.geometry_reused, 1);
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    let unchanged = fixture.sync(&mut evaluator);
    assert_eq!(unchanged.stats.geometry_evaluations, 0);
    assert_eq!(unchanged.stats.geometry_reused, 2);
    assert_eq!(unchanged.stats.meshes_written, 0);
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
}

#[test]
fn dimension_override_regenerates_one_source_and_undo_reuses_the_original() {
    let mut fixture = Fixture::new();
    let left = fixture.box_at("/World/Left", [1.0; 3]);
    fixture.box_at("/World/Right", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let edit = fixture.set(left, SIZE, Value::Vec3d([3.0, 1.0, 1.0]));
    let changed = fixture.sync(&mut evaluator);
    assert_eq!(changed.stats.geometry_evaluations, 1);
    assert_eq!(changed.stats.geometry_reused, 1);
    assert_eq!(changed.stats.meshes_written, 1);
    assert!(
        first.sources[1].shares_geometry(&changed.sources[1]),
        "unaffected source must retain evaluated topology"
    );
    assert!(
        fixture
            .generated_points(changed.sources[0].output)
            .iter()
            .any(|p| p[0] == 3.0),
        "composed dimension must reach output"
    );
    fixture
        .live
        .apply(&mut fixture.store, &edit.inverse)
        .unwrap();
    let undone = fixture.sync(&mut evaluator);
    assert_eq!(undone.stats.geometry_evaluations, 0);
    assert_eq!(undone.stats.meshes_written, 1);
    assert!(
        first.sources[0].shares_geometry(&undone.sources[0]),
        "undo must recover original geometry"
    );
}

#[test]
fn native_transform_and_material_edits_leave_local_geometry_and_output_untouched() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let output = first.sources[0].output;
    let mut transform = PrimSpec::over();
    let translate = fixture.store.tokens.intern("xformOp:translate");
    let order = fixture.store.tokens.intern("xformOpOrder");
    transform.set_property(
        translate,
        PropertySpec::typed_attribute(PropertyType::new("double3", false, Value::Vec3d([0.0; 3])))
            .with_default(Value::Vec3d([5.0, 2.0, 1.0])),
    );
    transform.set_property(
        order,
        PropertySpec::typed_attribute(PropertyType::new("token", true, Value::Token(translate)))
            .uniform()
            .with_default(Value::array(vec![Value::Token(translate)])),
    );
    // Preserve the base primitive; this local opinion only changes placement.
    fixture
        .store
        .layer_mut(LOCAL)
        .unwrap()
        .insert_prim(source, transform);
    fixture.live.notify_layer_prim_edits(LOCAL, &[source]);
    let material = fixture.store.path("/Materials/Timber");
    let mut bind = Transaction::new();
    bind.create_prim(
        EditTarget::for_layer(LOCAL).prim(material),
        Specifier::Def,
        Some(fixture.store.tokens.intern("Material")),
    );
    let binding = PropertyPath::new(output, fixture.store.tokens.intern("material:binding"));
    bind.add_applied_schema(
        EditTarget::for_layer(LOCAL).prim(output),
        fixture.store.tokens.intern("MaterialBindingAPI"),
    );
    bind.set_targets(
        EditTarget::for_layer(LOCAL).property(binding),
        ListOp::explicit(vec![TargetPath::Prim(material)]),
    );
    fixture.live.apply(&mut fixture.store, &bind).unwrap();
    let second = fixture.sync(&mut evaluator);
    assert!(
        first.sources[0].shares_geometry(&second.sources[0]),
        "scene-only edits must share geometry"
    );
    assert_eq!(second.stats.geometry_evaluations, 0);
    assert_eq!(second.stats.meshes_written, 0);
    let scene = Scene::new(fixture.live.stage(), &fixture.store);
    let mut cache = XformCache::new(Time::Default);
    assert_eq!(
        cache.local_to_world(&scene, output).unwrap()[3][..3],
        [5.0, 2.0, 1.0]
    );
    assert_eq!(
        fixture
            .live
            .stage()
            .resolve_target_list_path(binding)
            .unwrap()
            .value,
        vec![TargetPath::Prim(material)]
    );
    fixture.set(source, SIZE, Value::Vec3d([2.0; 3]));
    fixture.sync(&mut evaluator);
    let scene = Scene::new(fixture.live.stage(), &fixture.store);
    let mut bindings = schemas::BindingCache::new(
        schemas::MaterialPurpose::All,
        schemas::BindingOptions {
            support_legacy_bindings: false,
        },
    );
    assert_eq!(
        bindings.compute_bound_material(&scene, output).material,
        Some(material),
        "standard material resolution must survive geometry replacement"
    );
    assert_eq!(
        fixture
            .live
            .stage()
            .resolve_target_list_path(binding)
            .unwrap()
            .value,
        vec![TargetPath::Prim(material)]
    );
}

#[test]
fn generated_layer_cannot_also_occur_at_a_stronger_position() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    fixture
        .store
        .layer_mut(LOCAL)
        .unwrap()
        .sublayers
        .insert(0, SublayerEntry::new(GENERATED));
    fixture.live.notify_structural_change();
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    assert!(matches!(
        evaluator
            .sync(
                &mut fixture.live,
                &mut fixture.store,
                Time::Default,
                &EvalPolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::GeneratedLayer
    ));
    assert!(
        fixture.store.layer(GENERATED).unwrap().prims.is_empty(),
        "ambiguous layer strength must refuse before publication"
    );
}

#[test]
fn references_lower_composed_overrides_and_can_reuse_existing_generated_output() {
    let mut fixture = Fixture::new();
    let prototype = fixture.box_at("/World/Prototype", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let copy = fixture.store.path("/World/Copy");
    fixture.store.layer_mut(BASE).unwrap().insert_prim(
        copy,
        PrimSpec::def().with_reference(Reference::new(LOCAL, prototype)),
    );
    fixture.live.notify_structural_change();
    fixture.live.synchronize(&mut fixture.store);
    let inherited_output = fixture.store.path("/World/Copy/__exedra_geometry");
    assert!(
        fixture.live.stage().has_prim(inherited_output),
        "native reference must include prototype output"
    );
    let copied = fixture.sync(&mut evaluator);
    assert_eq!(copied.sources.len(), 2);
    assert_eq!(copied.stats.geometry_evaluations, 0);
    assert_eq!(copied.stats.meshes_written, 1);
    assert!(
        copied
            .sources
            .iter()
            .all(|source| source.shares_geometry(&first.sources[0])),
        "reference occurrences must share identical construction"
    );
    fixture.set(copy, SIZE, Value::Vec3d([4.0; 3]));
    let specialized = fixture.sync(&mut evaluator);
    assert_eq!(specialized.stats.geometry_evaluations, 1);
    assert_eq!(specialized.stats.meshes_written, 1);
}

#[test]
fn failed_publication_cannot_reuse_store_local_tokens_on_another_stage() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let output = fixture.store.path("/World/Box/__exedra_geometry");
    fixture
        .store
        .layer_mut(BASE)
        .unwrap()
        .insert_prim(output, PrimSpec::def());
    fixture.live.notify_structural_change();
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    assert!(matches!(
        evaluator
            .sync(
                &mut fixture.live,
                &mut fixture.store,
                Time::Default,
                &EvalPolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::OutputCollision
    ));
    assert_eq!(
        evaluator.cached_entries(),
        1,
        "failed sync may retain useful encoded geometry"
    );
    let mut other = Fixture::new();
    other.box_at("/World/Box", [1.0; 3]);
    assert!(matches!(
        evaluator
            .sync(
                &mut other.live,
                &mut other.store,
                Time::Default,
                &EvalPolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::Stage(ChangeHistoryError::DifferentStage)
    ));
}

#[test]
fn removed_sources_remove_only_generated_mesh_specs_and_undo_recreates_them() {
    let mut fixture = Fixture::new();
    let path = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let mut remove = Transaction::new();
    remove.remove_spec(EditTarget::for_layer(BASE).prim(path));
    let removed = fixture.live.apply(&mut fixture.store, &remove).unwrap();
    let empty = fixture.sync(&mut evaluator);
    assert!(
        empty.sources.is_empty(),
        "generated ancestors cannot retain source intent"
    );
    assert_eq!(empty.stats.meshes_removed, 1);
    assert!(
        !fixture.live.stage().has_prim(first.sources[0].output),
        "obsolete output must disappear"
    );
    fixture
        .live
        .apply(&mut fixture.store, &removed.inverse)
        .unwrap();
    let restored = fixture.sync(&mut evaluator);
    assert!(
        restored.sources[0].shares_geometry(&first.sources[0]),
        "restored source should reuse retained content"
    );
}

#[test]
fn invalid_input_leaves_the_last_generated_snapshot_intact() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let points = fixture.generated_points(first.sources[0].output);
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    fixture.set(source, SIZE, Value::Vec3d([-1.0, 2.0, 3.0]));
    let error = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            Time::Default,
            &EvalPolicy::default(),
        )
        .unwrap_err();
    assert_eq!(error.path, "/World/Box");
    assert!(
        matches!(error.kind, SceneErrorKind::Recipe(_)),
        "invalid intent must use recipe validation"
    );
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
    assert_eq!(fixture.generated_points(first.sources[0].output), points);
    fixture.set(source, SIZE, Value::Blocked);
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    Time::Default,
                    &EvalPolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::Parameter(SIZE)
        ),
        "blocked dimensions must refuse instead of inventing defaults"
    );
}

#[test]
fn time_and_policy_participate_in_geometry_identity() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    let size = PropertyPath::new(source, fixture.store.tokens.intern(SIZE));
    let target = EditTarget::for_layer(BASE);
    let mut animate = Transaction::new();
    animate.set_time_sample(target.property(size), 0.0, Value::Vec3d([1.0; 3]));
    animate.set_time_sample(target.property(size), 2.0, Value::Vec3d([3.0; 3]));
    fixture.live.apply(&mut fixture.store, &animate).unwrap();
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    let first = fixture.sync(&mut evaluator);
    let sampled = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            Time::at(1.0),
            &EvalPolicy::default(),
        )
        .unwrap();
    assert_eq!(sampled.stats.geometry_evaluations, 1);
    assert!(
        fixture
            .generated_points(sampled.sources[0].output)
            .iter()
            .any(|p| p[0] == 2.0),
        "sampled construction must use native interpolation"
    );
    let restored = fixture.sync(&mut evaluator);
    assert!(
        first.sources[0].shares_geometry(&restored.sources[0]),
        "default-time construction is separate from numeric sampling"
    );
    let mut policy = EvalPolicy::default();
    policy.sharp_sin_threshold = 0.5;
    let changed = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            Time::Default,
            &policy,
        )
        .unwrap();
    assert_eq!(changed.stats.geometry_evaluations, 1);
    assert!(
        !first.sources[0].shares_geometry(&changed.sources[0]),
        "evaluation policy must affect cache identity"
    );
}

#[test]
fn cache_eviction_and_release_preserve_held_snapshots() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Left", [1.0; 3]);
    fixture.box_at("/World/Right", [2.0; 3]);
    let mut evaluator = SceneEvaluator::with_capacity(fixture.root, GENERATED, 1);
    let first = fixture.sync(&mut evaluator);
    assert_eq!(evaluator.cached_entries(), 1);
    evaluator.clear_cache();
    assert_eq!(evaluator.cached_entries(), 0);
    assert_eq!(evaluator.cached_payload_bytes(), 0);
    assert_eq!(
        first.sources[0].evaluation().bodies[0]
            .body
            .mesh
            .faces()
            .count(),
        6
    );
    let second = fixture.sync(&mut evaluator);
    assert_eq!(second.stats.geometry_evaluations, 2);
    assert_eq!(second.stats.meshes_written, 0);
}

#[test]
fn generated_layer_ownership_and_foreign_stage_are_checked() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    fixture.sync(&mut evaluator);
    let mut other = Fixture::new();
    let error = evaluator
        .sync(
            &mut other.live,
            &mut other.store,
            Time::Default,
            &EvalPolicy::default(),
        )
        .unwrap_err();
    assert!(
        matches!(
            error.kind,
            SceneErrorKind::Stage(ChangeHistoryError::DifferentStage)
        ),
        "store-local identities must never cross stages"
    );
    fixture.store.layer_mut(GENERATED).unwrap().touch();
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    Time::Default,
                    &EvalPolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::GeneratedLayerEdited
        ),
        "external output edits violate explicit ownership"
    );
}

#[test]
fn output_collisions_and_nonfinite_output_are_atomic_refusals() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let output = fixture.store.path("/World/Box/__exedra_geometry");
    fixture
        .store
        .layer_mut(BASE)
        .unwrap()
        .insert_prim(output, PrimSpec::def());
    fixture.live.notify_structural_change();
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    Time::Default,
                    &EvalPolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::OutputCollision
        ),
        "authoring must never take over unrelated content"
    );
    assert!(
        fixture.store.layer(GENERATED).unwrap().prims.is_empty(),
        "collision must leave generated layer empty"
    );
    let mut finite = Fixture::new();
    finite.box_at("/World/Overflow", [f64::MAX; 3]);
    let mut evaluator = SceneEvaluator::new(finite.root, GENERATED);
    assert!(
        evaluator
            .sync(
                &mut finite.live,
                &mut finite.store,
                Time::Default,
                &EvalPolicy::default()
            )
            .is_err(),
        "finite source dimensions cannot authorize nonfinite mesh data"
    );
    assert!(
        finite.store.layer(GENERATED).unwrap().prims.is_empty(),
        "overflow must leave generated layer empty"
    );
}

#[test]
fn invalid_time_and_native_instances_refuse_without_generated_output() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(fixture.root, GENERATED);
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    Time::at(f64::NAN),
                    &EvalPolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::InvalidTime
        ),
        "numeric times must be finite"
    );
    let copy = fixture.store.path("/World/Copy");
    fixture.store.layer_mut(BASE).unwrap().insert_prim(
        copy,
        PrimSpec::def()
            .with_reference(Reference::new(BASE, source))
            .with_instanceable(true),
    );
    fixture.live.notify_structural_change();
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    Time::Default,
                    &EvalPolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::NativeInstance
        ),
        "unsupported instance descendant authoring must be explicit"
    );
    assert!(
        fixture.store.layer(GENERATED).unwrap().prims.is_empty(),
        "failure must be atomic across all sources"
    );
}
