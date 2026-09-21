// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;
use exedra_mesh::{BuildParams, Mesh};

use crate::cache::EvalCache;
use crate::evaluate::{Severity, evaluate, evaluate_with_cache, mesh_bounds};
use crate::ir::{NodeId, NodeKind, Placement3, Plane3, Recipe, RecipeBuilder, VertexStretchStep};
use crate::tessellate::EvalPolicy;
use crate::text::{dump_recipe, parse_recipe};

fn step(distance: f64, length: f64) -> VertexStretchStep {
    VertexStretchStep {
        plane: Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance,
        },
        length,
    }
}
fn triangle() -> Mesh {
    Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .unwrap()
}
fn import(builder: &mut RecipeBuilder, mesh: Mesh) -> NodeId {
    let import = builder.add_import(mesh).unwrap();
    builder
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap()
}
fn recipe(mesh: Mesh, steps: Vec<VertexStretchStep>) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let child = import(&mut builder, mesh);
    let root = builder
        .add(NodeKind::StretchVertices { child, steps })
        .unwrap();
    builder.finish(root).unwrap()
}

#[test]
fn cache_roundtrip_preserves_geometry_provenance_and_occurrence_materials() {
    let mut builder = RecipeBuilder::new();
    let child = import(&mut builder, triangle());
    let stretch = builder
        .add(NodeKind::StretchVertices {
            child,
            steps: vec![step(1.0, 1.0)],
        })
        .unwrap();
    let mut instances = Vec::new();
    let slots = [builder.material_slot("red"), builder.material_slot("blue")];
    for slot in slots {
        instances.push(
            builder
                .with_material(slot)
                .add(NodeKind::Instance {
                    of: stretch,
                    placement: Placement3::IDENTITY,
                })
                .unwrap(),
        );
    }
    let root = builder
        .add(NodeKind::Group {
            children: instances,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let policy = EvalPolicy::default();
    let pure = evaluate(&recipe, &policy).unwrap();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(pure.report.counters.vertex_stretch_passes, 2);
    assert_eq!(cold.report.counters.vertex_stretch_passes, 1);
    assert_eq!(warm.report.counters.vertex_stretch_passes, 0);
    for result in [&pure, &cold, &warm] {
        assert!(result.report.clean_at(Severity::Error));
        assert_eq!(result.bodies.len(), 2);
        for (index, placed) in result.bodies.iter().enumerate() {
            assert_eq!(placed.material, Some(slots[index]));
            assert_eq!(mesh_bounds(&placed.body.mesh).max, [3.0, 1.0, 1.0]);
            assert_eq!(placed.body.mesh.faces().count(), 1);
            placed.body.source_map.check(&placed.body.mesh).unwrap();
            for face in placed.body.mesh.faces() {
                assert!(placed.body.source_map.face_feature(face).is_some());
            }
            assert_eq!(
                exedra_testkit::dump_mesh_topology(&placed.body.mesh),
                exedra_testkit::dump_mesh_topology(&pure.bodies[index].body.mesh)
            );
        }
    }
    // Complete node content participates in the cache key and survives text.
    let text = dump_recipe(&recipe);
    assert_eq!(dump_recipe(&parse_recipe(&text).unwrap()), text);
}

#[test]
fn steps_have_one_input_frame_and_transform_with_ancestors() {
    let source = Mesh::from_indexed_triangles(
        &[[0.45, 0.0, 0.0], [0.45, 1.0, 0.0], [0.45, 0.0, 1.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .unwrap();
    let mut builder = RecipeBuilder::new();
    let child = import(&mut builder, source);
    let stretch = builder
        .add(NodeKind::StretchVertices {
            child,
            steps: vec![step(0.4, -0.3), step(0.2, -0.3)],
        })
        .unwrap();
    let xf = Placement3::from_axes(
        [0.0, -2.0, 0.0],
        [-3.0, 0.0, 0.0],
        [0.5, 0.0, 4.0],
        [7.0, 9.0, 11.0],
    );
    let root = builder
        .add(NodeKind::Transform { child: stretch, xf })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    assert!(result.report.clean_at(Severity::Error));
    assert_eq!(result.bodies.len(), 1);
    for v in result.bodies[0].body.mesh.vertices() {
        assert!((result.bodies[0].body.mesh.vertex_position(v).unwrap()[1] - 9.3).abs() < 1e-6);
    }
}

#[test]
fn group_refusal_is_atomic_including_a_child_that_already_refused() {
    for refuse_in_child in [false, true] {
        let mut builder = RecipeBuilder::new();
        let good = import(&mut builder, triangle());
        let quad = Mesh::from_polygons(
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[&[0, 1, 2, 3]],
        )
        .unwrap();
        let bad = import(&mut builder, quad);
        let bad = if refuse_in_child {
            builder
                .add(NodeKind::StretchVertices {
                    child: bad,
                    steps: vec![step(1.0, 1.0)],
                })
                .unwrap()
        } else {
            bad
        };
        let child = builder
            .add(NodeKind::Group {
                children: vec![good, bad],
            })
            .unwrap();
        let root = builder
            .add(NodeKind::StretchVertices {
                child,
                steps: vec![step(1.0, 1.0)],
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        let mut cache = EvalCache::new();
        for _ in 0..2 {
            let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
            assert!(result.bodies.is_empty());
            assert!(!result.report.clean_at(Severity::Error));
            let expected = if refuse_in_child {
                "eval.stretch_vertices.incomplete_child"
            } else {
                "eval.stretch_vertices.unsupported_face"
            };
            assert!(result.report.diagnostics.iter().any(|d| d.code == expected));
        }
    }
}

#[test]
fn zero_bypasses_polygon_restriction_and_active_collapse_is_typed() {
    let quad = exedra_testkit::quad_mesh();
    let result = evaluate(&recipe(quad, vec![step(0.5, 0.0)]), &EvalPolicy::default()).unwrap();
    assert!(result.report.clean_at(Severity::Error));
    assert_eq!(result.report.counters.vertex_stretch_passes, 0);
    let face = result.bodies[0].body.mesh.faces().next().unwrap();
    assert_eq!(result.bodies[0].body.mesh.face_loop(face).count(), 4);
    let mesh = Mesh::from_indexed_triangles(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
        &BuildParams::default(),
    )
    .unwrap();
    let result = evaluate(&recipe(mesh, vec![step(0.5, -1.0)]), &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.stretch_vertices.degenerate_triangle")
    );
}

#[test]
fn step_content_and_order_affect_fingerprints_and_invalid_steps_are_rejected() {
    let variants = [
        vec![step(1.0, 1.0)],
        vec![step(1.0, 2.0)],
        vec![step(0.5, 1.0)],
        vec![step(1.0, 1.0), step(0.5, -0.25)],
        vec![step(0.5, -0.25), step(1.0, 1.0)],
    ];
    let mut hashes = Vec::new();
    for steps in variants {
        let recipe = recipe(triangle(), steps);
        let parsed = parse_recipe(&dump_recipe(&recipe)).unwrap();
        let hash = recipe.fingerprint(recipe.root()).unwrap();
        assert_eq!(Some(hash), parsed.fingerprint(parsed.root()));
        assert!(!hashes.contains(&hash));
        hashes.push(hash);
    }
    let mut builder = RecipeBuilder::new();
    let child = import(&mut builder, triangle());
    for steps in [
        vec![],
        vec![step(0.0, f64::NAN)],
        vec![VertexStretchStep {
            plane: Plane3 {
                normal: [0.0; 3],
                distance: 0.0,
            },
            length: 0.0,
        }],
    ] {
        assert!(
            builder
                .add(NodeKind::StretchVertices { child, steps })
                .is_err()
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn json_preserves_steps_and_rejects_invalid_content() {
    use crate::interchange::{RecipeDto, from_dto, to_dto};
    let recipe = recipe(triangle(), vec![step(1.0, 1.0), step(0.5, -0.25)]);
    let json = serde_json::to_string(&to_dto(&recipe)).unwrap();
    let decoded: RecipeDto = serde_json::from_str(&json).unwrap();
    let parsed = from_dto(&decoded).unwrap();
    assert_eq!(recipe.recipe_fingerprint(), parsed.recipe_fingerprint());
    let mut invalid = decoded;
    let crate::interchange::NodeKindDto::StretchVertices { steps, .. } =
        &mut invalid.nodes.last_mut().unwrap().kind
    else {
        panic!("stretch root")
    };
    steps.clear();
    assert!(from_dto(&invalid).is_err());
}

#[test]
fn body_adapter_retains_face_ownership_and_rejects_stale_correspondence() {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("panel");
    let mesh = builder.add_import(triangle()).unwrap();
    let root = builder
        .with_source(source)
        .add(NodeKind::MeshImport {
            import: mesh,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let result = evaluate(&recipe, &EvalPolicy::default()).unwrap();
    let mut body = (*result.bodies[0].body).clone();
    let face = body.mesh.faces().next().unwrap();
    body.face_materials.insert(face, crate::ir::SlotId(7));
    body.refinement = Some(exedra_triangulate::RefineStats::default());
    let deformed =
        super::stretch_body_vertices(&body, &[step(1.0, 1.0)], &Placement3::IDENTITY).unwrap();
    assert!(deformed.refinement.is_none());
    assert_eq!(deformed.face_materials, body.face_materials);
    assert_eq!(deformed.source_map.dump(), body.source_map.dump());
    assert_eq!(
        deformed.source_map.surface_origin(face),
        body.source_map.surface_origin(face)
    );
    deformed.source_map.check(&deformed.mesh).unwrap();
    let vertex = body.mesh.vertices().next().unwrap();
    let mut edit = body.mesh.edit();
    exedra_mesh::op::set_vertex_position(&mut edit, vertex, [0.0, 0.0, 2.0]).unwrap();
    let _: () = edit.finish();
    assert_eq!(
        super::stretch_body_vertices(&body, &[step(1.0, 1.0)], &Placement3::IDENTITY).unwrap_err(),
        exedra_mesh_ops::stretch::VertexStretchError::InvalidMesh
    );
}
