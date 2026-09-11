// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::builders;
use crate::cache::EvalCache;
use crate::evaluate::{Severity, evaluate, evaluate_with_cache};
use crate::ir::{
    CapMode, CsgOp, NodeId, NodeKind, Placement3, PrimitiveSpec, Recipe, RecipeBuilder, SlotId,
};
use crate::tessellate::{EvalPolicy, tessellate_extrude, tessellate_primitive};
use crate::text;
use alloc::{format, rc::Rc, vec};
use exedra_mesh::op::{set_corner_uv, set_face_region};

fn box_node(b: &mut RecipeBuilder, size: [f64; 3]) -> NodeId {
    b.add(NodeKind::Primitive {
        spec: PrimitiveSpec::Box { size },
        placement: Placement3::IDENTITY,
    })
    .unwrap()
}

fn rail_recipe(selection: EdgeSelection, policy: RoundPolicy) -> Recipe {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let root = b
        .add(NodeKind::EdgeFinish {
            child,
            selection,
            policy,
        })
        .unwrap();
    b.finish(root).unwrap()
}

fn operand_region(operand: u16, region: u32) -> OperandRegion {
    OperandRegion { operand, region }
}

fn mesh_positions(body: &TessellatedBody) -> Vec<[u32; 3]> {
    let mut positions: Vec<_> = body
        .mesh
        .vertices()
        .map(|v| body.mesh.vertex_position(v).unwrap().map(f32::to_bits))
        .collect();
    positions.sort_unstable();
    positions
}

/// Rotate the box cutter so its region 5 is a pocket wall, meeting the
/// panel's region 5 top face. Equal labels must still identify distinct sides.
fn equal_region_recess() -> Recipe {
    let mut b = RecipeBuilder::new();
    let panel = box_node(&mut b, [1.0, 1.0, 0.1]);
    let cutter = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [0.8, 0.04, 0.8],
            },
            placement: Placement3 {
                rows: [
                    [1.0, 0.0, 0.0, 0.1],
                    [0.0, 0.0, -1.0, 0.9],
                    [0.0, 1.0, 0.0, 0.08],
                ],
            },
        })
        .unwrap();
    let root = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![panel, cutter],
        })
        .unwrap();
    b.finish(root).unwrap()
}

fn two_pockets(segments: u32, grouped: bool, nested: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let panel = box_node(&mut b, [4.0, 2.0, 1.0]);
    let outside = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box { size: [1.0; 3] },
            placement: Placement3::translate(10.0, 0.0, 0.0),
        })
        .unwrap();
    let mut cutters = Vec::new();
    for x in [1.0, 3.0] {
        cutters.push(
            b.add(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 0.25,
                    height: 0.4,
                    segments,
                },
                placement: Placement3::translate(x, 1.0, 0.8),
            })
            .unwrap(),
        );
    }
    if grouped {
        cutters = vec![b.add(NodeKind::Group { children: cutters }).unwrap()];
    }
    // Operand 1 is excluded by the Boolean's bounds test. The live cutters
    // must retain declared indices 2 and 3, rather than being renumbered.
    let mut operands = vec![panel, outside];
    operands.extend(cutters);
    let mut root = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands,
        })
        .unwrap();
    if nested {
        root = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![root, outside],
            })
            .unwrap();
    }
    b.finish(root).unwrap()
}

