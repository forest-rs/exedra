// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_analytic::extrusion::Extrusion;
use exedra_compile::{PartSource, PreparedSource};
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_scene::{
    SceneErrorKind,
    schemas::procedural::{ProceduralEvaluator, ProceduralInputs},
};

#[test]
fn caller_owned_scene_edit_placement_undo_and_export_are_verified() {
    // WASI has no host temp-dir/PID API; the runner explicitly preopens only `.`.
    // Atomic directory creation gives this invocation ownership even when two
    // test processes share the preopened directory or a prior run was aborted.
    let output = (0..1000)
        .find_map(|index| {
            let path = PathBuf::from(format!(".exedra-native-scene-test-{index}"));
            match std::fs::create_dir(&path) {
                Ok(()) => Some(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => panic!("create test-owned output directory: {error}"),
            }
        })
        .expect("test-owned output directory budget");
    run(&output, 3).expect("complete public scene workflow");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("work-report.json")).unwrap()).unwrap();
    assert_eq!(report["instances"], 3);
    assert_eq!(report["units"], "mm");
    for file in ["scene.usda", "authored.usda", "generated.usda"] {
        let text = std::fs::read_to_string(output.join(file)).unwrap();
        assert!(
            layerstack_usda::parser::parse(&text).diagnostics.is_empty(),
            "publicly exported layer parses"
        );
    }
    std::fs::remove_dir_all(output).expect("release test-owned output directory");
}
#[derive(Clone, Debug)]
struct Constant(PreparedSource);
impl ProceduralEvaluator for Constant {
    type Output = PreparedSource;
    type Error = SceneErrorKind;
    fn system(&self) -> &str {
        panel::SYSTEM
    }
    fn evaluate(&self, _: &mut ProceduralInputs<'_>) -> Result<PreparedSource, SceneErrorKind> {
        Ok(self.0.clone())
    }
}
#[test]
fn native_extent_encloses_unsampled_circle_and_picking_retains_source_scope() {
    let policy = ProfilePolicy::new(1e-6);
    let outline = Loop::new(vec![
        Segment::arc([-5.0, 0.0], 1.0),
        Segment::arc([5.0, 0.0], 1.0),
    ])
    .unwrap();
    let solid = Extrusion::new(
        Profile::new(outline, vec![], &policy).unwrap(),
        exedra_math::Placement3::IDENTITY,
        6.0,
    )
    .unwrap();
    let prepared = PreparedSource::new(PartSource::Native(solid.into()));
    let mut store = InMemoryStore::default();
    let mut registry = SchemaRegistry::builder();
    schemas::register(
        &mut registry,
        &[schemas::Domain::UsdGeom, schemas::Domain::UsdProc],
        &store.tokens,
    )
    .unwrap();
    register_schemas(&mut registry, &mut store.tokens);
    panel::register(&mut registry, &mut store.tokens);
    let registry = Arc::new(registry.build(&store.tokens));
    let root = store.path("/World");
    let asset = store.path("/World/Native");
    store.insert_layer(Layer::new(ROOT));
    store.insert_layer(Layer::new(GENERATED));
    // The caller controls layering; no scene adapter installs these layers.
    *store.layer_mut(ROOT).unwrap().sublayers_mut() = vec![SublayerEntry::new(GENERATED)];
    let mut live = LiveStage::compose(
        &mut store,
        ROOT,
        StageOptions {
            schemas: Some(registry),
            ..StageOptions::default()
        },
    );
    let mut author = Transaction::new();
    author.create_prim(
        EditTarget::for_layer(ROOT).prim(root),
        Specifier::Def,
        Some(store.tokens.intern("Xform")),
    );
    author.create_prim(
        EditTarget::for_layer(ROOT).prim(asset),
        Specifier::Def,
        Some(store.tokens.intern(panel::SCHEMA)),
    );
    live.apply(&mut store, &author).unwrap();
    let mut evaluator = SceneEvaluator::new(&store, root, EditTarget::for_layer(GENERATED));
    evaluator
        .register_generator(panel::SCHEMA, 5, Constant(prepared))
        .unwrap();
    let mut compiler = PartCompiler::new();
    let mut display = CompilePolicy::default();
    display.native.tolerance = 0.8;
    display.native.min_arc_segments = 5;
    let coarse = evaluator
        .sync(
            &mut live,
            &mut store,
            &mut compiler,
            Time::Default,
            &display,
        )
        .unwrap();
    let source = &coarse.sources[0];
    let picked = source.published_native_face(0).unwrap();
    let scene = schemas::Scene::new(live.stage(), &store);
    let mesh = schemas::usd_geom::Mesh::new(&scene, source.output).unwrap();
    let points = mesh.points().unwrap();
    let sampled_max_y = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        sampled_max_y < 4.99,
        "coarse circle deliberately misses the native y extremum"
    );
    let parent = schemas::usd_geom::Boundable::new(&scene, asset)
        .unwrap()
        .extent()
        .unwrap();
    let child = mesh.extent().unwrap();
    assert!(
        parent[1][1] >= 5.0 && parent[0][1] <= -5.0,
        "source extent encloses retained native support"
    );
    assert_eq!(
        child[1][1], sampled_max_y,
        "child extent describes emitted f32 positions"
    );
    let native_ids = mesh
        .find_primvar_with_inheritance("exedra:nativeFace")
        .unwrap()
        .value(Time::Default)
        .unwrap()
        .unwrap();
    assert_eq!(
        schemas::value::read_uint_array(&native_ids, &store.tokens).unwrap()[0],
        picked.native_id().0
    );
    display.native.tolerance = 0.02;
    let fine = evaluator
        .sync(
            &mut live,
            &mut store,
            &mut compiler,
            Time::Default,
            &display,
        )
        .unwrap();
    assert_eq!(fine.stats.source_evaluations, 0);
    assert_eq!(
        fine.sources[0].native().unwrap().resolve_face(&picked),
        Ok(picked.native_id())
    );
    assert!(fine.sources[0].recipe().is_none() && fine.sources[0].report().is_none());
    // A stronger partner can change composed points. Native queries and the
    // retained producer snapshot keep describing their original source.
    let current_scene = schemas::Scene::new(live.stage(), &store);
    let current_points = schemas::usd_geom::Mesh::new(&current_scene, source.output)
        .unwrap()
        .points()
        .unwrap();
    let shifted: Vec<_> = current_points
        .iter()
        .map(|p| [p[0] + 100.0, p[1], p[2]])
        .collect();
    let mut override_points = Transaction::new();
    attribute(
        &mut store,
        &mut override_points,
        &EditTarget::for_layer(ROOT),
        source.output,
        "points",
        "point3f[]",
        Value::TypedArray(TypedArray::Vec3f(Arc::new(shifted))),
    );
    live.apply(&mut store, &override_points).unwrap();
    let scene = schemas::Scene::new(live.stage(), &store);
    assert!(
        schemas::usd_geom::Mesh::new(&scene, source.output)
            .unwrap()
            .points()
            .unwrap()[0][0]
            > 90.0
    );
    assert_eq!(
        source.native().unwrap().resolve_face(&picked),
        Ok(picked.native_id()),
        "native token describes retained producer geometry, not stronger point overrides"
    );
}

