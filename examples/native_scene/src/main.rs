// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit construction and regeneration on one caller-owned native stage.

use exedra_constructive::{ir::PrimitiveSpec, tessellate::EvalPolicy};
use exedra_scene::{
    SceneEvaluator, Time, composition::*, define_primitive, register_schemas, schemas,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut store = InMemoryStore::default();
    let mut registry = SchemaRegistry::builder();
    schemas::register(
        &mut registry,
        &[schemas::Domain::UsdGeom, schemas::Domain::UsdShade],
        &mut store.tokens,
    )?;
    register_schemas(&mut registry, &mut store.tokens);
    let registry = Arc::new(registry.build(&mut store.tokens));
    let generated = LayerId(1);
    let authored = LayerId(2);
    store.insert_layer(Layer::new(generated));
    let mut layer = Layer::new(authored);
    layer.sublayers.push(SublayerEntry::new(generated));
    store.insert_layer(layer);
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
    let construction = define_primitive(
        &mut store,
        &target,
        panel,
        PrimitiveSpec::Box {
            size: [2.0, 0.2, 3.0],
        },
    )?;
    live.apply(&mut store, &construction)?;
    let mut evaluator = SceneEvaluator::new(root, generated);
    let first = evaluator.sync(&mut live, &mut store, Time::Default, &EvalPolicy::default())?;
    println!("Initial geometry: {:?}", first.stats);
    let size = PropertyPath::new(panel, store.tokens.intern(exedra_scene::SIZE));
    let mut edit = Transaction::new();
    edit.set_default(target.property(size), Value::Vec3d([4.0, 0.2, 3.0]));
    let changed = live.apply(&mut store, &edit)?;
    let second = evaluator.sync(&mut live, &mut store, Time::Default, &EvalPolicy::default())?;
    println!("Dimension change: {:?}", second.stats);
    live.apply(&mut store, &changed.inverse)?;
    let restored = evaluator.sync(&mut live, &mut store, Time::Default, &EvalPolicy::default())?;
    assert!(
        first.sources[0].shares_geometry(&restored.sources[0]),
        "undo should reuse original geometry"
    );
    println!("Undo reuses original geometry: {:?}", restored.stats);
    Ok(())
}
