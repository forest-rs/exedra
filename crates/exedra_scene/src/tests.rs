// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::{sync::Arc, vec, vec::Vec};
use composition::*;
use exedra_compile::{CompilePolicy, PartCompiler};
use exedra_constructive::{evaluate::evaluate, ir::PrimitiveSpec};
use schemas::{XformCache, usd_geom, value};

const GENERATED: LayerId = LayerId(1);
const BASE: LayerId = LayerId(2);
const LOCAL: LayerId = LayerId(3);
const CUSTOM_SCHEMA: &str = "ExamplePanel";
const CUSTOM_SYSTEM: &str = "example:panel";

struct Fixture {
    store: InMemoryStore,
    live: LiveStage,
    root: PathId,
    compiler: PartCompiler,
}

impl Fixture {
    fn new() -> Self {
        let mut store = InMemoryStore::default();
        let mut registry = SchemaRegistry::builder();
        schemas::register(
            &mut registry,
            &[
                schemas::Domain::UsdGeom,
                schemas::Domain::UsdProc,
                schemas::Domain::UsdShade,
            ],
            store.tokens(),
        )
        .unwrap();
        register_schemas(&mut registry, store.tokens());
        let system = store.tokens().intern(CUSTOM_SYSTEM);
        registry.register(
            SchemaDefinition::typed(store.tokens().intern(CUSTOM_SCHEMA))
                .with_parent(store.tokens().intern("GenerativeProcedural"))
                .with_override(
                    PropertyDefinition::attribute(store.tokens().intern("proceduralSystem"))
                        .with_type(PropertyType::parse("token").unwrap())
                        .with_fallback(Value::Token(system)),
                )
                .with_property(
                    PropertyDefinition::attribute(store.tokens().intern(SIZE))
                        .with_type(PropertyType::parse("double3").unwrap()),
                ),
        );
        let registry = registry.build(store.tokens());
        assert!(
            registry.issues().is_empty(),
            "schemas must have resolved inheritance: {:?}",
            registry.issues()
        );
        store.insert_layer(Layer::new(GENERATED));
        store.insert_layer(Layer::new(BASE));
        let root = store.path("/World");
        let mut local = Layer::new(LOCAL);
        *local.sublayers_mut() = vec![SublayerEntry::new(BASE), SublayerEntry::new(GENERATED)];
        local.insert_prim(
            root,
            PrimSpec::def().with_type_name(store.tokens().intern("Xform")),
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
        Self {
            store,
            live,
            root,
            compiler: PartCompiler::with_capacity(128),
        }
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
        let property = PropertyPath::new(path, self.store.tokens().intern(name));
        let mut transaction = Transaction::new();
        transaction.set_default(EditTarget::for_layer(LOCAL).property(property), value);
        self.live.apply(&mut self.store, &transaction).unwrap()
    }

    fn sync(&mut self, evaluator: &mut SceneEvaluator) -> SceneSnapshot {
        evaluator
            .sync(
                &mut self.live,
                &mut self.store,
                &mut self.compiler,
                Time::Default,
                &CompilePolicy::default(),
            )
            .unwrap()
    }

    fn generated_points(&self, output: PathId) -> Vec<[f32; 3]> {
        let scene = Scene::new(self.live.stage(), &self.store);
        usd_geom::Mesh::get(&scene, output)
            .unwrap()
            .points()
            .unwrap()
            .as_ref()
            .clone()
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let snapshot = fixture.sync(&mut evaluator);
    assert_eq!(snapshot.sources.len(), 2);
    assert_eq!(snapshot.stats.geometry_evaluations, 2);
    for source in &snapshot.sources {
        let direct = evaluate(
            source.recipe().unwrap(),
            &CompilePolicy::default().evaluation,
        )
        .unwrap();
        let retained = source.body(0).unwrap();
        let body = retained.geometry();
        let points: Vec<_> = body
            .mesh
            .vertices()
            .map(|v| *body.mesh.vertex_position(v).unwrap())
            .collect();
        assert_eq!(fixture.generated_points(source.output), points);
        assert_eq!(source.report(), Some(&direct.report));
        assert_eq!(
            body.source_map.stats(),
            direct.bodies[0].body.source_map.stats()
        );
        let scene = Scene::new(fixture.live.stage(), &fixture.store);
        let mesh = usd_geom::Mesh::get(&scene, source.output).unwrap();
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
        assert_eq!(
            mesh.find_primvar_with_inheritance("normals")
                .unwrap()
                .value(Time::Default)
                .unwrap()
                .unwrap()
                .array_ref()
                .unwrap()
                .len(),
            indices.len()
        );
        let normals = mesh.property_path("primvars:normals").unwrap();
        let interpolation = fixture.store.tokens().lookup("interpolation").unwrap();
        let resolved = fixture
            .live
            .stage()
            .view(&fixture.store)
            .property(normals)
            .unwrap()
            .metadata(interpolation)
            .unwrap();
        let ResolvedValue::Scalar(metadata) = resolved.value else {
            panic!("scalar metadata")
        };
        assert_eq!(
            value::read_token(&metadata, fixture.store.tokens()),
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
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
    assert_eq!(unchanged.stats.discovered_prims, 0);
    assert_eq!(unchanged.stats.source_evaluations, 0);
    assert_eq!(unchanged.stats.input_resolutions, 0);
    assert_eq!(unchanged.stats.polygon_validations, 0);
    assert!(unchanged.stats.input_reused > 0);
    assert!(
        core::ptr::eq(
            first.sources[0].recipe().unwrap(),
            unchanged.sources[0].recipe().unwrap()
        ),
        "snapshots retain the shared lowered intent without copying recipes"
    );
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
}

#[test]
fn standalone_compilation_and_native_stages_share_the_same_geometry_service() {
    use exedra_compile::{PartId, PartInput, PartSource, StaleSelection};
    use exedra_constructive::workplane::SurfaceInventoryPolicy;
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let recipe =
        PartSource::Recipe(primitive::recipe(PrimitiveSpec::Box { size: [1.0; 3] }).unwrap());
    let standalone = fixture
        .compiler
        .compile_snapshot(
            &[PartInput::new("standalone", &recipe)],
            &CompilePolicy::default(),
        )
        .unwrap();
    let body = standalone.body_by_source(PartId(0), "primitive").unwrap();
    let evidence = body
        .inspect_surfaces(&SurfaceInventoryPolicy::default())
        .unwrap();
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let native = fixture.sync(&mut evaluator);
    assert_eq!(native.stats.geometry_evaluations, 0);
    assert_eq!(native.stats.polygon_encodings, 1);
    assert!(
        Arc::ptr_eq(
            standalone.compiled().part(PartId(0)).unwrap(),
            native.sources[0].compiled()
        ),
        "stage publication must reuse standalone extraction buffers"
    );
    let native_body = native.sources[0].body_by_source("primitive").unwrap();
    assert!(
        core::ptr::eq(body.geometry(), native_body.geometry()),
        "stage and standalone queries must share evaluated topology"
    );
    assert_eq!(evidence.check(&native_body), Err(StaleSelection));
    evaluator.clear_cache();
    let encoded_again = fixture.sync(&mut evaluator);
    assert_eq!(encoded_again.stats.geometry_evaluations, 0);
    assert_eq!(encoded_again.stats.polygon_encodings, 1);
    assert_eq!(encoded_again.stats.meshes_written, 1);

    let mut other = Fixture::new();
    other.box_at("/World/Box", [1.0; 3]);
    let mut other_evaluator =
        SceneEvaluator::new(&other.store, other.root, EditTarget::for_layer(GENERATED));
    let other_native = other_evaluator
        .sync(
            &mut other.live,
            &mut other.store,
            &mut fixture.compiler,
            Time::Default,
            &CompilePolicy::default(),
        )
        .unwrap();
    assert_eq!(other_native.stats.geometry_evaluations, 0);
    assert!(
        native.sources[0].shares_geometry(&other_native.sources[0]),
        "shared geometry must be independent of native stores and token handles"
    );
    fixture.compiler.clear_cache();
    assert_eq!(evidence.check(&body), Ok(()));
    assert_eq!(native.sources[0].compiled().triangle_count(), 12);
}

#[test]
fn dimension_override_regenerates_one_source_and_undo_reuses_the_original() {
    let mut fixture = Fixture::new();
    let left = fixture.box_at("/World/Left", [1.0; 3]);
    fixture.box_at("/World/Right", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let output = first.sources[0].output;
    let mut transform = PrimSpec::over();
    let translate = fixture.store.tokens().intern("xformOp:translate");
    let order = fixture.store.tokens().intern("xformOpOrder");
    transform.set_property(
        translate,
        PropertySpec::typed_attribute(
            PropertyType::parse("double3").expect("registered USD value type"),
        )
        .with_default(Value::Vec3d([5.0, 2.0, 1.0])),
    );
    transform.set_property(
        order,
        PropertySpec::typed_attribute(
            PropertyType::parse("token[]").expect("registered USD value type"),
        )
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
        Some(fixture.store.tokens().intern("Material")),
    );
    let binding = PropertyPath::new(output, fixture.store.tokens().intern("material:binding"));
    bind.add_applied_schema(
        EditTarget::for_layer(LOCAL).prim(output),
        fixture.store.tokens().intern("MaterialBindingAPI"),
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
            .view(&fixture.store)
            .relationship(binding)
            .unwrap()
            .targets(),
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
            .view(&fixture.store)
            .relationship(binding)
            .unwrap()
            .targets(),
        vec![TargetPath::Prim(material)]
    );
}

#[test]
fn publication_target_can_be_the_root_layer_with_partner_content() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator =
        SceneEvaluator::new(&fixture.store, fixture.root, EditTarget::for_layer(LOCAL));
    let snapshot = fixture.sync(&mut evaluator);
    assert_eq!(snapshot.stats.meshes_written, 1);
    assert!(
        fixture
            .store
            .layer(LOCAL)
            .unwrap()
            .prims()
            .get(&snapshot.sources[0].output)
            .is_some()
    );
    assert_eq!(fixture.sync(&mut evaluator).stats.meshes_written, 0);
}

#[test]
fn references_lower_composed_overrides_and_can_reuse_existing_generated_output() {
    let mut fixture = Fixture::new();
    let prototype = fixture.box_at("/World/Prototype", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
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
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .prim(inherited_output)
            .is_some(),
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    assert!(matches!(
        evaluator
            .sync(
                &mut fixture.live,
                &mut fixture.store,
                &mut fixture.compiler,
                Time::Default,
                &CompilePolicy::default()
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
                &mut other.compiler,
                Time::Default,
                &CompilePolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::ForeignStore
    ));
}

#[test]
fn removed_sources_remove_only_generated_mesh_specs_and_undo_recreates_them() {
    let mut fixture = Fixture::new();
    let path = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
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
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .prim(first.sources[0].output)
            .is_none(),
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let points = fixture.generated_points(first.sources[0].output);
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    fixture.set(source, SIZE, Value::Vec3d([-1.0, 2.0, 3.0]));
    let error = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::Default,
            &CompilePolicy::default(),
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
                    &mut fixture.compiler,
                    Time::Default,
                    &CompilePolicy::default()
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
    let size = PropertyPath::new(source, fixture.store.tokens().intern(SIZE));
    let target = EditTarget::for_layer(BASE);
    let mut animate = Transaction::new();
    animate.set_time_sample(target.property(size), 0.0, Value::Vec3d([1.0; 3]));
    animate.set_time_sample(target.property(size), 2.0, Value::Vec3d([3.0; 3]));
    fixture.live.apply(&mut fixture.store, &animate).unwrap();
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let sampled = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::at(1.0),
            &CompilePolicy::default(),
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
    let mut policy = CompilePolicy::default();
    policy.evaluation.sharp_sin_threshold = 0.5;
    let changed = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
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
    fixture.compiler = PartCompiler::with_capacity(1);
    let mut evaluator = SceneEvaluator::with_capacity(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
        1,
    );
    let first = fixture.sync(&mut evaluator);
    assert_eq!(evaluator.cached_entries(), 1);
    evaluator.clear_cache();
    assert_eq!(evaluator.cached_entries(), 0);
    assert_eq!(evaluator.cached_payload_bytes(), 0);
    assert_eq!(
        first.sources[0]
            .body(0)
            .unwrap()
            .geometry()
            .mesh
            .faces()
            .count(),
        6
    );
    let second = fixture.sync(&mut evaluator);
    assert_eq!(second.stats.geometry_evaluations, 2);
    assert_eq!(second.stats.meshes_written, 2);
}

#[test]
fn store_affinity_is_checked_and_output_edits_are_repaired() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let mut other = Fixture::new();
    assert!(matches!(
        evaluator
            .sync(
                &mut other.live,
                &mut other.store,
                &mut other.compiler,
                Time::Default,
                &CompilePolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::ForeignStore
    ));
    let points = fixture
        .store
        .property_path("/World/Box/__exedra_geometry.points");
    let mut edit = Transaction::new();
    edit.clear_default(EditTarget::for_layer(GENERATED).property(points));
    fixture.live.apply(&mut fixture.store, &edit).unwrap();
    let repaired = fixture.sync(&mut evaluator);
    assert_eq!(repaired.stats.source_evaluations, 0);
    assert_eq!(repaired.stats.polygon_validations, 0);
    assert_eq!(repaired.stats.meshes_written, 1);
    assert_eq!(
        fixture.generated_points(first.sources[0].output),
        fixture.generated_points(repaired.sources[0].output)
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
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    &mut fixture.compiler,
                    Time::Default,
                    &CompilePolicy::default()
                )
                .unwrap_err()
                .kind,
            SceneErrorKind::OutputCollision
        ),
        "authoring must never take over unrelated content"
    );
    assert!(
        fixture.store.layer(GENERATED).unwrap().prims().is_empty(),
        "collision must leave generated layer empty"
    );
    let mut finite = Fixture::new();
    finite.box_at("/World/Overflow", [f64::MAX; 3]);
    let mut evaluator =
        SceneEvaluator::new(&finite.store, finite.root, EditTarget::for_layer(GENERATED));
    assert!(
        evaluator
            .sync(
                &mut finite.live,
                &mut finite.store,
                &mut finite.compiler,
                Time::Default,
                &CompilePolicy::default()
            )
            .is_err(),
        "finite source dimensions cannot authorize nonfinite mesh data"
    );
    assert!(
        finite.store.layer(GENERATED).unwrap().prims().is_empty(),
        "overflow must leave generated layer empty"
    );
}

#[test]
fn invalid_time_refuses_and_native_instances_consume_authored_geometry() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    assert!(
        matches!(
            evaluator
                .sync(
                    &mut fixture.live,
                    &mut fixture.store,
                    &mut fixture.compiler,
                    Time::at(f64::NAN),
                    &CompilePolicy::default()
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
    let snapshot = fixture.sync(&mut evaluator);
    assert_eq!(
        snapshot.sources.len(),
        1,
        "instance placements are not independent recipe sources"
    );
    let output = fixture.store.path("/World/Copy/__exedra_geometry");
    assert_eq!(
        fixture.generated_points(snapshot.sources[0].output),
        fixture.generated_points(output)
    );
    assert!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .prims()
            .get(&output)
            .is_none(),
        "never author beneath an instance proxy"
    );
}

#[test]
fn shared_stage_partners_keep_their_layers_and_strength_order() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [2.0; 3]);
    let partner_id = LayerId(4);
    let partner_path = fixture.store.path("/World/OtherPartner");
    let mut partner = Layer::new(partner_id);
    partner.insert_prim(
        partner_path,
        PrimSpec::def().with_type_name(fixture.store.tokens().intern("Xform")),
    );
    fixture.store.insert_layer(partner);
    *fixture.store.layer_mut(LOCAL).unwrap().sublayers_mut() = vec![
        SublayerEntry::new(GENERATED),
        SublayerEntry::new(partner_id),
        SublayerEntry::new(BASE),
    ];
    let partner_before = fixture.store.layer(partner_id).unwrap().prims().clone();
    let partner_generation = fixture.store.layer(partner_id).unwrap().generation();
    fixture.live.notify_structural_change();
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    assert_eq!(first.sources.len(), 1);
    assert_eq!(first.stats.meshes_written, 1);
    assert!(
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .prim(partner_path)
            .is_some()
    );
    assert_eq!(
        fixture.store.layer(partner_id).unwrap().prims(),
        &partner_before
    );
    assert_eq!(
        fixture.store.layer(partner_id).unwrap().generation(),
        partner_generation
    );
    assert_eq!(
        fixture.store.layer(LOCAL).unwrap().sublayers()[0].layer,
        GENERATED
    );
    let unchanged = fixture.sync(&mut evaluator);
    assert_eq!(unchanged.stats.meshes_written, 0);
}

#[test]
fn two_evaluators_publish_independent_layers_on_one_caller_owned_stage() {
    let mut fixture = Fixture::new();
    let left = fixture.box_at("/World/Left", [1.0; 3]);
    let right = fixture.box_at("/World/Right", [1.0; 3]);
    let other_generated = LayerId(4);
    fixture.store.insert_layer(Layer::new(other_generated));
    *fixture.store.layer_mut(LOCAL).unwrap().sublayers_mut() = vec![
        SublayerEntry::new(GENERATED),
        SublayerEntry::new(other_generated),
        SublayerEntry::new(BASE),
    ];
    fixture.live.notify_structural_change();
    let mut left_evaluator =
        SceneEvaluator::new(&fixture.store, left, EditTarget::for_layer(GENERATED));
    let mut right_evaluator = SceneEvaluator::new(
        &fixture.store,
        right,
        EditTarget::for_layer(other_generated),
    );
    let first = fixture.sync(&mut left_evaluator);
    let left_generation = fixture.store.layer(GENERATED).unwrap().generation();
    let second = fixture.sync(&mut right_evaluator);
    assert_eq!(first.sources.len(), 1);
    assert_eq!(second.sources.len(), 1);
    assert_eq!(second.stats.geometry_evaluations, 0);
    assert!(first.sources[0].shares_geometry(&second.sources[0]));
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        left_generation
    );
    fixture.set(right, SIZE, Value::Vec3d([2.0; 3]));
    assert_eq!(fixture.sync(&mut right_evaluator).stats.meshes_written, 1);
    assert_eq!(fixture.sync(&mut left_evaluator).stats.meshes_written, 0);
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        left_generation
    );
}

#[test]
fn two_producers_share_a_layer_and_preserve_partner_opinions_on_retirement() {
    let mut fixture = Fixture::new();
    let left = fixture.box_at("/World/Left", [1.0; 3]);
    let right = fixture.box_at("/World/Right", [2.0; 3]);
    let target = EditTarget::for_layer(GENERATED);
    let mut left_evaluator = SceneEvaluator::new(&fixture.store, left, target.clone());
    let mut right_evaluator = SceneEvaluator::new(&fixture.store, right, target.clone());
    let left_output = fixture.sync(&mut left_evaluator).sources[0].output;
    let right_output = fixture.sync(&mut right_evaluator).sources[0].output;
    let tag = fixture
        .store
        .property_path("/World/Left/__exedra_geometry.user:tag");
    let child = fixture
        .store
        .path("/World/Left/__exedra_geometry/Annotation");
    let mut partner = Transaction::new();
    partner.create_property(
        target.property(tag),
        PropertySpec::typed_attribute(
            PropertyType::parse("int").expect("registered USD value type"),
        )
        .with_default(Value::Int(17)),
    );
    partner.create_prim(target.prim(child), Specifier::Def, None);
    fixture.live.apply(&mut fixture.store, &partner).unwrap();
    fixture.set(left, SIZE, Value::Vec3d([3.0; 3]));
    assert_eq!(fixture.sync(&mut left_evaluator).stats.meshes_written, 1);
    assert_eq!(fixture.sync(&mut right_evaluator).stats.meshes_written, 0);
    let mut remove = Transaction::new();
    remove.remove_spec(EditTarget::for_layer(BASE).prim(left));
    fixture.live.apply(&mut fixture.store, &remove).unwrap();
    assert_eq!(fixture.sync(&mut left_evaluator).stats.meshes_removed, 1);
    assert!(
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .prim(child)
            .is_some()
    );
    assert_eq!(
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .attribute(tag)
            .unwrap()
            .get(Time::Default)
            .unwrap()
            .value,
        Value::Int(17)
    );
    assert!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .property(PropertyPath::new(
                left_output,
                fixture.store.tokens().lookup("points").unwrap()
            ))
            .is_none()
    );
    assert_eq!(fixture.generated_points(right_output).len(), 8);
}