#[test]
fn qualified_boundaries_follow_declared_operands_and_refuse_disconnected_matches() {
    let policy = RoundPolicy::chamfer(0.01);
    let first = [operand_region(0, 5), operand_region(2, 1)];
    let second = [operand_region(0, 5), operand_region(3, 1)];
    for grouped in [false, true] {
        let evaluated = evaluate(&two_pockets(16, grouped, false), &EvalPolicy::default()).unwrap();
        assert_eq!(
            evaluated.bodies.len(),
            1,
            "{:?}",
            evaluated.report.diagnostics
        );
        let body = &evaluated.bodies[0].body;
        let before = format!("{body:?}");
        let selection = EdgeSelection::OperandBoundaries(vec![first, second]);
        if grouped {
            assert!(matches!(finish_edges(body, &selection, &policy),
                Err(EdgeFinishError::AmbiguousOperandSelection { regions }) if regions == first));
        } else {
            let selected = targets(body, &selection, &policy).unwrap();
            // Boolean triangle cuts may subdivide a source polygon edge.
            assert!(selected.len() >= 32);
            for edge in selected {
                for vertex in [
                    body.mesh.from_vertex(edge).unwrap(),
                    body.mesh.to_vertex(edge).unwrap(),
                ] {
                    let p = body.mesh.vertex_position(vertex).unwrap();
                    let x = if p[0] < 2.0 { 1.0 } else { 3.0 };
                    assert!((p[2] - 1.0).abs() < 1e-6);
                    let distance = (p[0] - x).hypot(p[1] - 1.0);
                    let apothem = 0.25 * (core::f32::consts::PI / 16.0).cos();
                    assert!((apothem - 1e-6..=0.25 + 1e-6).contains(&distance));
                }
            }
        }
        assert_eq!(format!("{body:?}"), before);
    }
    let nested = evaluate(&two_pockets(16, false, true), &EvalPolicy::default()).unwrap();
    let body = &nested.bodies[0].body;
    assert!(body.mesh.faces().all(|face| {
        body.source_map.face_feature(face) == Some(Feature::BooleanFace { operand: 0 })
    }));
    assert!(matches!(
        finish_edges(
            body,
            &EdgeSelection::OperandBoundaries(vec![first]),
            &policy
        ),
        Err(EdgeFinishError::EmptySelection)
    ));
}

#[test]
fn qualified_boundaries_survive_compaction_and_tessellation_changes() {
    let policy = RoundPolicy::chamfer(0.01);
    for segments in [16, 32] {
        // A disjoint cutter leaves the cylinder intact while giving its faces
        // Boolean provenance. Both regions of each rim belong to operand 0.
        let mut b = RecipeBuilder::new();
        let cylinder = b
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Cylinder {
                    radius: 0.25,
                    height: 0.5,
                    segments,
                },
                placement: Placement3::IDENTITY,
            })
            .unwrap();
        let outside = b
            .add(NodeKind::Primitive {
                spec: PrimitiveSpec::Box { size: [1.0; 3] },
                placement: Placement3::translate(10.0, 0.0, 0.0),
            })
            .unwrap();
        let root = b
            .add(NodeKind::Csg {
                op: CsgOp::Difference,
                operands: vec![cylinder, outside],
            })
            .unwrap();
        let evaluated = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
        let first =
            EdgeSelection::OperandBoundaries(vec![[operand_region(0, 1), operand_region(0, 2)]]);
        let (finished, _) = finish_edges(&evaluated.bodies[0].body, &first, &policy).unwrap();
        let (mesh, remap) = finished.mesh.compact();
        let faces: BTreeMap<_, _> = finished
            .mesh
            .faces()
            .map(|face| {
                (
                    remap.face(face).unwrap(),
                    finished.source_map.face_feature(face).unwrap(),
                )
            })
            .collect();
        let vertices: BTreeMap<_, _> = finished
            .mesh
            .vertices()
            .map(|vertex| {
                (
                    remap.vertex(vertex).unwrap(),
                    finished.source_map.vertex_feature(vertex).unwrap(),
                )
            })
            .collect();
        let compacted = TessellatedBody {
            source_map: SourceMap::new(
                &mesh,
                mesh.faces().map(|f| faces[&f]).collect(),
                mesh.vertices().map(|v| vertices[&v]).collect(),
            ),
            mesh,
            face_materials: BTreeMap::new(),
            refinement: None,
        };
        let second =
            EdgeSelection::OperandBoundaries(vec![[operand_region(0, 1), operand_region(0, 3)]]);
        let selected_segments = |body: &TessellatedBody| {
            let edges = targets(body, &second, &policy).unwrap();
            assert!(edges.len() >= usize::try_from(segments).unwrap());
            let mut positions: Vec<_> = edges
                .into_iter()
                .map(|edge| {
                    let mut pair = [
                        body.mesh.from_vertex(edge).unwrap(),
                        body.mesh.to_vertex(edge).unwrap(),
                    ]
                    .map(|v| body.mesh.vertex_position(v).unwrap().map(f32::to_bits));
                    pair.sort_unstable();
                    pair
                })
                .collect();
            positions.sort_unstable();
            positions
        };
        assert_eq!(selected_segments(&finished), selected_segments(&compacted));
        let (a, stats_a) = finish_edges(&finished, &second, &policy).unwrap();
        let (b, stats_b) = finish_edges(&compacted, &second, &policy).unwrap();
        assert_eq!(stats_a, stats_b);
        assert_eq!(mesh_positions(&a), mesh_positions(&b));
        assert!(a.mesh.validate_deep().is_empty());
        assert!(b.mesh.validate_deep().is_empty());
    }
}

