// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construct, compile and query geometry without assembly or scene composition.

use exedra::compile::{CompilePolicy, PartCompiler, PartId, PartInput, PartSource};
use exedra::constructive::{
    builders,
    ir::{CapMode, NodeKind, Placement3, RecipeBuilder},
    tessellate::REGION_CAP_END,
    workplane::{SurfaceInventoryPolicy, WorkplanePolicy, WorkplaneSelection},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(builders::rect_from_corner(2.0, 3.0)?);
    let label = builder.source_ref("panel");
    let root = builder.with_source(label).add(NodeKind::Extrude {
        profile,
        placement: Placement3::IDENTITY,
        height: 1.0,
        caps: CapMode::Both,
    })?;
    let source = PartSource::Recipe(builder.finish(root)?);
    let inputs = [PartInput {
        key: "panel",
        source: &source,
    }];
    let mut compiler = PartCompiler::with_capacity(32);
    let snapshot = compiler.compile_snapshot(&inputs, &CompilePolicy::default())?;
    let body = snapshot.body_by_source(PartId(0), "panel")?;
    let inventory = body.inspect_surfaces(&SurfaceInventoryPolicy::default())?;
    inventory.check(&body)?;
    let plane = body.workplane(
        WorkplaneSelection::Region(REGION_CAP_END),
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        &WorkplanePolicy::default(),
    )?;
    let compiled = snapshot
        .compiled()
        .part(PartId(0))
        .expect("one compiled input");
    println!(
        "Triangles: {}; local bounds: {:?}",
        compiled.triangle_count(),
        compiled.bounds()
    );
    println!(
        "Signed volume: {}",
        compiled.bodies[0].signed_volume()?.value
    );
    let reused = compiler.compile_snapshot(&inputs, &CompilePolicy::default())?;
    assert_eq!(
        compiler.counters().parts_compiled,
        1,
        "unchanged inputs must reuse geometry"
    );
    assert_eq!(
        plane.check(&reused.body(PartId(0), 0).expect("one body")),
        Err(exedra::compile::StaleSelection),
        "fresh snapshots must create fresh selection scopes"
    );
    compiler.clear_cache();
    plane.check(&body)?;
    println!(
        "Work: {:?}; held query survives cache release",
        compiler.counters()
    );
    Ok(())
}
