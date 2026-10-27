// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tagged native construction on a caller-owned USD stage with repeated placements.

mod panel;
use exedra_compile::{CompilePolicy, NativeSelectionError, PartCompiler};
use exedra_scene::{
    SceneEvaluator, SceneSnapshot, SourceGeometry, Time, composition::*, register_schemas, schemas,
};
use std::{error::Error, path::PathBuf, sync::Arc, time::Instant};

const ROOT: LayerId = LayerId(1);
const AUTHORED: LayerId = LayerId(2);
const GENERATED: LayerId = LayerId(3);
fn attribute(
    store: &mut InMemoryStore,
    transaction: &mut Transaction,
    target: &EditTarget,
    prim: PathId,
    name: &str,
    ty: &str,
    value: Value,
) {
    let mut spec =
        PropertySpec::typed_attribute(PropertyType::parse(ty).expect("registered USD type"))
            .with_default(value);
    if matches!(name, "info:id" | "xformOpOrder") {
        spec = spec.uniform();
    }
    transaction.create_property(
        target.property(PropertyPath::new(prim, store.tokens.intern(name))),
        spec,
    );
}
fn apply_edit(
    label: &str,
    live: &mut LiveStage,
    store: &mut InMemoryStore,
    transaction: &Transaction,
    evidence: &mut Vec<serde_json::Value>,
) -> Result<Applied, Box<dyn Error>> {
    let start = Instant::now();
    let applied = live.apply(store, transaction)?;
    let row = serde_json::json!({"operation":label,"applyMs":start.elapsed().as_secs_f64()*1000.0,"created":applied.changes.created.len(),"removed":applied.changes.removed.len(),"resynced":applied.changes.resynced.len()});
    println!("{row}");
    evidence.push(row);
    Ok(applied)
}
fn record(
    label: &str,
    snapshot: &SceneSnapshot,
    elapsed: std::time::Duration,
) -> serde_json::Value {
    let s = snapshot.stats;
    serde_json::json!({"operation":label,"syncMs":elapsed.as_secs_f64()*1000.0,"sources":snapshot.sources.len(),"discoveredPrims":s.discovered_prims,
        "sourceEvaluations":s.source_evaluations,"inputResolutions":s.input_resolutions,"inputReused":s.input_reused,
        "geometryEvaluations":s.geometry_evaluations,"geometryReused":s.geometry_reused,"polygonEncodings":s.polygon_encodings,"polygonValidations":s.polygon_validations,
        "publicationProperties":s.publication_properties,"meshesWritten":s.meshes_written,"pointsWritten":s.points_written,"sourceBoundsWritten":s.source_bounds_written})
}
fn sync(
    label: &str,
    evaluator: &mut SceneEvaluator,
    live: &mut LiveStage,
    store: &mut InMemoryStore,
    compiler: &mut PartCompiler,
    policy: &CompilePolicy,
    evidence: &mut Vec<serde_json::Value>,
) -> Result<SceneSnapshot, Box<dyn Error>> {
    let start = Instant::now();
    let result = evaluator.sync(live, store, compiler, Time::Default, policy)?;
    let row = record(label, &result, start.elapsed());
    println!("{row}");
    evidence.push(row);
    #[cfg(test)]
    tests::assert_clean_equivalence(&result, live, store, policy);
    Ok(result)
}
fn native_checks(
    source: &SourceGeometry,
) -> Result<exedra_compile::NativeFaceSelection, Box<dyn Error>> {
    assert!(
        source.recipe().is_none(),
        "native input must not masquerade as a recipe"
    );
    assert!(
        source.report().is_none(),
        "native input has no constructive evaluation report"
    );
    let native = source.native().expect("retained native construction");
    let solid = native
        .as_extrusion()
        .expect("panel generator produces an extrusion");
    let floor = solid.floor_tagged(panel::RECESS)?;
    assert!(
        (native.volume()? - (108864.0 - 5.0 * (512.0 + 64.0 * std::f64::consts::PI))).abs() < 1e-7,
        "exact native volume"
    );
    let body = source.body(0).expect("one native body");
    let map = body
        .native_realization()
        .expect("native display correspondence");
    assert!(map.report.within_tolerance(), "display target must be met");
    let polygon = map
        .mesh_faces
        .iter()
        .position(|&(_, id)| id == floor)
        .expect("published floor polygon");
    let picked = source
        .published_native_face(polygon)
        .expect("generated polygon maps to source");
    assert_eq!(
        picked.native_id(),
        floor,
        "picking follows actual generated polygon order"
    );
    assert_eq!(
        solid.face_tag(floor),
        Some(panel::RECESS),
        "feature identity survives realization"
    );
    assert_eq!(
        native.face_tags(floor)?.collect::<Vec<_>>(),
        vec![panel::RECESS],
        "generic feature inspection preserves the floor tag"
    );
    assert_eq!(
        native.planar_frame(floor)?.rows.map(|r| r[3]),
        [0.0, 0.0, 7.0],
        "exact native floor frame precedes any scene placement"
    );
    Ok(picked)
}
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let output = PathBuf::from(args.next().unwrap_or_else(|| "native-scene-output".into()));
    let instances = args
        .next()
        .map(|s| s.parse::<u32>())
        .transpose()?
        .unwrap_or(1000);
    run(&output, instances)
}