#[test]
fn qualified_rims_disambiguate_reused_and_equal_region_labels() {
    let evaluated = evaluate(&equal_region_recess(), &EvalPolicy::default()).unwrap();
    let body = &evaluated.bodies[0].body;
    let before = format!("{body:?}");
    let mut policy = RoundPolicy::chamfer(0.005);
    policy.max_tangent_turn = core::f64::consts::FRAC_PI_2;
    assert!(matches!(
        targets(
            body,
            &EdgeSelection::RegionBoundaries(vec![[1, 5]]),
            &policy
        ),
        Err(EdgeFinishError::AmbiguousSelection { .. })
    ));
    let equal =
        EdgeSelection::OperandBoundaries(vec![[operand_region(0, 5), operand_region(1, 5)]]);
    let edges = targets(body, &equal, &policy).unwrap();
    assert!(!edges.is_empty());
    for edge in edges {
        for vertex in [
            body.mesh.from_vertex(edge).unwrap(),
            body.mesh.to_vertex(edge).unwrap(),
        ] {
            let p = body.mesh.vertex_position(vertex).unwrap();
            assert!((p[1] - 0.1).abs() < 1e-6 && (p[2] - 0.1).abs() < 1e-6);
        }
    }

    let walls = [1, 2, 5, 6];
    let selection = EdgeSelection::OperandBoundaries(
        walls
            .map(|region| [operand_region(0, 5), operand_region(1, region)])
            .to_vec(),
    );
    // Compare to the previous caller workaround on exactly the same input
    // geometry: give operand 1 regions an artificial +100 namespace.
    let mut renamed_mesh = body.mesh.clone();
    let regions: Vec<_> = body
        .mesh
        .faces()
        .map(|face| {
            let Feature::BooleanFace { operand } = body.source_map.face_feature(face).unwrap()
            else {
                panic!("Boolean face");
            };
            let region = *body
                .mesh
                .attrs()
                .dense(attr::FACE_REGION)
                .unwrap()
                .get(face.as_id())
                .unwrap();
            (face, region + u32::from(operand) * 100)
        })
        .collect();
    let mut edit = renamed_mesh.edit();
    for (face, region) in regions {
        set_face_region(&mut edit, face, region).unwrap();
    }
    let _: () = edit.finish();
    let renamed = TessellatedBody {
        source_map: body.source_map.repinned(&renamed_mesh),
        mesh: renamed_mesh,
        face_materials: body.face_materials.clone(),
        refinement: None,
    };
    let unqualified =
        EdgeSelection::RegionBoundaries(walls.map(|region| [5, region + 100]).to_vec());
    for kind in [
        RoundKind::Chamfer { setback: 0.005 },
        RoundKind::Fillet { radius: 0.005 },
    ] {
        policy.kind = kind;
        let (actual, stats) = finish_edges(body, &selection, &policy).unwrap();
        let (reference, reference_stats) = finish_edges(&renamed, &unqualified, &policy).unwrap();
        assert_eq!(stats.closed_chains, 1);
        assert_eq!(stats, reference_stats);
        assert_eq!(mesh_positions(&actual), mesh_positions(&reference));
        assert!(actual.mesh.validate_deep().is_empty());
    }
    assert_eq!(format!("{body:?}"), before);
}

