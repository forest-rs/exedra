// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use exedra_compile::{CompilePolicy, PartCompiler, PartInput, PartSource};
use exedra_mesh::{BuildParams, Mesh};
use std::sync::Arc;

fn geometry(size: f32) -> PartSource {
    PartSource::Baked(
        Mesh::from_indexed_triangles(
            &[[0.0, 0.0, 0.0], [size, 0.0, 0.0], [0.0, size, 0.0]],
            &[[0, 1, 2]],
            &BuildParams::default(),
        )
        .unwrap(),
    )
}
fn compiled() -> CompiledParts {
    let source = geometry(1.0);
    PartCompiler::new()
        .compile_parts(
            &[PartInput::new("caller geometry", &source)],
            &CompilePolicy::default(),
        )
        .unwrap()
}
fn drawable(name: &str, compiled: &CompiledParts) -> GltfNode {
    let mut node = GltfNode::new(name);
    node.geometry = Some(
        GltfMesh::new(PartId(0), "surface", compiled)
            .unwrap()
            .into(),
    );
    node
}
#[test]
fn caller_owned_hierarchy_and_resolved_materials_share_geometry() {
    let compiled = compiled();
    let mut left = drawable("caller:left", &compiled);
    left.parent = Some(2);
    left.geometry.as_mut().unwrap().mesh.materials = vec![vec![Some("oak".into())]];
    let mut right = drawable("caller:right", &compiled);
    right.parent = Some(2);
    right.transform = Placement3::translate(2.0, 0.0, 0.0);
    right.geometry.as_mut().unwrap().mesh.materials = vec![vec![Some("paint".into())]];
    let scene = GltfScene {
        nodes: vec![left, right, GltfNode::new("caller:frame")],
        ..GltfScene::default()
    };
    let export = export_glb(&scene, &compiled).unwrap();
    assert_eq!(export_glb(&scene, &compiled).unwrap().bytes, export.bytes);
    let document = GlbDocument::parse(&export.bytes).unwrap();
    let json = document.json();
    assert_eq!(json["scenes"][0]["nodes"], json!([2]));
    assert_eq!(json["nodes"][2]["children"], json!([0, 1]));
    assert_eq!(export.stats.meshes, 2);
    assert_eq!(
        json["meshes"][0]["primitives"][0]["attributes"],
        json["meshes"][1]["primitives"][0]["attributes"]
    );
    assert_eq!(document.triangle_count(), 2);
}
#[test]
fn invalid_records_fail_before_resolving_resources() {
    let compiled = compiled();
    let mut scene = GltfScene {
        nodes: vec![drawable("a", &compiled), drawable("b", &compiled)],
        ..GltfScene::default()
    };
    let never_resolve =
        |_: &str| -> Option<Value> { panic!("invalid structure must fail before material lookup") };
    scene.nodes[0].parent = Some(1);
    scene.nodes[1].parent = Some(0);
    assert!(matches!(
        export_glb_with_materials(
            &scene,
            &compiled,
            &never_resolve,
            GltfExportOptions::default()
        ),
        Err(GltfError::InvalidInput {
            reason: "cyclic hierarchy",
            ..
        })
    ));
    scene.nodes[1].parent = Some(2);
    assert!(matches!(
        export_glb(&scene, &compiled),
        Err(GltfError::InvalidInput {
            reason: "unknown parent node",
            ..
        })
    ));
    scene.nodes[1].parent = None;
    scene.nodes[0].transform.rows[0][3] = f64::NAN;
    assert!(matches!(
        export_glb(&scene, &compiled),
        Err(GltfError::InvalidInput {
            reason: "nonfinite transform",
            ..
        })
    ));
    scene.nodes[0].transform = Placement3::IDENTITY;
    scene.nodes[0].geometry.as_mut().unwrap().mesh.materials = vec![vec![None, None]];
    assert!(matches!(
        export_glb(&scene, &compiled),
        Err(GltfError::InvalidInput {
            reason: "region count mismatch",
            ..
        })
    ));
}
#[test]
fn captured_content_and_placement_attribute_counts_are_checked() {
    let compiled = compiled();
    let source = geometry(2.0);
    let replacement = PartCompiler::new()
        .compile_parts(
            &[PartInput::new("replacement", &source)],
            &CompilePolicy::default(),
        )
        .unwrap();
    let node = drawable("held geometry", &compiled);
    let mut scene = GltfScene {
        nodes: vec![node.clone()],
        ..GltfScene::default()
    };
    assert!(matches!(
        export_glb(&scene, &replacement),
        Err(GltfError::CompilationMismatch(
            CompilationMismatch::PartContent { .. }
        ))
    ));
    scene.placement_sets.push(GltfPlacementSet {
        name: "caller scatter".into(),
        parent: None,
        geometry: node.geometry.unwrap(),
        transforms: Arc::from([Placement3::IDENTITY]),
        seeds: Some(Arc::from([])),
        tints: None,
        extras: GltfExtras::new(),
    });
    assert!(matches!(
        export_glb(&scene, &compiled),
        Err(GltfError::InvalidInput {
            reason: "placement count mismatch",
            ..
        })
    ));
}
#[test]
fn different_lod_thresholds_do_not_share_an_instance_batch() {
    let source = geometry(1.0);
    let compiled = PartCompiler::new()
        .compile_parts(
            &[
                PartInput::new("fine", &source),
                PartInput::new("coarse", &source),
            ],
            &CompilePolicy::default(),
        )
        .unwrap();
    let nodes = [0.5, 0.7].map(|coverage| {
        let mut node = drawable("lod occurrence", &compiled);
        let geometry = node.geometry.as_mut().unwrap();
        geometry.lods = vec![
            GltfLod {
                mesh: geometry.mesh.clone(),
                min_coverage: coverage,
                crossfade: 0.0,
            },
            GltfLod {
                mesh: GltfMesh::new(PartId(1), "coarse", &compiled).unwrap(),
                min_coverage: 0.0,
                crossfade: 0.0,
            },
        ];
        node
    });
    let scene = GltfScene {
        nodes: nodes.into(),
        ..GltfScene::default()
    };
    let options = GltfExportOptions::default()
        .with_instancing(GltfInstancing::GpuInstancing)
        .with_lods(GltfLods::MsftLod);
    let export = export_glb_with_options(&scene, &compiled, options).unwrap();
    assert_eq!(export.stats.batched_instances, 0);
    assert_eq!(export.stats.lod_nodes, 2);
}
#[test]
fn finite_transforms_that_overflow_float_accessors_keep_matrix_nodes() {
    let compiled = compiled();
    let mut distant = drawable("distant", &compiled);
    distant.transform = Placement3::translate(f64::from(f32::MAX) * 2.0, 0.0, 0.0);
    let scene = GltfScene {
        nodes: vec![distant, drawable("a", &compiled), drawable("b", &compiled)],
        ..GltfScene::default()
    };
    let export = export_glb_with_options(
        &scene,
        &compiled,
        GltfExportOptions::default().with_instancing(GltfInstancing::GpuInstancing),
    )
    .unwrap();
    assert_eq!(export.stats.unbatched_unrepresentable_instances, 1);
    assert_eq!(export.stats.batched_instances, 2);
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert!(
        document.json()["nodes"][0]["matrix"][12]
            .as_f64()
            .unwrap()
            .is_finite()
    );
}