fn run(output: &std::path::Path, instances: u32) -> Result<(), Box<dyn Error>> {
    if instances > 10_000 {
        return Err("sample supports at most 10000 placements".into());
    }
    let mut store = InMemoryStore::default();
    let mut registry = SchemaRegistry::builder();
    schemas::register(
        &mut registry,
        &[
            schemas::Domain::UsdGeom,
            schemas::Domain::UsdProc,
            schemas::Domain::UsdShade,
        ],
        &store.tokens,
    )?;
    register_schemas(&mut registry, &mut store.tokens);
    panel::register(&mut registry, &mut store.tokens);
    let registry = Arc::new(registry.build(&store.tokens));
    let world = store.path("/World");
    let asset = store.path("/World/PanelAsset");
    let placements = store.path("/World/Placements");
    let mut root = Layer::new(ROOT);
    *root.sublayers_mut() = vec![
        SublayerEntry::with_asset(AUTHORED, "authored.usda", LayerOffset::IDENTITY),
        SublayerEntry::with_asset(GENERATED, "generated.usda", LayerOffset::IDENTITY),
    ];
    root.set_default_prim(Some(store.tokens.intern("World")));
    root.metadata_fields_mut().push(FieldEntry {
        name: store.tokens.intern("metersPerUnit"),
        value: FieldValue::Value(Value::Double(0.001)),
    });
    root.metadata_fields_mut().push(FieldEntry {
        name: store.tokens.intern("upAxis"),
        value: FieldValue::Value(Value::Token(store.tokens.intern("Z"))),
    });
    root.metadata_fields_mut().push(FieldEntry {
        name: store.tokens.intern("fallbackPrimTypes"),
        value: FieldValue::Value(Value::Dictionary(vec![(
            panel::SCHEMA.into(),
            Value::TypedArray(TypedArray::Token(
                vec![store.tokens.intern("GenerativeProcedural")].into(),
            )),
        )])),
    });
    store.insert_layer(root);
    store.insert_layer(Layer::new(AUTHORED));
    store.insert_layer(Layer::new(GENERATED));
    let mut live = LiveStage::compose(
        &mut store,
        ROOT,
        StageOptions {
            schemas: Some(registry),
            ..StageOptions::default()
        },
    );
    let target = EditTarget::for_layer(ROOT);
    let author_target = EditTarget::for_layer(AUTHORED);
    let mut author = Transaction::new();
    author.create_prim(
        target.prim(world),
        Specifier::Def,
        Some(store.tokens.intern("Xform")),
    );
    author.create_prim(
        target.prim(placements),
        Specifier::Def,
        Some(store.tokens.intern("Xform")),
    );
    author.create_prim(
        author_target.prim(asset),
        Specifier::Def,
        Some(store.tokens.intern(panel::SCHEMA)),
    );
    let system = store.tokens.intern(panel::SYSTEM);
    attribute(
        &mut store,
        &mut author,
        &author_target,
        asset,
        "proceduralSystem",
        "token",
        Value::Token(system),
    );
    attribute(
        &mut store,
        &mut author,
        &author_target,
        asset,
        panel::WIDTH,
        "double",
        Value::Double(120.0),
    );
    attribute(
        &mut store,
        &mut author,
        &author_target,
        asset,
        panel::DEPTH,
        "double",
        Value::Double(5.0),
    );
    attribute(
        &mut store,
        &mut author,
        &target,
        asset,
        "primvars:displayColor",
        "color3f[]",
        Value::TypedArray(TypedArray::Vec3f(Arc::new(vec![[0.2, 0.5, 0.7]]))),
    );
    let mut materials = Vec::new();
    for (name, color) in [("Blue", [0.15, 0.4, 0.7]), ("Gold", [0.7, 0.45, 0.15])] {
        let material = store.path(&format!("/World/PanelAsset/Looks/{name}"));
        let shader = store.path(&format!("/World/PanelAsset/Looks/{name}/Surface"));
        author.create_prim(
            target.prim(material),
            Specifier::Def,
            Some(store.tokens.intern("Material")),
        );
        author.create_prim(
            target.prim(shader),
            Specifier::Def,
            Some(store.tokens.intern("Shader")),
        );
        let preview = store.tokens.intern("UsdPreviewSurface");
        attribute(
            &mut store,
            &mut author,
            &target,
            shader,
            "info:id",
            "token",
            Value::Token(preview),
        );
        attribute(
            &mut store,
            &mut author,
            &target,
            shader,
            "inputs:diffuseColor",
            "color3f",
            Value::Vec3f(color),
        );
        attribute(
            &mut store,
            &mut author,
            &target,
            shader,
            "inputs:roughness",
            "float",
            Value::Float(0.4),
        );
        let surface = store.tokens.intern("outputs:surface");
        let output = PropertyPath::new(shader, surface);
        let material_output = PropertyPath::new(material, surface);
        author.create_property(
            target.property(output),
            PropertySpec::typed_attribute(PropertyType::parse("token").unwrap()),
        );
        author.create_property(
            target.property(material_output),
            PropertySpec::typed_attribute(PropertyType::parse("token").unwrap()),
        );
        author.set_targets(
            target.property(material_output),
            ListOp::explicit(vec![TargetPath::Property(output)]),
        );
        materials.push(material);
    }
    let binding = PropertyPath::new(asset, store.tokens.intern("material:binding"));
    author.add_applied_schema(
        target.prim(asset),
        store.tokens.intern("MaterialBindingAPI"),
    );
    author.create_property(
        target.property(binding),
        PropertySpec::relationship()
            .with_targets(ListOp::explicit(vec![TargetPath::Prim(materials[0])])),
    );
    let translate = store.tokens.intern("xformOp:translate");
    let mut first = None;
    for index in 0..instances {
        let path = store.path(&format!("/World/Placements/P{index:04}"));
        if first.is_none() {
            first = Some(path);
        }
        author.create_prim(target.prim(path), Specifier::Def, None);
        author.set_instanceable(target.prim(path), true);
        author.add_reference(
            target.prim(path),
            Reference::new(ROOT, asset),
            ListPosition::BackOfPrepend,
        );
        attribute(
            &mut store,
            &mut author,
            &target,
            path,
            "xformOp:translate",
            "double3",
            Value::Vec3d([
                180.0 + f64::from(index % 40) * 140.0,
                f64::from(index / 40) * 100.0,
                0.0,
            ]),
        );
        attribute(
            &mut store,
            &mut author,
            &target,
            path,
            "xformOpOrder",
            "token[]",
            Value::TypedArray(TypedArray::Token(vec![translate].into())),
        );
    }
    let mut stage_edits = Vec::new();
    apply_edit(
        "initial-authoring",
        &mut live,
        &mut store,
        &author,
        &mut stage_edits,
    )?;
    let mut evaluator = SceneEvaluator::new(&store, world, EditTarget::for_layer(GENERATED));
    evaluator.register_generator(panel::SCHEMA, 1, panel::PanelGenerator)?;
    let mut compiler = PartCompiler::with_capacity(16);
    let mut policy = CompilePolicy::default();
    policy.native.tolerance = 0.2;
    let mut evidence = Vec::new();
    let initial = sync(
        "initial",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        initial.sources.len(),
        1,
        "instances consume one authored source"
    );
    let picked = native_checks(&initial.sources[0])?;
    let unchanged = sync(
        "no-op",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        (
            unchanged.stats.discovered_prims,
            unchanged.stats.source_evaluations,
            unchanged.stats.geometry_evaluations,
            unchanged.stats.polygon_encodings,
            unchanged.stats.meshes_written
        ),
        (0, 0, 0, 0, 0),
        "unchanged geometry must not be rebuilt or rewritten"
    );
    let mut appearance = Transaction::new();
    appearance.set_default(
        target.property(PropertyPath::new(
            asset,
            store.tokens.intern("primvars:displayColor"),
        )),
        Value::TypedArray(TypedArray::Vec3f(Arc::new(vec![[0.3, 0.6, 0.4]]))),
    );
    apply_edit(
        "attribute",
        &mut live,
        &mut store,
        &appearance,
        &mut stage_edits,
    )?;
    let attributes = sync(
        "attribute",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        (
            attributes.stats.geometry_evaluations,
            attributes.stats.polygon_encodings,
            attributes.stats.meshes_written
        ),
        (0, 0, 0),
        "appearance attributes must not regenerate native geometry"
    );
    let mut material = Transaction::new();
    material.set_targets(
        target.property(binding),
        ListOp::explicit(vec![TargetPath::Prim(materials[1])]),
    );
    apply_edit(
        "material",
        &mut live,
        &mut store,
        &material,
        &mut stage_edits,
    )?;
    let rebound = sync(
        "material",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        (
            rebound.stats.geometry_evaluations,
            rebound.stats.polygon_encodings,
            rebound.stats.meshes_written
        ),
        (0, 0, 0),
        "material binding must remain native scene data"
    );
    let scene = schemas::Scene::new(live.stage(), &store);
    let mut bindings = schemas::BindingCache::new(
        schemas::MaterialPurpose::All,
        schemas::BindingOptions {
            support_legacy_bindings: false,
        },
    );
    assert_eq!(
        bindings
            .compute_bound_material(&scene, initial.sources[0].output)
            .material,
        Some(materials[1]),
        "native inherited material binding changes independently of geometry"
    );
    let depth = PropertyPath::new(asset, store.tokens.intern(panel::DEPTH));
    let mut edit = Transaction::new();
    edit.set_default(author_target.property(depth), Value::Double(6.0));
    let edited = apply_edit(
        "pocket-depth",
        &mut live,
        &mut store,
        &edit,
        &mut stage_edits,
    )?;
    let changed = sync(
        "pocket-depth",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        (
            changed.stats.discovered_prims,
            changed.stats.source_evaluations,
            changed.stats.geometry_evaluations,
            changed.stats.meshes_written
        ),
        (0, 1, 1, 1),
        "one edited native source must cause one geometry update"
    );
    assert_eq!(
        changed.sources[0].native().unwrap().resolve_face(&picked),
        Err(NativeSelectionError::SourceMismatch),
        "parameter edits install a new native scope"
    );
    if let Some(first) = first {
        let mut move_one = Transaction::new();
        move_one.set_default(
            target.property(PropertyPath::new(first, translate)),
            Value::Vec3d([200.0, 20.0, 0.0]),
        );
        apply_edit(
            "placement",
            &mut live,
            &mut store,
            &move_one,
            &mut stage_edits,
        )?;
        let moved = sync(
            "placement",
            &mut evaluator,
            &mut live,
            &mut store,
            &mut compiler,
            &policy,
            &mut evidence,
        )?;
        assert_eq!(
            (
                moved.stats.discovered_prims,
                moved.stats.source_evaluations,
                moved.stats.geometry_evaluations,
                moved.stats.polygon_encodings,
                moved.stats.meshes_written
            ),
            (0, 0, 0, 0, 0),
            "unchanged geometry must not be rebuilt or rewritten"
        );
        assert!(
            moved.sources[0]
                .native()
                .unwrap()
                .same_installation(changed.sources[0].native().unwrap()),
            "placement must preserve native source installation"
        );
        let scene = schemas::Scene::new(live.stage(), &store);
        let mut transforms = schemas::XformCache::new(Time::Default);
        assert_eq!(
            &transforms.local_to_world(&scene, first).unwrap()[3][..3],
            &[200.0, 20.0, 0.0],
            "the native stage must carry the changed placement"
        );
    }
    apply_edit(
        "undo",
        &mut live,
        &mut store,
        &edited.inverse,
        &mut stage_edits,
    )?;
    let restored = sync(
        "undo",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        restored.stats.geometry_evaluations, 0,
        "undo must reuse cached geometry"
    );
    assert_eq!(
        restored.stats.polygon_encodings, 0,
        "undo must reuse cached USD encoding"
    );
    assert!(
        restored.sources[0].shares_geometry(&initial.sources[0]),
        "undo reuses original compiled buffers"
    );
    let selection = native_checks(&restored.sources[0])?;
    policy.native.tolerance = 0.02;
    let fine = sync(
        "display-quality",
        &mut evaluator,
        &mut live,
        &mut store,
        &mut compiler,
        &policy,
        &mut evidence,
    )?;
    assert_eq!(
        fine.stats.source_evaluations, 0,
        "display policy does not lower native source"
    );
    assert_eq!(
        fine.sources[0].native().unwrap().resolve_face(&selection),
        Ok(selection.native_id()),
        "native face identity must survive display policy changes"
    );
    let encoding_payload_bytes = evaluator.cached_payload_bytes();
    compiler.clear_cache();
    evaluator.clear_cache();
    drop(compiler);
    drop(evaluator);
    let _ = native_checks(&initial.sources[0])?;
    std::fs::create_dir_all(output)?;
    for (layer, name) in [
        (ROOT, "scene.usda"),
        (AUTHORED, "authored.usda"),
        (GENERATED, "generated.usda"),
    ] {
        let text = layerstack_usda::save::save_usda(
            store.layer(layer).unwrap(),
            &store.tokens,
            &store.paths,
        )?;
        assert!(
            layerstack_usda::parser::parse(&text).diagnostics.is_empty(),
            "saved USDA must parse"
        );
        std::fs::write(output.join(name), text)?;
    }
    let summary = serde_json::json!({"units":"mm","metersPerUnit":0.001,"upAxis":"Z","instances":instances,"layerstackRevision":"08d51a9af3770eff3fc0bf7f82a05d78c2ab91d6","generatorRevision":fine.sources[0].generator.revision,"operations":evidence,"stageEdits":stage_edits,
        "nativeVolume":fine.sources[0].native().unwrap().volume()?,"nativeArea":fine.sources[0].native().unwrap().surface_area()?,"nativeBounds":fine.sources[0].native().unwrap().conservative_bounds()?,"publishedSource":fine.sources[0].source,
        "pickingScope":"retained generated asset polygons, not composed partner overrides","nativeFaceProperty":exedra_scene::NATIVE_FACE,"encodingPayloadBytesBeforeClear":encoding_payload_bytes,"heldSnapshotAfterEviction":true});
    std::fs::write(
        output.join("work-report.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    println!(
        "Saved native USD layers and work-report.json to {}",
        output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests;