#[test]
fn qualified_selection_is_canonical_and_qualifiers_affect_fingerprints() {
    let pairs = [
        [operand_region(0, 5), operand_region(1, 1)],
        [operand_region(0, 5), operand_region(2, 1)],
    ];
    let policy = RoundPolicy::chamfer(0.005);
    let recipe = rail_recipe(EdgeSelection::OperandBoundaries(pairs.to_vec()), policy);
    let permuted = rail_recipe(
        EdgeSelection::OperandBoundaries(vec![
            [pairs[1][1], pairs[1][0]],
            pairs[0],
            pairs[1],
            [pairs[0][1], pairs[0][0]],
        ]),
        policy,
    );
    assert_eq!(recipe.recipe_fingerprint(), permuted.recipe_fingerprint());
    assert_eq!(text::dump_recipe(&recipe), text::dump_recipe(&permuted));
    for replacement in [
        operand_region(3, 1),
        operand_region(1, 2),
        operand_region(u16::MAX, u32::MAX),
    ] {
        let mut changed = pairs;
        changed[0][1] = replacement;
        let variant = rail_recipe(EdgeSelection::OperandBoundaries(changed.to_vec()), policy);
        assert_ne!(recipe.recipe_fingerprint(), variant.recipe_fingerprint());
        let restored = text::parse_recipe(&text::dump_recipe(&variant)).unwrap();
        assert_eq!(variant.recipe_fingerprint(), restored.recipe_fingerprint());
    }
    let dump = text::dump_recipe(&recipe);
    assert_eq!(
        text::parse_recipe(&dump).unwrap().recipe_fingerprint(),
        recipe.recipe_fingerprint()
    );
    assert!(
        text::parse_recipe(
            &dump.replace("operand_boundaries 2 0 5", "operand_boundaries 2 65536 5")
        )
        .is_err()
    );
    assert_ne!(
        recipe.recipe_fingerprint(),
        rail_recipe(EdgeSelection::RegionBoundaries(vec![[5, 1]]), policy,).recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        use crate::interchange::{from_dto, to_dto};
        let json = serde_json::to_string(&to_dto(&recipe)).unwrap();
        let restored = from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored.recipe_fingerprint(), recipe.recipe_fingerprint());
        assert_eq!(json, serde_json::to_string(&to_dto(&permuted)).unwrap());
    }
}