#[test]
fn instance_batches_retain_caller_names_and_metadata() {
    let compiled = compiled();
    let mut first = drawable("application:left", &compiled);
    first.extras.insert("component".into(), json!(42));
    first
        .extras
        .insert("name".into(), json!("metadata collision"));
    let scene = GltfScene {
        nodes: vec![first, drawable("application:right", &compiled)],
        ..GltfScene::default()
    };
    let export = export_glb_with_options(
        &scene,
        &compiled,
        GltfExportOptions::default().with_instancing(GltfInstancing::GpuInstancing),
    )
    .unwrap();
    let document = GlbDocument::parse(&export.bytes).unwrap();
    let identities = &document.json()["nodes"][0]["extras"]["instances"];
    assert_eq!(identities[0]["name"], "application:left");
    assert_eq!(identities[0]["component"], 42);
    assert_eq!(identities[1]["name"], "application:right");
}

#[test]
fn positive_scales_that_underflow_float_accessors_keep_matrix_nodes() {
    let compiled = compiled();
    let mut tiny = drawable("tiny", &compiled);
    tiny.transform.rows[0][0] = 1e-46;
    let scene = GltfScene {
        nodes: vec![tiny, drawable("a", &compiled), drawable("b", &compiled)],
        ..GltfScene::default()
    };
    let export = export_glb_with_options(
        &scene,
        &compiled,
        GltfExportOptions::default().with_instancing(GltfInstancing::GpuInstancing),
    )
    .unwrap();
    assert_eq!(export.stats.unbatched_unrepresentable_instances, 1);
    assert_eq!(export.stats.batched_instances, 2);
    let document = GlbDocument::parse(&export.bytes).unwrap();
    assert_eq!(
        document.json()["nodes"][0]["matrix"][0].as_f64(),
        Some(1e-46)
    );
}

