// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit construction and regeneration on one caller-owned native stage.

use exedra_compile::{CompilePolicy, PartCompiler};
use exedra_constructive::ir::PrimitiveSpec;
use exedra_scene::{
    SceneEvaluator, Time, composition::*, define_primitive, register_schemas, schemas,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = InMemoryStore::default();
    let mut registry = SchemaRegistry::builder();
    schemas::register(
        &mut registry,
        &[
            schemas::Domain::UsdGeom,
            schemas::Domain::UsdProc,
            schemas::Domain::UsdShade,
        ],
        &mut store.tokens,
    )?;
    register_schemas(&mut registry, &mut store.tokens);
    let registry = Arc::new(registry.build(&mut store.tokens));
    let authored = LayerId(1);
    store.insert_layer(Layer::new(authored));
    let mut live = LiveStage::compose(
        &mut store,
        authored,
        StageOptions {
            schemas: Some(registry),
            ..StageOptions::default()
        },
    );
    let root = store.path("/World");
    let panel = store.path("/World/Panel");
    let target = EditTarget::for_layer(authored);
    let mut world = Transaction::new();
    world.create_prim(
        target.prim(root),
        Specifier::Def,
        Some(store.tokens.intern("Xform")),
    );
    live.apply(&mut store, &world)?;
    let construction = define_primitive(
        &mut store,
        &target,
        panel,
        PrimitiveSpec::Box {
            size: [2.0, 0.2, 3.0],
        },
    )?;
    live.apply(&mut store, &construction)?;
    let mut evaluator = SceneEvaluator::new(&store, root, target.clone());
    let mut compiler = PartCompiler::with_capacity(128);
    let first = evaluator.sync(
        &mut live,
        &mut store,
        &mut compiler,
        Time::Default,
        &CompilePolicy::default(),
    )?;
    println!("Initial geometry: {:?}", first.stats);
    let mut bounds =
        schemas::bounds::BoundsCache::new(Time::Default, schemas::bounds::BoundsOptions::default());
    let initial_bound = bounds
        .world_bound(&schemas::Scene::new(live.stage(), &store), panel)
        .expect("published construction has native bounds");
    let size = PropertyPath::new(panel, store.tokens.intern(exedra_scene::SIZE));
    let mut edit = Transaction::new();
    edit.set_default(target.property(size), Value::Vec3d([4.0, 0.2, 3.0]));
    let changed = live.apply(&mut store, &edit)?;
    let second = evaluator.sync(
        &mut live,
        &mut store,
        &mut compiler,
        Time::Default,
        &CompilePolicy::default(),
    )?;
    println!("Dimension change: {:?}", second.stats);
    let scene = schemas::Scene::new(live.stage(), &store);
    bounds.apply_changes(&scene, &changed.changes);
    bounds.apply_changes(&scene, &second.changes);
    assert_eq!(
        bounds
            .world_bound(&scene, panel)
            .expect("regenerated construction has native bounds")
            .aligned_range()
            .max[0],
        4.0,
        "publication changes must invalidate retained bounds"
    );
    let undo = live.apply(&mut store, &changed.inverse)?;
    let restored = evaluator.sync(
        &mut live,
        &mut store,
        &mut compiler,
        Time::Default,
        &CompilePolicy::default(),
    )?;
    let scene = schemas::Scene::new(live.stage(), &store);
    bounds.apply_changes(&scene, &undo.changes);
    bounds.apply_changes(&scene, &restored.changes);
    assert_eq!(
        bounds
            .world_bound(&scene, panel)
            .expect("undo restores native bounds"),
        initial_bound,
        "undo must restore the original procedural bounds"
    );
    assert!(
        first.sources[0].shares_geometry(&restored.sources[0]),
        "undo should reuse original geometry"
    );
    println!("Undo reuses original geometry: {:?}", restored.stats);
    Ok(())
}