#[test]
fn invalid_or_missing_qualified_pairs_refuse_the_entire_selection() {
    let evaluated = evaluate(&equal_region_recess(), &EvalPolicy::default()).unwrap();
    let body = &evaluated.bodies[0].body;
    let before = format!("{body:?}");
    let good = [operand_region(0, 5), operand_region(1, 5)];
    let policy = RoundPolicy::chamfer(0.005);
    for pairs in [vec![], vec![[good[0], good[0]]]] {
        let selection = EdgeSelection::OperandBoundaries(pairs);
        assert!(!selection.valid());
        assert!(matches!(
            finish_edges(body, &selection, &policy),
            Err(EdgeFinishError::InvalidSelection)
        ));
    }
    for missing in [operand_region(2, 5), operand_region(1, 99)] {
        let selection = EdgeSelection::OperandBoundaries(vec![good, [good[0], missing]]);
        assert!(matches!(
            finish_edges(body, &selection, &policy),
            Err(EdgeFinishError::EmptySelection)
        ));
    }
    let primitive = tessellate_primitive(
        PrimitiveSpec::Box { size: [1.0; 3] },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(matches!(
        finish_edges(
            &primitive,
            &EdgeSelection::OperandBoundaries(vec![good]),
            &policy
        ),
        Err(EdgeFinishError::EmptySelection)
    ));
    assert_eq!(format!("{body:?}"), before);
}

#[test]
fn finished_geometry_and_uvs_replay_from_cache_without_capturing_defaults() {
    let mut b = RecipeBuilder::new();
    let red = b.material_slot("red");
    let blue = b.material_slot("blue");
    let mut mesh = tessellate_primitive(
        PrimitiveSpec::Box {
            size: [0.09, 0.2, 2.0],
        },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )
    .unwrap()
    .mesh;
    let values: Vec<_> = mesh
        .faces()
        .flat_map(|f| mesh.face_loop(f))
        .map(|corner| {
            let p = mesh
                .vertex_position(mesh.to_vertex(corner).unwrap())
                .unwrap();
            (corner, [p[0] + p[1] + 0.25, p[1] + p[2] + 0.75])
        })
        .collect();
    let mut edit = mesh.edit();
    for (corner, uv) in values {
        set_corner_uv(&mut edit, corner, uv).unwrap();
    }
    let _: () = edit.finish();
    let import = b.add_import(mesh).unwrap();
    let child = b
        .add(NodeKind::MeshImport {
            import,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    let finish = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let a = b
        .with_material(red)
        .add(NodeKind::Transform {
            child: finish,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let c = b
        .with_material(blue)
        .add(NodeKind::Transform {
            child: finish,
            xf: Placement3::IDENTITY,
        })
        .unwrap();
    let root = b
        .add(NodeKind::Group {
            children: vec![a, c],
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let policy = EvalPolicy::default();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(cold.bodies.len(), 2);
    assert_eq!(cold.report.counters.edge_finish_passes, 1);
    assert_eq!(warm.report.counters.edge_finish_passes, 0);
    assert!(Rc::ptr_eq(&cold.bodies[0].body, &cold.bodies[1].body));
    assert!(Rc::ptr_eq(&cold.bodies[0].body, &warm.bodies[0].body));
    assert_eq!(cold.report.diagnostics, warm.report.diagnostics);
    assert_eq!(
        cold.report
            .diagnostics
            .iter()
            .filter(|d| d.code == "eval.edge_finish.uv_deferred")
            .count(),
        0
    );
    for result in [&cold, &warm] {
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .all(|d| d.severity != Severity::Error)
        );
        for (placed, slot) in result.bodies.iter().zip([red, blue]) {
            let mesh = &placed.body.mesh;
            for corner in mesh.faces().flat_map(|f| mesh.face_loop(f)) {
                let uv = mesh
                    .attrs()
                    .sparse(attr::CORNER_UV)
                    .unwrap()
                    .get(corner.as_id())
                    .unwrap();
                assert!(uv.iter().all(|v| v.is_finite()));
            }
            assert!(
                placed
                    .body
                    .mesh
                    .faces()
                    .all(|f| placed.material_for_face(f) == Some(slot))
            );
        }
    }
    let pure = evaluate(&recipe, &policy).unwrap();
    assert_eq!(
        format!("{:?}", cold.bodies[0].body.mesh),
        format!("{:?}", pure.bodies[0].body.mesh)
    );
}

#[test]
fn face_slots_regions_and_features_survive_selected_finishing() {
    let mut body = tessellate_primitive(
        PrimitiveSpec::Box {
            size: [0.09, 0.2, 0.1],
        },
        &Placement3::IDENTITY,
        &EvalPolicy::default(),
    )
    .unwrap();
    for face in body.mesh.faces() {
        let Feature::PrimitiveRegion { region } = body.source_map.face_feature(face).unwrap()
        else {
            panic!("primitive")
        };
        body.face_materials.insert(face, SlotId(region));
    }
    let selection = EdgeSelection::RegionBoundaries(vec![[1, 3]]);
    let mut policy = RoundPolicy::chamfer(0.005);
    policy.region = Some(77);
    let before = format!("{:?}", body.mesh);
    let (finished, stats) = finish_edges(&body, &selection, &policy).unwrap();
    assert_eq!(stats.chains, 1);
    assert_eq!(format!("{:?}", body.mesh), before);
    finished.source_map.check(&finished.mesh).unwrap();
    let mut band = 0;
    for face in finished.mesh.faces() {
        let Feature::PrimitiveRegion { region } = finished.source_map.face_feature(face).unwrap()
        else {
            panic!("lost source feature")
        };
        assert_eq!(finished.face_materials[&face], SlotId(region));
        let output_region = finished
            .mesh
            .attrs()
            .dense(attr::FACE_REGION)
            .unwrap()
            .get(face.as_id())
            .copied()
            .unwrap();
        if output_region == 77 {
            band += 1;
            assert_eq!(region, 1);
        } else {
            assert_eq!(output_region, region);
        }
    }
    assert_eq!(band, 1);
}

#[test]
fn finishing_a_boolean_preserves_cut_slots_and_per_occurrence_defaults() {
    let mut b = RecipeBuilder::new();
    let cut = b.material_slot("cut");
    let red = b.material_slot("red");
    let blue = b.material_slot("blue");
    let block = box_node(&mut b, [1.0, 1.0, 1.0]);
    let cutter = b
        .with_material(cut)
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Cylinder {
                radius: 0.15,
                height: 0.4,
                segments: 16,
            },
            placement: Placement3::translate(0.7, 0.7, 0.8),
        })
        .unwrap();
    let difference = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![block, cutter],
        })
        .unwrap();
    let finish = b
        .add(NodeKind::EdgeFinish {
            child: difference,
            selection: EdgeSelection::OperandBoundaries(vec![[
                OperandRegion {
                    operand: 0,
                    region: 5,
                },
                OperandRegion {
                    operand: 1,
                    region: 1,
                },
            ]]),
            policy: RoundPolicy::chamfer(0.05),
        })
        .unwrap();
    let mut children = Vec::new();
    for slot in [red, blue] {
        children.push(
            b.with_material(slot)
                .add(NodeKind::Transform {
                    child: finish,
                    xf: Placement3::IDENTITY,
                })
                .unwrap(),
        );
    }
    let root = b.add(NodeKind::Group { children }).unwrap();
    let recipe = b.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for pass in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert_eq!(result.bodies.len(), 2, "{:?}", result.report.diagnostics);
        assert_eq!(
            result.report.counters.edge_finish_passes,
            u32::from(pass == 0)
        );
        assert!(Rc::ptr_eq(&result.bodies[0].body, &result.bodies[1].body));
        for (placed, default) in result.bodies.iter().zip([red, blue]) {
            let mut cuts = 0;
            let mut surfaces = 0;
            for face in placed.body.mesh.faces() {
                match placed.body.source_map.face_feature(face).unwrap() {
                    Feature::BooleanFace { operand: 0 } => {
                        assert_eq!(placed.material_for_face(face), Some(default));
                        surfaces += 1;
                    }
                    Feature::BooleanFace { operand: 1 } => {
                        assert_eq!(placed.material_for_face(face), Some(cut));
                        cuts += 1;
                    }
                    other => panic!("lost Boolean ownership: {other:?}"),
                }
            }
            assert!(cuts > 0 && surfaces > 0);
        }
    }
}

#[test]
fn outer_reflection_and_scale_apply_after_local_finishing() {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let finish = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let xf = Placement3 {
        rows: [
            [-2.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 3.0],
        ],
    };
    let placed = b.add(NodeKind::Transform { child: finish, xf }).unwrap();
    let root = b
        .add(NodeKind::Group {
            children: vec![finish, placed],
        })
        .unwrap();
    let eval = evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(eval.bodies.len(), 2, "{:?}", eval.report.diagnostics);
    let local = &eval.bodies[0].body.mesh;
    let world = &eval.bodies[1].body.mesh;
    assert!(world.validate_deep().is_empty());
    for (a, c) in local.vertices().zip(world.vertices()) {
        let p = local.vertex_position(a).unwrap().map(f64::from);
        let q = world.vertex_position(c).unwrap().map(f64::from);
        for (row, actual) in xf.rows.iter().zip(q) {
            let expected = row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3];
            assert!((actual - expected).abs() < 3e-7);
        }
    }
}

