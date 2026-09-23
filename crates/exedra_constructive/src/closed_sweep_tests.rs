// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Geometric oracles for authored planar surrounds.

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::builders;
use crate::cache::EvalCache;
use crate::evaluate::{Severity, evaluate, evaluate_with_cache};
use crate::ir::{CsgOp, NodeKind, Path3, Plane3, Recipe, RecipeBuilder};
use crate::profile::{Loop2, Seg2};
use crate::section::{SectionPolicy, section_body};
use alloc::vec;

fn rectangle() -> Vec<[f64; 3]> {
    vec![
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 6.0, 0.0],
        [0.0, 6.0, 0.0],
    ]
}

fn surround(
    profile: &Profile2,
    points: &[[f64; 3]],
    datum: [f64; 2],
) -> Result<TessellatedBody, TessellateError> {
    tessellate_mitered_sweep(
        profile,
        &Placement3::IDENTITY,
        points,
        [0.0, 1.0, 0.0],
        datum,
        PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        4.0,
        CapMode::None,
        &EvalPolicy::default(),
    )
}

#[test]
fn asymmetric_rectangle_has_four_miters_and_a_real_opening() {
    let profile = builders::l_profile(0.8, 0.6, 0.6, 0.45).unwrap();
    let body = surround(&profile, &rectangle(), [0.0; 2]).unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    let d = discretize_profile(&profile, &EvalPolicy::default().discretize).unwrap();
    assert_eq!(
        body.mesh.vertices().count(),
        4 * d.points_len(),
        "one shared seam ring"
    );
    assert_eq!(body.mesh.faces().count(), 4 * d.points_len());
    let points: Vec<_> = body
        .mesh
        .vertices()
        .map(|v| body.mesh.vertex_position(v).unwrap().map(f64::from))
        .collect();
    for (i, &[x, y]) in d.outer.points.iter().enumerate() {
        for (ring, expected) in [
            [x, x, y],
            [10.0 - x, x, y],
            [10.0 - x, 6.0 - x, y],
            [x, 6.0 - x, y],
        ]
        .into_iter()
        .enumerate()
        {
            assert!(norm(sub(points[ring * d.points_len() + i], expected)) < 1e-6);
        }
    }
    let area = 0.8 * 0.15 + 0.2 * 0.45;
    let first_moment = 0.8 * 0.8 * 0.15 / 2.0 + 0.2 * 0.2 * 0.45 / 2.0;
    assert!((mesh_volume(&body.mesh) - (32.0 * area - 8.0 * first_moment)).abs() < 1e-5);
    assert!(
        body.source_map
            .face_features()
            .iter()
            .all(|f| matches!(f, Feature::SweepWall { band: 0..=3, .. }))
    );
    let section = section_body(
        &body,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 0.075,
        },
        &SectionPolicy::default(),
    )
    .unwrap();
    assert_eq!(section.regions.len(), 1);
    assert_eq!(
        section.regions[0].holes.len(),
        1,
        "opening is geometry, not a missing cap"
    );
    assert!((section.measure().unwrap().area - (60.0 - 8.4 * 4.4)).abs() < 1e-5);
}

fn sorted_positions(body: &TessellatedBody) -> Vec<[f32; 3]> {
    let mut points: Vec<_> = body
        .mesh
        .vertices()
        .map(|v| *body.mesh.vertex_position(v).unwrap())
        .collect();
    points.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    points
}

#[test]
fn relocated_start_reversed_traversal_and_normal_sign_have_explicit_frames() {
    let profile = builders::l_profile(0.8, 0.6, 0.6, 0.45).unwrap();
    let expected = surround(&profile, &rectangle(), [0.0; 2]).unwrap();
    let mut rotated = rectangle();
    rotated.rotate_left(1);
    // Keep physical section +Y pointing up: section X follows the new run.
    let moved = tessellate_mitered_sweep(
        &profile,
        &Placement3::IDENTITY,
        &rotated,
        [-1.0, 0.0, 0.0],
        [0.0; 2],
        PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, -3.0],
        },
        4.0,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_eq!(sorted_positions(&expected), sorted_positions(&moved));
    // Reversal flips section Y when physical section X is held inward.
    // Author the corresponding reflected profile; the kernel does not guess.
    let reflected_profile = Profile2::simple(
        Loop2::new(
            profile
                .outer()
                .segs()
                .iter()
                .rev()
                .map(|s| Seg2::line((s.to.x, -s.to.y)))
                .collect(),
        )
        .unwrap(),
    )
    .unwrap();
    let original = rectangle();
    let reversed = [original[0], original[3], original[2], original[1]];
    let reversed = tessellate_mitered_sweep(
        &reflected_profile,
        &Placement3::IDENTITY,
        &reversed,
        [1.0, 0.0, 0.0],
        [0.0; 2],
        PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        4.0,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_eq!(sorted_positions(&expected), sorted_positions(&reversed));
    assert!((mesh_volume(&expected.mesh) - mesh_volume(&reversed.mesh)).abs() < 1e-6);
    assert_clean(&moved);
    assert_clean(&reversed);
}

