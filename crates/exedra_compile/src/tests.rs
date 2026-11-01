// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::sync::Arc;
use exedra_constructive::{
    builders,
    ir::{CapMode, Node, NodeKind, Placement3, RecipeBuilder},
    tessellate::REGION_CAP_END,
    workplane::{SurfaceInventoryPolicy, WorkplanePolicy, WorkplaneSelection},
};

fn panel(width: f64) -> PartSource {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(builders::rect_from_corner(width, 3.0).unwrap());
    let label = builder.source_ref("panel");
    let root = builder
        .add_node(
            Node::new(NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 1.0,
                caps: CapMode::Both,
            })
            .with_source(label),
        )
        .unwrap();
    PartSource::Recipe(builder.finish(root).unwrap())
}

#[test]
fn independent_inputs_share_compilation_and_keep_queries_after_eviction() {
    let source = panel(2.0);
    let inputs = [
        PartInput::new("first", &source),
        PartInput::new("alias", &source),
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
    let a = [PartInput::new("a", &first)];
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
        .compile_snapshot(&[PartInput::new("b", &second)], &policy)
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
fn repeated_inputs_share_one_compilation_under_any_capacity() {
    let (first, second) = (panel(2.0), panel(4.0));
    let inputs = [
        PartInput::new("a", &first),
        PartInput::new("b", &second),
        PartInput::new("a again", &first),
    ];
    let policy = CompilePolicy::default();
    let mut compiler = PartCompiler::with_capacity(1);
    let compiled = compiler.compile_parts(&inputs, &policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 2);
    assert_eq!(compiler.counters().cache_hits, 1);
    assert!(Arc::ptr_eq(
        compiled.part(PartId(0)).unwrap(),
        compiled.part(PartId(2)).unwrap()
    ));
    assert_eq!(compiled.validate_for(&inputs), Ok(()));

    let snapshot = compiler.compile_snapshot(&inputs, &policy).unwrap();
    assert_eq!(compiler.counters().parts_compiled, 4);
    let (body, alias) = (
        snapshot.body_by_source(PartId(0), "panel").unwrap(),
        snapshot.body_by_source(PartId(2), "panel").unwrap(),
    );
    assert!(core::ptr::eq(body.geometry(), alias.geometry()));
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
            &[PartInput::new("import", &source)],
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
                PartInput::new("first", &first),
                PartInput::new("second", &second),
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

#[test]
fn prepared_sources_bind_geometry_and_identity_across_reinstallation() {
    let original = PreparedSource::new(PartSource::Baked(exedra_mesh::Mesh::new()));
    let inputs = [PartInput::prepared("original", &original)];
    assert_eq!(inputs[0].fingerprint(), part_fingerprint(original.source()));
    let compiled = PartCompiler::new()
        .compile_parts(&inputs, &CompilePolicy::default())
        .unwrap();
    compiled.validate_for(&inputs).unwrap();
    let copied = original.clone();
    compiled
        .validate_for(&[PartInput::prepared("renamed", &copied)])
        .unwrap();
    let mut builder = RecipeBuilder::new();
    let root = builder
        .add_node(NodeKind::Primitive {
            spec: exedra_constructive::ir::PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let replacement = PreparedSource::new(PartSource::Recipe(builder.finish(root).unwrap()));
    assert!(matches!(
        compiled.validate_for(&[PartInput::prepared("replacement", &replacement)]),
        Err(CompilationMismatch::PartContent { .. })
    ));
    let restored = PreparedSource::new(original.into_source());
    assert_eq!(restored.fingerprint(), copied.fingerprint());
}

#[test]
fn public_extraction_settings_match_compiled_concave_mesh_buffers() {
    let mesh = exedra_mesh::Mesh::from_polygons(
        &[
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ],
        &[&[0, 1, 2, 3, 4, 5]],
    )
    .unwrap();
    let policy = CompilePolicy {
        uvs: exedra_mesh::UvSource::CustomOrBoxProjected { scale: 2.0 },
        ..CompilePolicy::default()
    };
    let (expected, _) = mesh.to_trimesh(&policy.extraction_params());
    let source = PartSource::Baked(mesh);
    let compiled = PartCompiler::new()
        .compile_parts(&[PartInput::new("concave", &source)], &policy)
        .unwrap();
    let actual = &compiled.part(PartId(0)).unwrap().bodies[0].tri;
    assert_eq!(actual.positions, expected.positions);
    assert_eq!(actual.normals, expected.normals);
    assert_eq!(actual.uvs, expected.uvs);
    assert_eq!(actual.indices, expected.indices);
}

#[test]
fn render_vertex_limit_covers_recipe_and_baked_sources_per_body() {
    let recipe = panel(2.0);
    let baked = PartSource::Baked(
        exedra_mesh::Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            &[[0, 1, 2]],
            &exedra_mesh::BuildParams::default(),
        )
        .unwrap(),
    );
    for source in [&recipe, &baked] {
        let inputs = [PartInput::new("one", source), PartInput::new("two", source)];
        let mut compiler = PartCompiler::new();
        let uncapped = compiler
            .compile_parts(&inputs, &CompilePolicy::default())
            .unwrap();
        let count = u32::try_from(
            uncapped.part(PartId(0)).unwrap().bodies[0]
                .tri
                .positions
                .len(),
        )
        .unwrap();
        let mut policy = CompilePolicy {
            max_render_vertices: Some(count),
            ..Default::default()
        };
        let output = compiler.compile_parts(&inputs, &policy).unwrap();
        for part in [PartId(0), PartId(1)] {
            assert_eq!(
                output.part(part).unwrap().bodies[0].tri,
                uncapped.part(part).unwrap().bodies[0].tri
            );
        }
        // The sum exceeds the per-body limit; both bodies still succeed.
        assert!(2 * count > count);
        let entries = compiler.cached_entries();
        policy.max_render_vertices = Some(count - 1);
        assert!(matches!(compiler.compile_parts(&inputs, &policy),
            Err(CompileError::RenderVertices { part: PartId(0), part_key, body: 0, error })
            if part_key == "one" && error.maximum == count - 1 && error.required_at_least == u64::from(count)));
        assert_eq!(compiler.cached_entries(), entries);
    }
}

#[test]
fn each_native_triangle_quality_setting_changes_compilation_identity() {
    use exedra_analytic::realize::TriangleQuality;
    let base = CompilePolicy::default();
    let mut quality = base.clone();
    quality.native.quality = Some(TriangleQuality::new(4.0));
    let identity = policy_fingerprint(&quality);
    assert_ne!(identity, policy_fingerprint(&base));
    for settings in [
        TriangleQuality::new(2.0),
        TriangleQuality::new(4.0).with_max_radius_edge_ratio(1.5),
        TriangleQuality::new(4.0).with_max_steiner_points(17),
        TriangleQuality::new(4.0).with_max_candidate_insertions(23),
        TriangleQuality::new(4.0).with_max_grid_triangle_tests(29),
    ] {
        let mut changed = quality.clone();
        changed.native.quality = Some(settings);
        assert_ne!(identity, policy_fingerprint(&changed));
    }
}