#[test]
fn target_order_is_canonical_and_every_policy_field_affects_the_fingerprint() {
    let policy = RoundPolicy::chamfer(0.005);
    let a = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[1, 3], [2, 4]]),
        policy,
    );
    let b = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[4, 2], [3, 1], [1, 3]]),
        policy,
    );
    assert_eq!(a.recipe_fingerprint(), b.recipe_fingerprint());
    let baseline = rail_recipe(EdgeSelection::SharpEdges, policy).recipe_fingerprint();
    let variants = [
        RoundPolicy {
            kind: RoundKind::Fillet { radius: 0.005 },
            ..policy
        },
        RoundPolicy {
            kind: RoundKind::Chamfer { setback: 0.006 },
            ..policy
        },
        RoundPolicy {
            segments: Some(4),
            ..policy
        },
        RoundPolicy {
            chord_tolerance: 0.0001,
            ..policy
        },
        RoundPolicy {
            sharpness_threshold: 0.75,
            ..policy
        },
        RoundPolicy {
            region: Some(7),
            ..policy
        },
        RoundPolicy {
            max_planar_deviation: 0.0001,
            ..policy
        },
        RoundPolicy {
            max_tangent_turn: 0.3,
            ..policy
        },
    ];
    for variant in variants {
        assert_ne!(
            rail_recipe(EdgeSelection::SharpEdges, variant).recipe_fingerprint(),
            baseline
        );
    }
    assert_ne!(a.recipe_fingerprint(), baseline);
    let dump = text::dump_recipe(&a);
    let restored = text::parse_recipe(&dump).unwrap();
    assert_eq!(restored.recipe_fingerprint(), a.recipe_fingerprint());
    #[cfg(feature = "serde")]
    {
        use crate::interchange::{from_dto, to_dto};
        let json = serde_json::to_string(&to_dto(&a)).unwrap();
        let restored = from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored.recipe_fingerprint(), a.recipe_fingerprint());
    }
}