#[test]
fn masked_inputs_reuse_lowering_and_history_loss_refreshes_only_membership() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [1.0; 3]);
    fixture.set(source, SIZE, Value::Vec3d([2.0; 3]));
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let mut masked = Transaction::new();
    let size = PropertyPath::new(source, fixture.store.tokens().lookup(SIZE).unwrap());
    masked.set_default(
        EditTarget::for_layer(BASE).property(size),
        Value::Vec3d([4.0; 3]),
    );
    fixture.live.apply(&mut fixture.store, &masked).unwrap();
    let second = fixture.sync(&mut evaluator);
    assert_eq!(second.stats.discovered_prims, 0);
    assert_eq!(second.stats.source_evaluations, 0);
    assert_eq!(second.stats.meshes_written, 0);
    fixture.live.set_change_history_budget(ChangeHistoryBudget {
        max_batches: 0,
        max_retained_bytes: 0,
    });
    let mut unrelated = Transaction::new();
    unrelated.set_metadata(
        EditTarget::for_layer(LOCAL).prim(fixture.root),
        fixture.store.tokens().intern("documentation"),
        "partner edit".into(),
    );
    fixture.live.apply(&mut fixture.store, &unrelated).unwrap();
    let recovered = fixture.sync(&mut evaluator);
    assert!(recovered.stats.discovered_prims > 0);
    assert_eq!(recovered.stats.source_evaluations, 0);
    assert_eq!(recovered.stats.polygon_validations, 0);
    assert_eq!(recovered.stats.meshes_written, 0);
    assert!(recovered.sources[0].shares_geometry(&first.sources[0]));
}