/// A recipe whose refused union leaves the compiled input partial.
fn partial_geometry() -> PartSource {
    use exedra_constructive::ir::{CsgOp, NodeKind, PrimitiveSpec, RecipeBuilder};
    let mut builder = RecipeBuilder::new();
    let boxes = [Placement3::IDENTITY, Placement3::translate(1.0, 1.0, 0.0)].map(|placement| {
        builder
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size: [1.0; 3] },
                placement,
            })
            .unwrap()
    });
    let refused = builder
        .add(NodeKind::Csg {
            op: CsgOp::Union,
            operands: boxes.into(),
        })
        .unwrap();
    let root = builder
        .add(NodeKind::Group {
            children: vec![boxes[0], refused],
        })
        .unwrap();
    PartSource::Recipe(builder.finish(root).unwrap())
}

#[test]
fn only_exported_geometry_must_be_complete() {
    let (complete, partial) = (geometry(1.0), partial_geometry());
    let compiled = PartCompiler::new()
        .compile_parts(
            &[
                PartInput::new("complete", &complete),
                PartInput::new("partial", &partial),
            ],
            &CompilePolicy::default(),
        )
        .unwrap();
    assert!(
        !compiled
            .report(PartId(1))
            .unwrap()
            .clean_at(Severity::Error)
    );
    let partial_mesh = GltfMesh::new(PartId(1), "partial", &compiled).unwrap();
    let refused = |scene: &GltfScene, options| {
        matches!(
            export_glb_with_options(scene, &compiled, options),
            Err(GltfError::IncompleteGeometry { part: 1, .. })
        )
    };
    let defaults = GltfExportOptions::default();
    let with_lods = GltfExportOptions::default().with_lods(GltfLods::MsftLod);

    let mut scene = GltfScene {
        nodes: vec![drawable("complete", &compiled)],
        ..GltfScene::default()
    };
    scene.placement_sets.push(GltfPlacementSet {
        name: "unplaced".into(),
        parent: None,
        geometry: partial_mesh.clone().into(),
        transforms: Arc::from([]),
        seeds: None,
        tints: None,
        extras: GltfExtras::new(),
    });
    export_glb_with_options(&scene, &compiled, defaults).unwrap();

    let geometry = scene.nodes[0].geometry.as_mut().unwrap();
    geometry.lods = vec![
        GltfLod {
            mesh: geometry.mesh.clone(),
            min_coverage: 0.5,
            crossfade: 0.0,
        },
        GltfLod {
            mesh: partial_mesh.clone(),
            min_coverage: 0.0,
            crossfade: 0.0,
        },
    ];
    export_glb_with_options(&scene, &compiled, defaults).unwrap();
    assert!(refused(&scene, with_lods), "an enabled level is exported");

    scene.nodes[0].geometry.as_mut().unwrap().lods.clear();
    scene.placement_sets[0].transforms = Arc::from([Placement3::IDENTITY]);
    assert!(refused(&scene, defaults), "a placed set is exported");

    scene.placement_sets.clear();
    let mut node = GltfNode::new("partial");
    node.geometry = Some(partial_mesh.into());
    scene.nodes.push(node);
    assert!(refused(&scene, defaults), "a drawable node is exported");
}