#[test]
fn unsupported_geometry_is_explicit_and_does_not_emit_a_partial_finish() {
    let body = tessellate_extrude(
        &builders::l_profile(2.0, 2.0, 1.0, 1.0).unwrap(),
        &Placement3::IDENTITY,
        1.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let before = format!("{:?}", body.mesh);
    assert!(matches!(
        finish_edges(&body, &EdgeSelection::SharpEdges, &RoundPolicy::fillet(0.1)),
        Err(EdgeFinishError::Round(RoundError::ConcaveEdge { .. }))
    ));
    assert_eq!(format!("{:?}", body.mesh), before);
    let missing = rail_recipe(
        EdgeSelection::RegionBoundaries(vec![[99, 100]]),
        RoundPolicy::fillet(0.006),
    );
    let result = evaluate(&missing, &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.empty_selection")
    );
    let too_wide = rail_recipe(EdgeSelection::SharpEdges, RoundPolicy::fillet(0.046));
    let result = evaluate(&too_wide, &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.clearance_exceeded")
    );
}

#[test]
fn recessed_panel_region_collisions_are_refused_as_ambiguous() {
    let mut b = RecipeBuilder::new();
    let panel = box_node(&mut b, [1.0, 1.0, 0.1]);
    let cutter = b
        .add(NodeKind::Primitive {
            spec: PrimitiveSpec::Box {
                size: [0.8, 0.8, 0.04],
            },
            placement: Placement3::translate(0.1, 0.1, 0.08),
        })
        .unwrap();
    let csg = b
        .add(NodeKind::Csg {
            op: CsgOp::Difference,
            operands: vec![panel, cutter],
        })
        .unwrap();
    let finish = b
        .add(NodeKind::EdgeFinish {
            child: csg,
            selection: EdgeSelection::RegionBoundaries(vec![[1, 5]]),
            policy: RoundPolicy::fillet(0.005),
        })
        .unwrap();
    let result = evaluate(&b.finish(finish).unwrap(), &EvalPolicy::default()).unwrap();
    assert!(result.bodies.is_empty());
    assert!(
        result
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "eval.edge_finish.ambiguous_selection"),
        "{:?}",
        result.report.diagnostics
    );
}

