// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{vec, vec::Vec};

use crate::{
    builders,
    discretize::discretize_profile,
    evaluate::evaluate,
    ir::{CapMode, LoftPolicy, LoftSection, NodeKind, Placement3, RecipeBuilder},
    profile::{Loop2, Profile2, Seg2},
    tessellate::{EvalPolicy, Feature, TessellatedBody},
};
use exedra_mesh::{ExtractParams, FaceId};
use exedra_mesh_ops::measure::signed_volume;

fn leaf_panel(rows: u32) -> Profile2 {
    let width = 0.027_449_906_698_392_81;
    let height = 0.150;
    let holes = (0..rows)
        .map(|row| {
            let across = width * 0.5;
            let up = (f64::from(row) + 0.5) * height / f64::from(rows);
            let lean = if row % 2 == 0 { 0.22 } else { -0.22 };
            let (w, h) = (0.0054, 0.009);
            let p = |x, y| (across + x + lean * y, up + y);
            Loop2::new(vec![
                Seg2::cubic(
                    p(0.0, h),
                    p(w * 4.0 / 3.0, -h / 3.0),
                    p(w * 4.0 / 3.0, h / 3.0),
                ),
                Seg2::cubic(
                    p(0.0, -h),
                    p(-w * 4.0 / 3.0, h / 3.0),
                    p(-w * 4.0 / 3.0, -h / 3.0),
                ),
            ])
            .unwrap()
            .reversed()
        })
        .collect::<Vec<_>>();
    Profile2::new(
        builders::rect_from_corner(width, height)
            .unwrap()
            .outer()
            .clone(),
        holes,
    )
    .unwrap()
}

#[test]
fn multiple_curved_holes_have_intact_caps() {
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.0003;
    for rows in [1, 5] {
        let source = leaf_panel(rows);
        let sampled = discretize_profile(&source, &policy.discretize).unwrap();
        let expected_area: f64 = core::iter::once(&sampled.outer)
            .chain(&sampled.holes)
            .map(|ring| {
                ring.points
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let b = ring.points[(i + 1) % ring.points.len()];
                        (a[0] * b[1] - b[0] * a[1]) * 0.5
                    })
                    .sum::<f64>()
            })
            .sum();
        for loft in [false, true] {
            let mut builder = RecipeBuilder::new();
            let profile = builder.add_profile(source.clone());
            let operation = if loft {
                NodeKind::Loft {
                    sections: vec![
                        LoftSection::new(Placement3::IDENTITY, profile),
                        LoftSection::new(Placement3::translate(0.0, 0.0, 0.006), profile),
                    ],
                    policy: LoftPolicy::Ruled,
                    caps: CapMode::Both,
                }
            } else {
                NodeKind::Extrude {
                    profile,
                    height: 0.006,
                    caps: CapMode::Both,
                    placement: Placement3::IDENTITY,
                }
            };
            let root = builder.add(operation).unwrap();
            let recipe = builder.finish(root).unwrap();
            let result = evaluate(&recipe, &policy).unwrap_or_else(|error| {
                panic!("{rows} disjoint leaf holes, loft={loft}: {error:?}")
            });
            assert_eq!(result.bodies.len(), 1);
            assert_panel(&result.bodies[0].body, rows, expected_area);
        }
    }
}

fn assert_panel(body: &TessellatedBody, holes: u32, expected_area: f64) {
    let mesh = &body.mesh;
    assert!(mesh.validate_deep().is_empty());
    body.source_map.check(mesh).unwrap();
    let edge_count = mesh
        .faces()
        .map(|f| mesh.face_loop(f).count())
        .sum::<usize>()
        / 2;
    assert_eq!(
        mesh.vertices().count() + mesh.faces().count() + 2 * holes as usize,
        edge_count + 2
    );
    let mut cap_areas = [0.0; 2];
    let mut cap_triangles = [Vec::new(), Vec::new()];
    for face in mesh.faces() {
        for he in mesh.face_loop(face) {
            assert_ne!(mesh.face(mesh.twin(he).unwrap()), Some(FaceId::OUTSIDE));
        }
        let cap = match body.source_map.face_feature(face).unwrap() {
            Feature::CapStart => 0,
            Feature::CapEnd => 1,
            _ => continue,
        };
        let points: Vec<_> = mesh
            .face_loop(face)
            .map(|he| {
                mesh.vertex_position(mesh.to_vertex(he).unwrap())
                    .unwrap()
                    .map(f64::from)
            })
            .collect();
        let triangle: [[f64; 3]; 3] = points.try_into().unwrap();
        let [a, b, c] = triangle;
        let area2 = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert!(if cap == 0 { area2 < 0.0 } else { area2 > 0.0 });
        cap_areas[cap] += area2.abs() * 0.5;
        cap_triangles[cap].push(triangle);
    }
    for (area, triangles) in cap_areas.into_iter().zip(cap_triangles) {
        assert!((area - expected_area).abs() < expected_area * 1e-6);
        // Each opening stays empty; each material interval between openings
        // stays covered. These are independent interior point witnesses.
        for row in 0..holes {
            let y = (f64::from(row) + 0.5) * 0.15 / f64::from(holes);
            assert!(
                !triangles
                    .iter()
                    .any(|t| covers(t, [0.027_449_906_698_392_81 * 0.5, y]))
            );
            assert!(triangles.iter().any(|t| covers(t, [0.002, y])));
        }
        for gap in 1..holes {
            let y = f64::from(gap) * 0.15 / f64::from(holes);
            assert!(
                triangles
                    .iter()
                    .any(|t| covers(t, [0.027_449_906_698_392_81 * 0.5, y]))
            );
        }
    }
    let (triangles, _) = mesh.to_trimesh(&ExtractParams::default());
    let volume = signed_volume(&triangles).unwrap().value;
    assert!((volume - expected_area * 0.006).abs() < expected_area * 0.006 * 1e-6);
}

fn covers(triangle: &[[f64; 3]; 3], q: [f64; 2]) -> bool {
    let sides = core::array::from_fn::<_, 3, _>(|i| {
        let a = triangle[i];
        let b = triangle[(i + 1) % 3];
        (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
    });
    sides.iter().all(|&v| v >= 0.0) || sides.iter().all(|&v| v <= 0.0)
}
