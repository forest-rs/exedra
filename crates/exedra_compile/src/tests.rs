// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::sync::Arc;
use exedra_constructive::{
    builders,
    ir::{CapMode, NodeKind, Placement3, RecipeBuilder},
    tessellate::REGION_CAP_END,
    workplane::{SurfaceInventoryPolicy, WorkplanePolicy, WorkplaneSelection},
};

fn panel(width: f64) -> PartSource {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(builders::rect_from_corner(width, 3.0).unwrap());
    let label = builder.source_ref("panel");
    let root = builder
        .with_source(label)
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 1.0,
            caps: CapMode::Both,
        })
        .unwrap();
    PartSource::Recipe(builder.finish(root).unwrap())
}

#[test]
fn independent_inputs_share_compilation_and_keep_queries_after_eviction() {
    let source = panel(2.0);
    let inputs = [
        PartInput {
            key: "first",
            source: &source,
        },
        PartInput {
            key: "alias",
            source: &source,
        },
    ];
    let mut compiler = PartCompiler::new();
    let snapshot = compiler
        .compile_snapshot(&inputs, &CompilePolicy::default())
        .unwrap();
    assert_eq!(compiler.counters().parts_compiled, 1);
    assert!(
        Arc::ptr_eq(
            snapshot.compiled().part(PartId(0)).unwrap(),
            snapshot.compiled().part(PartId(1)).unwrap()
        ),
        "geometry identity must be independent of caller keys"
    );
    assert_eq!(snapshot.compiled().validate_for(&inputs), Ok(()));
    let body = snapshot.body_by_source(PartId(0), "panel").unwrap();
    let plane = body
        .workplane(
            WorkplaneSelection::Region(REGION_CAP_END),
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            &WorkplanePolicy::default(),
        )
        .unwrap();
    let inventory = body
        .inspect_surfaces(&SurfaceInventoryPolicy::default())
        .unwrap();
    let again = compiler
        .compile_snapshot(&inputs, &CompilePolicy::default())
        .unwrap();
    assert_eq!(compiler.counters().parts_compiled, 1);
    let next = again.body_by_source(PartId(0), "panel").unwrap();
    assert!(
        core::ptr::eq(body.geometry(), next.geometry()),
        "cache reuse must share topology"
    );
    assert_eq!(plane.check(&next), Err(StaleSelection));
    assert_eq!(inventory.check(&next), Err(StaleSelection));
    compiler.clear_cache();
    assert_eq!(plane.check(&body), Ok(()));
    assert_eq!(inventory.check(&body), Ok(()));
    assert_eq!(
        snapshot.compiled().part(PartId(0)).unwrap().bodies[0]
            .signed_volume()
            .unwrap()
            .value,
        6.0
    );
}

#[test]
fn bounded_retention_and_render_only_upgrade_preserve_owned_results() {
    let first = panel(2.0);
    let second = panel(4.0);
    let policy = CompilePolicy::default();
    let mut compiler = PartCompiler::with_capacity(1);
    let a = [PartInput {
        key: "a",
        source: &first,
    }];
    let buffers = compiler.compile_parts(&a, &policy).unwrap();
    assert_eq!(compiler.retained_entries(), 0);
    let geometry = compiler.compile_snapshot(&a, &policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 2);
    assert_eq!(compiler.retained_entries(), 1);
    assert_eq!(
        buffers.part(PartId(0)).unwrap().bodies[0].tri.indices,
        geometry.compiled().part(PartId(0)).unwrap().bodies[0]
            .tri
            .indices
    );
    compiler
        .compile_snapshot(
            &[PartInput {
                key: "b",
                source: &second,
            }],
            &policy,
        )
        .unwrap();
    assert_eq!(compiler.cached_entries(), 1);
    assert_eq!(compiler.counters().cache_evictions, 1);
    assert_eq!(
        geometry
            .body(PartId(0), 0)
            .unwrap()
            .geometry()
            .mesh
            .faces()
            .count(),
        6
    );
    compiler.compile_snapshot(&a, &policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 4);
}

#[test]
fn baked_inputs_need_no_construction_or_scene_records() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PartCompiler>();
    assert_send_sync::<GeometrySnapshot>();
    assert_send_sync::<SnapshotBody>();
    assert_send_sync::<PartSnapshot>();
    let mesh = exedra_mesh::Mesh::from_polygons(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[&[0, 1, 2]],
    )
    .unwrap();
    let source = PartSource::Baked(mesh);
    let mut compiler = PartCompiler::new();
    let snapshot = compiler
        .compile_snapshot(
            &[PartInput {
                key: "import",
                source: &source,
            }],
            &CompilePolicy::default(),
        )
        .unwrap();
    assert_eq!(snapshot.compiled().report(PartId(0)), None);
    assert_eq!(
        snapshot
            .compiled()
            .part(PartId(0))
            .unwrap()
            .triangle_count(),
        1
    );
    assert_eq!(snapshot.body(PartId(0), 0).unwrap().node(), None);
}

#[test]
fn a_retained_part_does_not_keep_other_input_buffers_alive() {
    let first = panel(2.0);
    let second = panel(4.0);
    let mut compiler = PartCompiler::new();
    let snapshot = compiler
        .compile_snapshot(
            &[
                PartInput {
                    key: "first",
                    source: &first,
                },
                PartInput {
                    key: "second",
                    source: &second,
                },
            ],
            &CompilePolicy::default(),
        )
        .unwrap();
    let peer = Arc::downgrade(snapshot.compiled().part(PartId(1)).unwrap());
    let retained = snapshot.part(PartId(0)).unwrap();
    drop(snapshot);
    compiler.clear_cache();
    assert!(
        peer.upgrade().is_none(),
        "retaining one part must release unrelated compiled buffers"
    );
    assert_eq!(retained.compiled().triangle_count(), 12);
    assert_eq!(
        retained
            .body_by_source("panel")
            .unwrap()
            .geometry()
            .mesh
            .faces()
            .count(),
        6
    );
    assert!(
        retained.report().is_some(),
        "the retained part must own its report independently"
    );
}