#[test]
fn incompatible_publication_is_atomic_across_sources_and_can_be_retried() {
    let mut fixture = Fixture::new();
    let left = fixture.box_at("/World/Left", [1.0; 3]);
    fixture.box_at("/World/Right", [2.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let old_points = fixture.generated_points(first.sources[0].output);
    fixture.set(left, SIZE, Value::Vec3d([4.0; 3]));
    let points = fixture.store.tokens().lookup("points").unwrap();
    let right_output = first.sources[1].output;
    let layer = fixture.store.layer_mut(GENERATED).unwrap();
    layer
        .prims_mut()
        .get_mut(&right_output)
        .unwrap()
        .set_property(
            points,
            PropertySpec::typed_attribute(
                PropertyType::parse("string").expect("registered USD value type"),
            )
            .with_default(Value::string("partner declaration")),
        );
    layer.touch();
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    assert!(matches!(
        evaluator
            .sync(
                &mut fixture.live,
                &mut fixture.store,
                &mut fixture.compiler,
                Time::Default,
                &CompilePolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::Mesh(schemas::MeshPublicationError::PropertyConflict(_))
    ));
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
    assert_eq!(
        fixture.generated_points(first.sources[0].output),
        old_points
    );
    let mut remove = Transaction::new();
    remove.remove_spec(
        EditTarget::for_layer(GENERATED).property(PropertyPath::new(right_output, points)),
    );
    fixture.live.apply(&mut fixture.store, &remove).unwrap();
    let retried = fixture.sync(&mut evaluator);
    assert_eq!(retried.stats.source_evaluations, 0);
    assert_eq!(retried.stats.meshes_written, 2);
    assert!(
        fixture
            .generated_points(retried.sources[0].output)
            .iter()
            .any(|p| p[0] == 4.0)
    );
}

#[test]
fn a_replacement_stage_in_the_same_store_refreshes_retained_queries() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = fixture.sync(&mut evaluator);
    let options = StageOptions {
        schemas: fixture.live.stage().schemas().cloned().map(Arc::new),
        ..StageOptions::default()
    };
    fixture.live = LiveStage::compose(&mut fixture.store, LOCAL, options);
    let replaced = fixture.sync(&mut evaluator);
    assert!(replaced.stats.discovered_prims > 0);
    assert_eq!(replaced.stats.source_evaluations, 0);
    assert_eq!(replaced.stats.meshes_written, 0);
    assert!(replaced.sources[0].shares_geometry(&first.sources[0]));
}

#[test]
fn procedural_sources_and_native_instances_have_native_bounds() {
    let mut fixture = Fixture::new();
    let source = fixture.box_at("/World/Box", [2.0, 3.0, 4.0]);
    let copy = fixture.store.path("/World/Copy");
    fixture.store.layer_mut(BASE).unwrap().insert_prim(
        copy,
        PrimSpec::def()
            .with_reference(Reference::new(BASE, source))
            .with_instanceable(true),
    );
    let instancer = fixture.store.path("/World/Instances");
    let prototypes = fixture.store.tokens().intern("prototypes");
    let positions = fixture.store.tokens().intern("positions");
    let indices = fixture.store.tokens().intern("protoIndices");
    let instancer_type = fixture.store.tokens().intern("PointInstancer");
    fixture.store.layer_mut(BASE).unwrap().insert_prim(
        instancer,
        PrimSpec::def()
            .with_type_name(instancer_type)
            .with_property(
                prototypes,
                PropertySpec::relationship()
                    .with_targets(ListOp::explicit(vec![TargetPath::Prim(source)])),
            )
            .with_property(
                positions,
                PropertySpec::typed_attribute(
                    PropertyType::parse("point3f[]").expect("registered USD value type"),
                )
                .with_default(Value::TypedArray(TypedArray::Vec3f(Arc::new(
                    vec![[10.0, 0.0, 0.0]],
                )))),
            )
            .with_property(
                indices,
                PropertySpec::typed_attribute(
                    PropertyType::parse("int[]").expect("registered USD value type"),
                )
                .with_default(Value::TypedArray(TypedArray::Int(Arc::new(vec![0])))),
            ),
    );
    fixture.live.notify_structural_change();
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let snapshot = fixture.sync(&mut evaluator);
    let scene = Scene::new(fixture.live.stage(), &fixture.store);
    let mut bounds =
        schemas::bounds::BoundsCache::new(Time::Default, schemas::bounds::BoundsOptions::default());
    let expected = bounds
        .world_bound(&scene, snapshot.sources[0].output)
        .unwrap();
    assert_eq!(bounds.world_bound(&scene, source).unwrap(), expected);
    assert_eq!(bounds.world_bound(&scene, copy).unwrap(), expected);
    assert_eq!(
        bounds
            .world_bound(&scene, instancer)
            .unwrap()
            .aligned_range()
            .max,
        [12.0, 3.0, 4.0]
    );
    let edit = fixture.set(source, SIZE, Value::Vec3d([5.0, 6.0, 7.0]));
    let changed = fixture.sync(&mut evaluator);
    let scene = Scene::new(fixture.live.stage(), &fixture.store);
    bounds.apply_changes(&scene, &edit.changes);
    bounds.apply_changes(&scene, &changed.changes);
    for path in [source, copy] {
        assert_eq!(
            bounds
                .world_bound(&scene, path)
                .unwrap()
                .aligned_range()
                .max,
            [5.0, 6.0, 7.0]
        );
    }
    assert_eq!(
        bounds
            .world_bound(&scene, instancer)
            .unwrap()
            .aligned_range()
            .max,
        [15.0, 6.0, 7.0]
    );
}

#[test]
fn mapped_publication_retires_samples_in_authored_layer_time() {
    let mut fixture = Fixture::new();
    let asset = fixture.box_at("/Asset", [2.0, 3.0, 4.0]);
    let source = fixture.store.path("/World/Box");
    let mut reference = Reference::with_asset(BASE, asset, "asset.usda");
    reference.layer_offset = LayerOffset {
        offset: 10.0,
        scale: 2.0,
    };
    fixture
        .store
        .layer_mut(LOCAL)
        .unwrap()
        .insert_prim(source, PrimSpec::def().with_reference(reference));
    fixture.live.notify_structural_change();
    fixture.live.synchronize(&mut fixture.store);
    let graph = fixture
        .live
        .stage()
        .view(&fixture.store)
        .prim(source)
        .unwrap()
        .composition_graph();
    let node = graph
        .nodes()
        .find(|(_, node)| node.arc_kind() == ArcKind::References)
        .unwrap()
        .0;
    let target = EditTarget::for_node(fixture.live.stage(), source, node).unwrap();
    assert_eq!(target.layer(), BASE);
    assert_eq!(target.map_to_spec_time(14.0), 2.0);
    let mut evaluator = SceneEvaluator::new(&fixture.store, source, target.clone());
    let first = fixture.sync(&mut evaluator);
    let output = first.sources[0].output;
    let original_points = fixture.generated_points(output);
    let mut edits = Transaction::new();
    let properties = [
        PropertyPath::new(source, fixture.store.tokens().intern("extent")),
        PropertyPath::new(output, fixture.store.tokens().intern("points")),
    ];
    for property in properties {
        edits.set_time_sample(
            target.property(property),
            14.0,
            Value::TypedArray(TypedArray::Vec3f(Arc::new(vec![[99.0; 3]; 2]))),
        );
    }
    fixture.live.apply(&mut fixture.store, &edits).unwrap();
    for property in properties {
        let site = target
            .map_property_to_spec_path(property, fixture.store.paths())
            .unwrap();
        let spec = fixture
            .store
            .layer(BASE)
            .unwrap()
            .property_at(&site)
            .unwrap();
        assert_eq!(spec.time_samples.get().unwrap()[0].0, 2.0);
    }
    let repaired = fixture.sync(&mut evaluator);
    assert_eq!(repaired.stats.source_evaluations, 0);
    assert_eq!(repaired.stats.polygon_validations, 0);
    assert_eq!(repaired.stats.meshes_written, 1);
    assert_eq!(repaired.stats.source_bounds_written, 1);
    for property in properties {
        let site = target
            .map_property_to_spec_path(property, fixture.store.paths())
            .unwrap();
        let spec = fixture
            .store
            .layer(BASE)
            .unwrap()
            .property_at(&site)
            .unwrap();
        assert!(
            spec.time_samples
                .get()
                .is_none_or(|samples| samples.is_empty())
        );
    }
    assert_eq!(fixture.generated_points(output), original_points);
    assert!(
        !fixture
            .store
            .layer(LOCAL)
            .unwrap()
            .prims()
            .contains_key(&output)
    );
}

#[test]
fn obsolete_uvs_are_retired_without_removing_partner_properties() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let projected = CompilePolicy {
        uvs: exedra_mesh::UvSource::CustomOrBoxProjected { scale: 1.0 },
        ..CompilePolicy::default()
    };
    let first = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::Default,
            &projected,
        )
        .unwrap();
    let output = first.sources[0].output;
    let uv = PropertyPath::new(
        output,
        fixture.store.tokens().lookup("primvars:st").unwrap(),
    );
    assert!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .property(uv)
            .is_some()
    );
    let tag = PropertyPath::new(output, fixture.store.tokens().intern("user:label"));
    let mut partner = Transaction::new();
    partner.create_property(
        EditTarget::for_layer(GENERATED).property(tag),
        PropertySpec::typed_attribute(
            PropertyType::parse("int").expect("registered USD value type"),
        )
        .with_default(Value::Int(7)),
    );
    fixture.live.apply(&mut fixture.store, &partner).unwrap();
    let policy = CompilePolicy {
        uvs: exedra_mesh::UvSource::CustomOnly,
        ..CompilePolicy::default()
    };
    let changed = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::Default,
            &policy,
        )
        .unwrap();
    assert_eq!(changed.stats.source_evaluations, 0);
    assert!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .property(uv)
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .property(tag)
            .unwrap()
            .default,
        Some(Value::Int(7))
    );
}