#[test]
fn semantic_selection_survives_compaction_and_different_tessellation() {
    for segments in [16, 32] {
        let body = tessellate_primitive(
            PrimitiveSpec::Cylinder {
                radius: 0.1,
                height: 0.2,
                segments,
            },
            &Placement3::IDENTITY,
            &EvalPolicy::default(),
        )
        .unwrap();
        let selection = EdgeSelection::RegionBoundaries(vec![[1, 2]]);
        let policy = RoundPolicy::chamfer(0.003);
        let (finished, stats) = finish_edges(&body, &selection, &policy).unwrap();
        assert_eq!(stats.closed_chains, 1);
        assert_eq!(stats.strip_faces, segments);
        // Rounding deleted source faces and vertices. Compact that result,
        // then resolve a different rim by semantic region, never by edge ID.
        let (mesh, remap) = finished.mesh.compact();
        let faces: BTreeMap<_, _> = finished
            .mesh
            .faces()
            .map(|f| {
                (
                    remap.face(f).unwrap(),
                    finished.source_map.face_feature(f).unwrap(),
                )
            })
            .collect();
        let vertices: BTreeMap<_, _> = finished
            .mesh
            .vertices()
            .map(|v| {
                (
                    remap.vertex(v).unwrap(),
                    finished.source_map.vertex_feature(v).unwrap(),
                )
            })
            .collect();
        let source_map = SourceMap::new(
            &mesh,
            mesh.faces().map(|f| faces[&f]).collect(),
            mesh.vertices().map(|v| vertices[&v]).collect(),
        );
        let compacted = TessellatedBody {
            mesh,
            source_map,
            face_materials: BTreeMap::new(),
            refinement: None,
        };
        let bottom = EdgeSelection::RegionBoundaries(vec![[1, 3]]);
        let (a, stats_a) = finish_edges(&finished, &bottom, &policy).unwrap();
        let (b, stats_b) = finish_edges(&compacted, &bottom, &policy).unwrap();
        assert_eq!(stats_a, stats_b);
        let positions = |body: &TessellatedBody| {
            let mut p: Vec<_> = body
                .mesh
                .vertices()
                .map(|v| body.mesh.vertex_position(v).unwrap().map(f32::to_bits))
                .collect();
            p.sort_unstable();
            p
        };
        assert_eq!(positions(&a), positions(&b));
        assert!(a.mesh.validate_deep().is_empty());
        assert!(b.mesh.validate_deep().is_empty());
    }
}

#[test]
fn incomplete_children_remain_refused_when_nested_or_cached() {
    let mut b = RecipeBuilder::new();
    let child = box_node(&mut b, [0.09, 0.2, 2.0]);
    let bad = b
        .add(NodeKind::EdgeFinish {
            child,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.046),
        })
        .unwrap();
    let group = b
        .add(NodeKind::Group {
            children: vec![child, bad],
        })
        .unwrap();
    let root = b
        .add(NodeKind::EdgeFinish {
            child: group,
            selection: EdgeSelection::SharpEdges,
            policy: RoundPolicy::fillet(0.006),
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let result = evaluate_with_cache(&recipe, &EvalPolicy::default(), &mut cache).unwrap();
        assert!(result.bodies.is_empty());
        assert_eq!(result.report.counters.edge_finish_refusals, 2);
        assert!(
            result
                .report
                .diagnostics
                .iter()
                .any(|d| d.code == "eval.edge_finish.incomplete_child")
        );
    }
}