#[test]
fn concave_surround_and_collinear_seam_close_without_artificial_creases() {
    let profile = builders::rect_from_corner(0.3, 0.4).unwrap();
    let points = [
        [2.0, 0.0, 0.0],
        [6.0, 0.0, 0.0],
        [6.0, 2.0, 0.0],
        [3.0, 2.0, 0.0],
        [3.0, 5.0, 0.0],
        [0.0, 5.0, 0.0],
        [0.0, 0.0, 0.0],
    ];
    let body = surround(&profile, &points, [0.0; 2]).unwrap();
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    // Independent inset-area oracle for this orthogonal concave polygon.
    assert!((mesh_volume(&body.mesh) - (22.0 * 0.3 - 4.0 * 0.3 * 0.3) * 0.4).abs() < 1e-5);
    let vertices: Vec<_> = body.mesh.vertices().collect();
    let seam = &vertices[..4];
    let mut seen = 0;
    for face in body.mesh.faces() {
        for edge in body.mesh.face_loop(face) {
            if seam.contains(&body.mesh.from_vertex(edge).unwrap())
                && seam.contains(&body.mesh.to_vertex(edge).unwrap())
            {
                let edge = body.mesh.canonical_edge(edge).unwrap();
                assert_eq!(body.mesh.edge_sharpness(edge).unwrap_or(0.0), 0.0);
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 8);
}

#[test]
fn declared_plane_caps_and_degenerate_closure_are_checked() {
    let profile = builders::rect_from_corner(0.3, 0.4).unwrap();
    let run = |points: &[[f64; 3]], normal, caps| {
        tessellate_mitered_sweep(
            &profile,
            &Placement3::IDENTITY,
            points,
            [0.0, 1.0, 0.0],
            [0.0; 2],
            PathClosure::ClosedPlanar { normal },
            4.0,
            caps,
            &EvalPolicy::default(),
        )
    };
    assert!(matches!(
        run(&rectangle(), [0.0; 3], CapMode::None),
        Err(TessellateError::InvalidSweepPlane)
    ));
    assert!(matches!(
        run(&rectangle(), [0.0, 0.0, 1.0], CapMode::Both),
        Err(TessellateError::ClosedSweepCaps)
    ));
    let mut points = rectangle();
    points[2][2] = 0.01;
    assert!(matches!(
        run(&points, [0.0, 0.0, 1.0], CapMode::None),
        Err(TessellateError::NonPlanarSweep { point: 2, .. })
    ));
    let points = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [3.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    assert!(matches!(
        run(&points, [0.0, 0.0, 1.0], CapMode::None),
        Err(TessellateError::PathCusp { point: 0 })
    ));
    let mut points = rectangle();
    points.push(points[0]);
    assert!(matches!(
        run(&points, [0.0, 0.0, 1.0], CapMode::None),
        Err(TessellateError::InvalidSweepPath)
    ));
}

#[test]
fn tilted_plane_reflection_and_curved_holed_section_preserve_geometry() {
    let profile = builders::ring(0.3, 0.1).unwrap();
    let points = rectangle();
    let original = surround(&profile, &points, [0.0; 2]).unwrap();
    // Rotate the path plane into XZ; authored section X follows the plane.
    let tilted: Vec<_> = points.iter().map(|p| [p[0], -p[2], p[1]]).collect();
    let placed = tessellate_mitered_sweep(
        &profile,
        &Placement3 {
            rows: [
                [-1.0, 0.0, 0.0, 3.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        },
        &tilted,
        [0.0, 0.0, 1.0],
        [0.0; 2],
        PathClosure::ClosedPlanar {
            normal: [0.0, -1e-300, 0.0],
        },
        4.0,
        CapMode::None,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert_clean(&placed);
    assert!(placed.mesh.boundary_loops().unwrap().is_empty());
    assert_eq!(
        original.source_map.face_features(),
        placed.source_map.face_features()
    );
    // Placement narrows a different coordinate to f32; allow that rounding,
    // while checking each vertex against the independent rigid-map oracle.
    assert!((mesh_volume(&placed.mesh) / mesh_volume(&original.mesh) - 1.0).abs() < 1e-6);
    for (a, b) in original.mesh.vertices().zip(placed.mesh.vertices()) {
        let a = original.mesh.vertex_position(a).unwrap().map(f64::from);
        let b = placed.mesh.vertex_position(b).unwrap().map(f64::from);
        assert!(norm(sub(b, [3.0 - a[0], -a[2], a[1]])) < 1e-6);
    }
    let d = discretize_profile(&profile, &EvalPolicy::default().discretize).unwrap();
    assert_eq!(
        placed.mesh.vertices().count(),
        points.len() * d.points_len()
    );
}

fn retained_surround(datum: [f64; 2], cuts: bool, reflect: bool) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(builders::l_profile(0.8, 0.6, 0.6, 0.45).unwrap());
    let source = builder.source_ref("surround");
    let material = builder.material_slot("molding");
    let mut root = builder
        .with_source(source)
        .with_material(material)
        .add(NodeKind::Sweep {
            section: SectionLaw::IDENTITY,
            profile,
            path: Path3::MiteredPolyline {
                points: rectangle(),
                section_x: [0.0, 1.0, 0.0],
                section_origin: datum,
                closure: PathClosure::ClosedPlanar {
                    normal: [0.0, 0.0, 1.0],
                },
                miter_limit: 4.0,
            },
            caps: CapMode::None,
        })
        .unwrap();
    if cuts {
        for x in [2.3, 6.3] {
            let source = builder.source_ref("cutter");
            let material = builder.material_slot("cut-face");
            let cut = builder
                .with_source(source)
                .with_material(material)
                .add(NodeKind::Primitive {
                    spec: PrimitiveSpec::Box {
                        size: [0.7, 2.0, 2.0],
                    },
                    placement: Placement3::translate(x, -0.5, -0.5),
                })
                .unwrap();
            root = builder
                .add(NodeKind::Csg {
                    op: CsgOp::Difference,
                    operands: vec![root, cut],
                })
                .unwrap();
        }
    }
    if reflect {
        root = builder
            .add(NodeKind::Instance {
                of: root,
                placement: Placement3 {
                    rows: [
                        [-1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                },
            })
            .unwrap();
    }
    builder.finish(root).unwrap()
}

#[test]
fn retained_closure_roundtrips_and_replays_from_cache() {
    let recipe = retained_surround([0.2, 0.1], false, false);
    let text = crate::text::dump_recipe(&recipe);
    assert!(text.contains("closure closed_planar"));
    assert_eq!(
        recipe.recipe_fingerprint(),
        crate::text::parse_recipe(&text)
            .unwrap()
            .recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&crate::interchange::to_dto(&recipe)).unwrap();
        assert!(
            json.contains("mitered_path_sweep"),
            "old readers must reject the new geometry semantics"
        );
        let dto = serde_json::from_str(&json).unwrap();
        assert_eq!(
            recipe.recipe_fingerprint(),
            crate::interchange::from_dto(&dto)
                .unwrap()
                .recipe_fingerprint()
        );
    }
    assert_ne!(
        recipe.recipe_fingerprint(),
        retained_surround([0.0; 2], false, false).recipe_fingerprint()
    );
    let policy = EvalPolicy::default();
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(warm.report.counters.tessellations, 0);
    assert_eq!(
        cold.bodies[0].body.source_map,
        warm.bodies[0].body.source_map
    );
    assert_eq!(
        sorted_positions(&cold.bodies[0].body),
        sorted_positions(&warm.bodies[0].body)
    );
    let mut limited = policy;
    limited.max_sweep_vertices = 23;
    assert!(matches!(
        evaluate_with_cache(&recipe, &limited, &mut cache)
            .unwrap_err()
            .error,
        TessellateError::SweepVertexBudgetExceeded {
            required: 24,
            maximum: 23
        }
    ));
}

#[test]
fn boolean_cuts_keep_run_origins_materials_and_sampling_without_inheriting_checks() {
    let recipe = retained_surround([0.0; 2], true, true);
    let policy = EvalPolicy::default();
    let mut cache = EvalCache::new();
    for warm in [false, true] {
        let result = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
        assert!(
            result.report.clean_at(Severity::Error),
            "{:?}",
            result.report.diagnostics
        );
        if warm {
            assert_eq!(result.report.counters.tessellations, 0);
        }
        assert_eq!(result.bodies.len(), 1);
        let placed = &result.bodies[0];
        let body = &placed.body;
        assert_clean(body);
        assert!(body.mesh.boundary_loops().unwrap().is_empty());
        assert!(body.sweep_checks.is_none());
        assert!(body.path_sampling.is_none());
        let mut bands = [false; 4];
        let mut cut_faces = 0;
        for face in body.mesh.faces() {
            let origin = body.source_map.surface_origin(face).unwrap();
            if let Feature::SweepWall { band, seg, .. } = origin.feature {
                bands[usize::from(band)] = true;
                assert_eq!(
                    body.mesh
                        .attrs()
                        .dense(exedra_mesh::attr::FACE_REGION)
                        .unwrap()
                        .get(face.as_id()),
                    Some(&(REGION_WALL_BASE + seg))
                );
                assert_eq!(origin.source.as_deref(), Some("surround"));
                let evidence = body.source_map.sweep_sampling(face).unwrap();
                assert_eq!(evidence.profile, policy.discretize);
                assert!(
                    evidence.path.is_none(),
                    "each polyline band is an authored source run"
                );
                assert_eq!(
                    recipe.slot(placed.material_for_face(face).unwrap()),
                    Some("molding")
                );
            } else {
                cut_faces += 1;
                assert!(body.source_map.sweep_sampling(face).is_none());
                assert_eq!(
                    recipe.slot(placed.material_for_face(face).unwrap()),
                    Some("cut-face")
                );
            }
        }
        assert!(bands.into_iter().all(|present| present));
        assert!(cut_faces > 0);
        assert!(mesh_volume(&body.mesh) > 0.0);
    }
    // Ancestry is revision-scoped, just like the feature map.
    let result = evaluate(&recipe, &policy).unwrap();
    let mut body = (*result.bodies[0].body).clone();
    let vertex = body.mesh.vertices().next().unwrap();
    let mut edit = body.mesh.edit();
    exedra_mesh::op::set_vertex_position(&mut edit, vertex, [42.0; 3]).unwrap();
    #[expect(unused_must_use, reason = "the unit sink output is irrelevant here")]
    {
        edit.finish();
    }
    assert!(body.source_map.check(&body.mesh).is_err());
}

#[test]
fn profile_datum_is_applied_before_mitering() {
    let profile = builders::rect_from_corner(1.0, 0.5).unwrap();
    let body = surround(&profile, &rectangle(), [0.4, 0.2]).unwrap();
    let bounds = crate::evaluate::mesh_bounds(&body.mesh);
    for (actual, expected) in bounds.min.into_iter().zip([-0.4, -0.4, -0.2]) {
        assert!((actual - expected).abs() < 1e-6);
    }
    for (actual, expected) in bounds.max.into_iter().zip([10.4, 6.4, 0.3]) {
        assert!((actual - expected).abs() < 1e-6);
    }
    assert_clean(&body);
}

#[test]
fn closing_corner_and_closing_band_get_the_same_checks() {
    let profile = builders::rect_from_corner(1.0, 0.5).unwrap();
    let mut policy = EvalPolicy::default();
    let run = |points: &[[f64; 3]], limit, policy: &EvalPolicy| {
        tessellate_mitered_sweep(
            &profile,
            &Placement3::IDENTITY,
            points,
            [0.0, 1.0, 0.0],
            [0.0; 2],
            PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            },
            limit,
            CapMode::None,
            policy,
        )
    };
    assert!(matches!(
        run(&rectangle(), 1.4, &policy),
        Err(TessellateError::MiterLimitExceeded { point: 0, .. })
    ));
    assert!(run(&rectangle(), 1.0 / libm::sqrt(0.5), &policy).is_ok());
    let short_close = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 6.0, 0.0],
        [5.0, 6.0, 0.0],
        [5.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    assert!(matches!(
        run(&short_close, 4.0, &policy),
        Err(TessellateError::SweepFoldover { band: 5, .. })
    ));
    policy.sweep_path.max_path_edges = 3;
    assert!(matches!(
        run(&rectangle(), 4.0, &policy),
        Err(TessellateError::Path(
            crate::path::PathDiscretizeError::PathBudgetExceeded { maximum: 3 }
        ))
    ));
    policy.sweep_path.max_path_edges = 4;
    policy.max_sweep_vertices = 15;
    assert!(matches!(
        run(&rectangle(), 4.0, &policy),
        Err(TessellateError::SweepVertexBudgetExceeded {
            required: 16,
            maximum: 15
        })
    ));
}