#[test]
fn initial_publication_refuses_occupied_geometry_in_its_target() {
    let mut fixture = Fixture::new();
    fixture.box_at("/World/Box", [1.0; 3]);
    let output = fixture.store.path("/World/Box/__exedra_geometry");
    let points = fixture.store.tokens().intern("points");
    let mesh = fixture.store.tokens().intern("Mesh");
    let mut partner = Transaction::new();
    partner.create_prim(
        EditTarget::for_layer(GENERATED).prim(output),
        Specifier::Def,
        Some(mesh),
    );
    partner.create_property(
        EditTarget::for_layer(GENERATED).property(PropertyPath::new(output, points)),
        PropertySpec::typed_attribute(
            PropertyType::parse("point3f[]").expect("registered USD value type"),
        )
        .with_default(Value::TypedArray(TypedArray::Vec3f(Arc::new(vec![
            [7.0; 3],
        ])))),
    );
    fixture.live.apply(&mut fixture.store, &partner).unwrap();
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    assert!(matches!(
        evaluator
            .sync(
                &mut fixture.live,
                &mut fixture.store,
                &mut fixture.compiler,
                Time::Default,
                &CompilePolicy::default()
            )
            .unwrap_err()
            .kind,
        SceneErrorKind::OutputCollision
    ));
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
    assert_eq!(fixture.generated_points(output), vec![[7.0; 3]]);
}

