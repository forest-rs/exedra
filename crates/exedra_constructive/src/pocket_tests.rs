// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::builders;
use crate::evaluate::{Fidelity, evaluate};
use crate::ir::{
    CapMode, ExtrusionEnd, NodeId, NodeKind, Placement3, PrimitiveSpec, RecipeBuilder, RecipeError,
};
use crate::tessellate::EvalPolicy;
use exedra_math::{cross, dot, sub};
use exedra_mesh::{FaceId, FaceTriangulation};

/// A 4 x 3 block, 2 tall, placed off the origin.
fn block(builder: &mut RecipeBuilder) -> NodeId {
    let profile = builder.add_profile(builders::rect_centered(4.0, 3.0).unwrap());
    builder
        .add_node(NodeKind::Extrude {
            profile,
            placement: Placement3::rotate_z_then_translate(0.4, 5.0, -2.0, 1.0),
            height: 2.0,
            caps: CapMode::Both,
        })
        .unwrap()
}

fn closed_volume(recipe: &crate::ir::Recipe) -> f64 {
    let result = evaluate(recipe, &EvalPolicy::default()).unwrap();
    assert_eq!(
        result.report.fidelity_of(recipe.root()),
        Some(Fidelity::Exact),
        "{:?}",
        result.report.diagnostics
    );
    assert_eq!(result.bodies.len(), 1);
    let body = &result.bodies[0].body;
    let mesh = &body.mesh;
    assert!(mesh.validate_deep().is_empty());
    body.source_map.check(mesh).unwrap();
    let mut volume = 0.0;
    for face in mesh.faces() {
        assert!(
            mesh.face_loop(face)
                .all(|he| mesh.face(mesh.twin(he).unwrap()) != Some(FaceId::OUTSIDE)),
            "open face {face:?}"
        );
        let (triangles, fallback) = mesh.face_triangles_counted(face, FaceTriangulation::Robust);
        assert!(!fallback, "triangulation fallback on {face:?}");
        for t in triangles {
            let [a, b, c] = t.map(|he| {
                mesh.vertex_position(mesh.to_vertex(he).unwrap())
                    .unwrap()
                    .map(f64::from)
            });
            assert_ne!(cross(sub(b, a), sub(c, a)), [0.0; 3]);
            volume += dot(a, cross(b, c)) / 6.0;
        }
    }
    volume
}

#[test]
fn pockets_remove_their_region_to_depth_from_either_cap() {
    // A 2 x 1 recess from the end cap, then a 1 x 1 one from the start
    // cap beside it, chained.
    let mut builder = RecipeBuilder::new();
    let base = block(&mut builder);
    let recess = builder.add_profile(
        builders::rect_centered(2.0, 1.0)
            .unwrap()
            .translated((-0.5, 0.5))
            .unwrap(),
    );
    let first = builder
        .add_node(NodeKind::Pocket {
            base,
            profile: recess,
            end: ExtrusionEnd::End,
            depth: 0.75,
        })
        .unwrap();
    let square = builder.add_profile(
        builders::rect_centered(1.0, 1.0)
            .unwrap()
            .translated((1.0, -0.75))
            .unwrap(),
    );
    let root = builder
        .add_node(NodeKind::Pocket {
            base: first,
            profile: square,
            end: ExtrusionEnd::Start,
            depth: 0.5,
        })
        .unwrap();
    let recipe = builder.finish(root).unwrap();
    let volume = closed_volume(&recipe);
    let expected = 24.0 - 2.0 * 0.75 - 0.5;
    assert!((volume - expected).abs() < 1e-4, "{volume} {expected}");
}

#[test]
fn pockets_name_what_they_cannot_cut() {
    let refused = |build: &dyn Fn(&mut RecipeBuilder, NodeId) -> NodeKind| {
        let mut builder = RecipeBuilder::new();
        let base = block(&mut builder);
        let kind = build(&mut builder, base);
        builder.add_node(kind).unwrap_err()
    };
    let recess = |builder: &mut RecipeBuilder| {
        builder.add_profile(builders::rect_centered(1.0, 1.0).unwrap())
    };
    // As deep as the block is tall.
    assert_eq!(
        refused(&|builder, base| NodeKind::Pocket {
            base,
            profile: recess(builder),
            end: ExtrusionEnd::End,
            depth: 2.0,
        }),
        RecipeError::InvalidParameter { what: "depth" }
    );
    // A region with a hole would leave an island.
    assert_eq!(
        refused(&|builder, base| NodeKind::Pocket {
            base,
            profile: builder.add_profile(builders::ring(1.0, 0.5).unwrap()),
            end: ExtrusionEnd::End,
            depth: 0.5,
        }),
        RecipeError::InvalidParameter {
            what: "pocket profile holes"
        }
    );
    // Neither a closed extrusion nor a pocket of one.
    assert_eq!(
        refused(&|builder, _| {
            let base = builder
                .add_node(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [1.0, 1.0, 1.0],
                    },
                    placement: Placement3::IDENTITY,
                })
                .unwrap();
            NodeKind::Pocket {
                base,
                profile: recess(builder),
                end: ExtrusionEnd::End,
                depth: 0.5,
            }
        }),
        RecipeError::InvalidParameter {
            what: "pocket base"
        }
    );
    assert_eq!(
        refused(&|builder, _| {
            let profile = recess(builder);
            let base = builder
                .add_node(NodeKind::Extrude {
                    profile,
                    placement: Placement3::IDENTITY,
                    height: 2.0,
                    caps: CapMode::Start,
                })
                .unwrap();
            NodeKind::Pocket {
                base,
                profile,
                end: ExtrusionEnd::End,
                depth: 0.5,
            }
        }),
        RecipeError::InvalidParameter {
            what: "pocket base"
        }
    );
}