#[derive(Debug, PartialEq)]
struct ComposedAttribute {
    name: String,
    type_name: Option<String>,
    variability: Variability,
    custom: bool,
    value: Option<Value>,
}
#[derive(Debug, PartialEq)]
struct ComposedPrim {
    path: String,
    type_name: Option<String>,
    world: Option<[[f64; 4]; 4]>,
    attributes: Vec<ComposedAttribute>,
    property_metadata: Vec<(String, String, Option<ResolvedValue>)>,
    targets: Vec<(String, Vec<String>)>,
    material: Option<String>,
}
fn composed_state(live: &LiveStage, store: &InMemoryStore) -> Vec<ComposedPrim> {
    let scene = schemas::Scene::new(live.stage(), store);
    let mut transforms = schemas::XformCache::new(Time::Default);
    let mut bindings = schemas::BindingCache::new(
        schemas::MaterialPurpose::All,
        schemas::BindingOptions {
            support_legacy_bindings: false,
        },
    );
    let root = store
        .paths
        .lookup(&Path::parse_absolute("/World", &store.tokens).unwrap())
        .unwrap();
    let mut output = Vec::new();
    for path in live.stage().traverse(root) {
        let prim = live.stage().view(store).prim(path).unwrap();
        let mut names = prim.property_names();
        names.sort_unstable();
        let mut attributes = Vec::new();
        let mut property_metadata = Vec::new();
        let mut targets = Vec::new();
        for name in names {
            let property = prim.property(name).unwrap();
            for key in [
                "interpolation",
                "elementSize",
                "bindMaterialAs",
                "unauthoredValuesIndex",
            ] {
                property_metadata.push((
                    name.into(),
                    key.into(),
                    property.metadata(key).map(|v| v.value),
                ));
            }
            if let Some(attribute) = prim.attribute(name) {
                attributes.push(ComposedAttribute {
                    name: name.into(),
                    type_name: attribute.type_name().map(str::to_owned),
                    variability: attribute.variability(),
                    custom: attribute.is_custom(),
                    value: attribute.get(Time::Default).map(|v| v.value),
                });
                let connections = attribute.connections();
                if !connections.is_empty() {
                    targets.push((
                        name.into(),
                        connections
                            .iter()
                            .map(|p| match p {
                                TargetPath::Prim(id) => store.paths.display(*id, &store.tokens),
                                TargetPath::Property(p) => format!(
                                    "{}.{}",
                                    store.paths.display(p.prim_path(), &store.tokens),
                                    store.tokens.resolve(p.property())
                                ),
                            })
                            .collect(),
                    ));
                }
            } else if let Some(relationship) = prim.relationship(name) {
                targets.push((
                    name.into(),
                    relationship
                        .targets()
                        .iter()
                        .map(|p| match p {
                            TargetPath::Prim(id) => store.paths.display(*id, &store.tokens),
                            TargetPath::Property(p) => format!(
                                "{}.{}",
                                store.paths.display(p.prim_path(), &store.tokens),
                                store.tokens.resolve(p.property())
                            ),
                        })
                        .collect(),
                ));
            }
        }
        let material = bindings
            .compute_bound_material(&scene, path)
            .material
            .map(|p| store.paths.display(p, &store.tokens));
        output.push(ComposedPrim {
            path: store.paths.display(path, &store.tokens),
            type_name: prim.type_name().map(str::to_owned),
            world: transforms.local_to_world(&scene, path),
            attributes,
            property_metadata,
            targets,
            material,
        });
    }
    output.sort_by(|a, b| a.path.cmp(&b.path));
    output
}
pub(super) fn assert_clean_equivalence(
    incremental: &SceneSnapshot,
    live: &LiveStage,
    store: &InMemoryStore,
    policy: &CompilePolicy,
) {
    let mut fresh = store.snapshot();
    // Authored inputs/placements/appearance are the same; derived output,
    // composition evidence, generator outputs and compiler caches start empty.
    fresh.insert_layer(Layer::new(GENERATED));
    let mut registry = SchemaRegistry::builder();
    schemas::register(
        &mut registry,
        &[
            schemas::Domain::UsdGeom,
            schemas::Domain::UsdProc,
            schemas::Domain::UsdShade,
        ],
        &fresh.tokens,
    )
    .unwrap();
    register_schemas(&mut registry, &mut fresh.tokens);
    panel::register(&mut registry, &mut fresh.tokens);
    let registry = Arc::new(registry.build(&fresh.tokens));
    let root = fresh.path("/World");
    let mut clean_live = LiveStage::compose(
        &mut fresh,
        ROOT,
        StageOptions {
            schemas: Some(registry),
            ..StageOptions::default()
        },
    );
    let mut clean = SceneEvaluator::new(&fresh, root, EditTarget::for_layer(GENERATED));
    clean
        .register_generator(panel::SCHEMA, 1, panel::PanelGenerator)
        .unwrap();
    let mut compiler = PartCompiler::new();
    let rebuilt = clean
        .sync(
            &mut clean_live,
            &mut fresh,
            &mut compiler,
            Time::Default,
            policy,
        )
        .unwrap();
    assert_eq!(incremental.sources.len(), rebuilt.sources.len());
    for (a, b) in incremental.sources.iter().zip(&rebuilt.sources) {
        assert_eq!(
            a.input().fingerprint(),
            b.input().fingerprint(),
            "canonical geometry inputs match without comparing installation handles"
        );
        assert_eq!(a.native().unwrap().volume(), b.native().unwrap().volume());
        assert_eq!(
            a.native().unwrap().surface_area(),
            b.native().unwrap().surface_area()
        );
        assert_eq!(
            a.native().unwrap().conservative_bounds(),
            b.native().unwrap().conservative_bounds()
        );
    }
    assert_eq!(
        composed_state(live, store),
        composed_state(&clean_live, &fresh),
        "incremental composed geometry, semantic attributes/materials and final placements must match clean regeneration"
    );
}