#[derive(Clone, Copy, Debug)]
struct CustomGenerator {
    factor: f64,
    fail: bool,
    read_output: Option<PathId>,
}
impl schemas::procedural::ProceduralEvaluator for CustomGenerator {
    type Output = exedra_compile::PreparedSource;
    type Error = SceneErrorKind;
    fn system(&self) -> &str {
        CUSTOM_SYSTEM
    }
    fn evaluate(
        &self,
        inputs: &mut schemas::procedural::ProceduralInputs<'_>,
    ) -> Result<Self::Output, Self::Error> {
        if self.fail {
            return Err(SceneErrorKind::Generator {
                code: "example.refused",
                message: "replacement refused".into(),
            });
        }
        if let Some(output) = self.read_output {
            let _ = inputs
                .attribute(output, "points")
                .map_err(SceneErrorKind::Input)?;
        }
        let size = inputs
            .attribute(inputs.recipe(), SIZE)
            .map_err(SceneErrorKind::Input)?
            .ok_or(SceneErrorKind::Parameter(SIZE))?;
        let size = value::read_double3(&size, inputs.tokens())
            .ok_or(SceneErrorKind::Parameter(SIZE))?
            .map(|v| v * self.factor);
        primitive::recipe(PrimitiveSpec::Box { size })
            .map(|recipe| {
                exedra_compile::PreparedSource::new(exedra_compile::PartSource::Recipe(recipe))
            })
            .map_err(SceneErrorKind::Recipe)
    }
}
fn custom_source(fixture: &mut Fixture) -> PathId {
    let source = fixture.store.path("/World/Panel");
    let target = EditTarget::for_layer(BASE);
    let mut author = Transaction::new();
    author.create_prim(
        target.prim(source),
        Specifier::Def,
        Some(fixture.store.tokens().intern(CUSTOM_SCHEMA)),
    );
    author.create_property(
        target.property(PropertyPath::new(
            source,
            fixture.store.tokens().intern(SIZE),
        )),
        PropertySpec::typed_attribute(PropertyType::parse("double3").unwrap())
            .with_default(Value::Vec3d([2.0; 3])),
    );
    fixture.live.apply(&mut fixture.store, &author).unwrap();
    source
}
#[test]
fn same_revision_generator_replacement_invalidates_inputs_and_keeps_publication_ownership() {
    let mut fixture = Fixture::new();
    let source = custom_source(&mut fixture);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    let first = evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            7,
            CustomGenerator {
                factor: 1.0,
                fail: false,
                read_output: None,
            },
        )
        .unwrap();
    assert!(!first.replaced);
    let initial = fixture.sync(&mut evaluator);
    assert_eq!(initial.sources[0].generator.revision, 7);
    assert_eq!(fixture.sync(&mut evaluator).stats.source_evaluations, 0);
    let note = PropertyPath::new(
        initial.sources[0].output,
        fixture.store.tokens().intern("partner:note"),
    );
    let mut partner = Transaction::new();
    partner.create_property(
        EditTarget::for_layer(GENERATED).property(note),
        PropertySpec::typed_attribute(PropertyType::parse("string").unwrap())
            .with_default(Value::string("keep")),
    );
    fixture.live.apply(&mut fixture.store, &partner).unwrap();
    let replacement = evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            7,
            CustomGenerator {
                factor: 2.0,
                fail: false,
                read_output: None,
            },
        )
        .unwrap();
    assert_eq!(
        replacement,
        GeneratorRegistration {
            replaced: true,
            invalidated_sources: 1
        }
    );
    let changed = fixture.sync(&mut evaluator);
    assert_eq!(changed.stats.source_evaluations, 1);
    assert!(!initial.sources[0].shares_generator_definition(&changed.sources[0]));
    assert!(!initial.sources[0].shares_geometry(&changed.sources[0]));
    assert_eq!(
        fixture
            .generated_points(changed.sources[0].output)
            .iter()
            .map(|p| p[0])
            .fold(0.0_f32, f32::max),
        4.0
    );
    evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            7,
            CustomGenerator {
                factor: 0.0,
                fail: true,
                read_output: None,
            },
        )
        .unwrap();
    let generation = fixture.store.layer(GENERATED).unwrap().generation();
    let points = fixture.generated_points(changed.sources[0].output);
    let failure = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::Default,
            &CompilePolicy::default(),
        )
        .unwrap_err();
    assert_eq!(failure.path, "/World/Panel");
    assert!(matches!(
        failure.kind,
        SceneErrorKind::Generator {
            code: "example.refused",
            ..
        }
    ));
    assert_eq!(
        fixture.store.layer(GENERATED).unwrap().generation(),
        generation
    );
    assert_eq!(fixture.generated_points(changed.sources[0].output), points);
    evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            7,
            CustomGenerator {
                factor: 3.0,
                fail: false,
                read_output: None,
            },
        )
        .unwrap();
    let recovered = fixture.sync(&mut evaluator);
    assert_eq!(recovered.stats.meshes_written, 1);
    assert_eq!(
        fixture
            .generated_points(recovered.sources[0].output)
            .iter()
            .map(|p| p[0])
            .fold(0.0_f32, f32::max),
        6.0
    );
    assert_eq!(
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .attribute(note)
            .unwrap()
            .get(Time::Default)
            .unwrap()
            .value,
        Value::string("keep")
    );
    let mut remove = Transaction::new();
    remove.remove_spec(EditTarget::for_layer(BASE).prim(source));
    fixture.live.apply(&mut fixture.store, &remove).unwrap();
    let retired = fixture.sync(&mut evaluator);
    assert_eq!(retired.stats.meshes_removed, 1);
    assert!(
        fixture
            .live
            .stage()
            .view(&fixture.store)
            .attribute(note)
            .is_some()
    );
    assert!(
        fixture
            .store
            .layer(GENERATED)
            .unwrap()
            .prim(initial.sources[0].output)
            .unwrap()
            .property(fixture.store.tokens().lookup("points").unwrap())
            .is_none()
    );
}
#[test]
fn custom_generators_cannot_consume_published_output_inside_the_atomic_batch() {
    let mut fixture = Fixture::new();
    custom_source(&mut fixture);
    let mut evaluator = SceneEvaluator::new(
        &fixture.store,
        fixture.root,
        EditTarget::for_layer(GENERATED),
    );
    evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            1,
            CustomGenerator {
                factor: 1.0,
                fail: false,
                read_output: None,
            },
        )
        .unwrap();
    let first = fixture.sync(&mut evaluator);
    evaluator
        .register_generator(
            CUSTOM_SCHEMA,
            2,
            CustomGenerator {
                factor: 1.0,
                fail: false,
                read_output: Some(first.sources[0].output),
            },
        )
        .unwrap();
    let failure = evaluator
        .sync(
            &mut fixture.live,
            &mut fixture.store,
            &mut fixture.compiler,
            Time::Default,
            &CompilePolicy::default(),
        )
        .unwrap_err();
    assert!(matches!(failure.kind, SceneErrorKind::GeneratedInput));
    assert!(!fixture.generated_points(first.sources[0].output).is_empty());
}
