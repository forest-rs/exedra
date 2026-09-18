// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Geometric oracles for the controlled rail contract.

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::builders;
use crate::cache::EvalCache;
use crate::evaluate::{evaluate, evaluate_with_cache};
use crate::ir::{NodeKind, Path3, Recipe, RecipeBuilder};
use alloc::vec;

fn asymmetric() -> Profile2 {
    builders::l_profile(0.8, 0.6, 0.2, 0.15).expect("asymmetric L")
}

fn rail(profile: &Profile2, path: &[[f64; 3]]) -> Result<TessellatedBody, TessellateError> {
    tessellate_mitered_sweep(
        profile,
        &Placement3::IDENTITY,
        path,
        [1.0, 0.0, 0.0],
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
}

fn positions(body: &TessellatedBody) -> Vec<[f64; 3]> {
    body.mesh
        .vertices()
        .map(|v| {
            body.mesh
                .vertex_position(v)
                .expect("position")
                .map(f64::from)
        })
        .collect()
}

#[test]
fn authored_orientation_is_continuous_across_world_axis_seed_boundary() {
    let profile = asymmetric();
    let a = rail(&profile, &[[0.0; 3], [1e-6, 2e-6, 4.0]]).expect("rail A");
    let b = rail(&profile, &[[0.0; 3], [2e-6, 1e-6, 4.0]]).expect("rail B");
    assert_clean(&a);
    assert_clean(&b);
    for (a, b) in positions(&a).into_iter().zip(positions(&b)) {
        assert!(
            norm(sub(a, b)) < 4e-6,
            "a tiny path edit must not rotate the L section"
        );
    }
    let straight = rail(&profile, &[[0.0; 3], [0.0, 0.0, 4.0]]).expect("straight rail");
    let extruded = tessellate_extrude(
        &profile,
        &Placement3::IDENTITY,
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .expect("extrude");
    assert!((mesh_volume(&straight.mesh) - mesh_volume(&extruded.mesh)).abs() < 1e-7);
}

#[test]
fn right_angle_miter_preserves_both_run_sections_and_caps() {
    let profile = asymmetric();
    let path = [[0.0; 3], [0.0, 0.0, 4.0], [3.0, 0.0, 4.0]];
    let body = rail(&profile, &path).expect("mitered L rail");
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().expect("boundaries").is_empty());
    let points = positions(&body);
    let d = discretize_profile(&profile, &EvalPolicy::default().discretize).expect("profile");
    let n = d.points_len();
    assert_eq!(
        body.sweep_checks,
        Some(SweepChecks {
            bands: 2,
            section_vertices: n
        })
    );
    for (i, p) in d.outer.points.iter().enumerate() {
        // Independent cut-plane oracle: x + z = 4. Outgoing section X is -Z.
        let expected = [
            [p[0], p[1], 0.0],
            [p[0], p[1], 4.0 - p[0]],
            [3.0, p[1], 4.0 - p[0]],
        ];
        for (ring, expected) in expected.into_iter().enumerate() {
            assert!(
                norm(sub(points[ring * n + i], expected)) < 3e-7,
                "miter dimensions at ring {ring}, vertex {i}"
            );
        }
    }
    for face in body.mesh.faces() {
        let feature = body.source_map.face_feature(face).expect("face provenance");
        let vertices: Vec<_> = body
            .mesh
            .face_loop(face)
            .map(|he| {
                let v = body.mesh.to_vertex(he).expect("origin");
                body.mesh
                    .vertex_position(v)
                    .expect("position")
                    .map(f64::from)
            })
            .collect();
        let normal = cross(sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0]));
        match feature {
            Feature::CapStart => assert!(normal[2] < 0.0, "start cap faces -Z"),
            Feature::CapEnd => assert!(normal[0] > 0.0, "end cap faces +X"),
            Feature::SweepWall { band, .. } => assert!(band < 2),
            _ => panic!("unexpected rail feature: {feature:?}"),
        }
    }
    assert!(mesh_volume(&body.mesh) > 0.0, "outward winding");
}

#[test]
fn spatial_bends_preserve_longitudinal_generators() {
    let profile = asymmetric();
    let path = [
        [0.0; 3],
        [0.0, 0.0, 5.0],
        [5.0, 0.0, 5.0],
        [5.0, 5.0, 5.0],
        [8.0, 8.0, 8.0],
    ];
    let body = rail(&profile, &path).expect("spatial rail");
    assert_clean(&body);
    let points = positions(&body);
    let n = body.sweep_checks.expect("checks").section_vertices;
    for (band, run) in path.windows(2).enumerate() {
        let t = sweep_direction(run[0], run[1]).expect("direction");
        for i in 0..n {
            let generator = sub(points[(band + 1) * n + i], points[band * n + i]);
            assert!(
                norm(cross(generator, t)) < 2e-6,
                "constant section along run {band}"
            );
            assert!(dot(generator, t) > 0.0);
        }
    }
}

#[test]
fn reflected_placement_keeps_outward_winding_and_provenance() {
    let profile = asymmetric();
    let path = [[0.0; 3], [0.0, 0.0, 4.0], [3.0, 0.0, 4.0]];
    let original = rail(&profile, &path).expect("original");
    let placement = Placement3 {
        rows: [
            [-1.0, 0.0, 0.0, 10.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };
    let mirrored = tessellate_mitered_sweep(
        &profile,
        &placement,
        &path,
        [1.0, 0.0, 0.0],
        4.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .expect("mirrored");
    assert_clean(&mirrored);
    assert!((mesh_volume(&original.mesh) - mesh_volume(&mirrored.mesh)).abs() < 2e-6);
    assert_eq!(
        original.source_map.face_features(),
        mirrored.source_map.face_features()
    );
    for (a, b) in positions(&original).into_iter().zip(positions(&mirrored)) {
        assert!(norm(sub(apply_placement(&placement, a), b)) < 1e-6);
    }
}

#[test]
fn caps_and_holes_close_only_when_requested() {
    let profile = builders::ring(1.0, 0.4).expect("ring");
    let path = [[0.0; 3], [0.0, 0.0, 5.0], [5.0, 0.0, 5.0]];
    for caps in [CapMode::None, CapMode::Start, CapMode::End, CapMode::Both] {
        let body = tessellate_mitered_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            [1.0, 0.0, 0.0],
            4.0,
            caps,
            &EvalPolicy::default(),
        )
        .expect("holed rail");
        assert_clean(&body);
        let boundaries = body.mesh.boundary_loops().expect("boundary loops").len();
        assert_eq!(
            boundaries,
            match caps {
                CapMode::None => 4,
                CapMode::Both => 0,
                _ => 2,
            }
        );
    }
}

#[test]
fn invalid_inputs_and_local_foldovers_are_typed_failures() {
    let profile = builders::rect_from_corner(2.0, 1.0).expect("rectangle");
    let straight = [[0.0; 3], [0.0, 0.0, 3.0]];
    for x in [
        [0.0; 3],
        [0.0, 0.0, 1.0],
        [1e-14, 0.0, 1.0],
        [f64::NAN, 1.0, 0.0],
    ] {
        assert!(matches!(
            tessellate_mitered_sweep(
                &profile,
                &Placement3::IDENTITY,
                &straight,
                x,
                4.0,
                CapMode::Both,
                &EvalPolicy::default()
            ),
            Err(TessellateError::InvalidSweepOrientation)
        ));
    }
    for path in [
        vec![],
        vec![[0.0; 3]],
        vec![[0.0; 3], [0.0; 3]],
        vec![[0.0; 3], [0.0, 0.0, 3.0], [0.0; 3]],
        vec![[0.0; 3], [f64::INFINITY; 3]],
    ] {
        assert!(matches!(
            rail(&profile, &path),
            Err(TessellateError::InvalidSweepPath)
        ));
    }
    for limit in [0.0, 0.9, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            tessellate_mitered_sweep(
                &profile,
                &Placement3::IDENTITY,
                &straight,
                [1.0, 0.0, 0.0],
                limit,
                CapMode::Both,
                &EvalPolicy::default()
            ),
            Err(TessellateError::InvalidMiterLimit)
        ));
    }
    assert!(matches!(
        rail(&profile, &[[0.0; 3], [0.0, 0.0, 3.0], [0.0, 0.0, -1.0]]),
        Err(TessellateError::PathCusp { point: 1 })
    ));
    assert!(matches!(
        rail(&profile, &[[0.0; 3], [0.0, 0.0, 3.0], [0.01, 0.0, 0.0]]),
        Err(TessellateError::MiterLimitExceeded { point: 1, .. })
    ));
    assert!(matches!(
        rail(&profile, &[[0.0; 3], [0.0, 0.0, 0.2], [3.0, 0.0, 0.2]]),
        Err(TessellateError::SweepFoldover { band: 0, .. })
    ));
    // Two individually moderate corners consume their intervening short run.
    assert!(matches!(
        rail(
            &profile,
            &[[0.0; 3], [0.0, 0.0, 4.0], [0.2, 0.0, 4.0], [0.2, 0.0, 0.0]]
        ),
        Err(TessellateError::SweepFoldover { band: 1, .. })
    ));
}

fn recipe(x: [f64; 3], miter_limit: f64) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(asymmetric());
    let root = b
        .add(NodeKind::Sweep {
            profile,
            path: Path3::MiteredPolyline {
                points: vec![[0.0; 3], [0.0, 0.0, 4.0], [3.0, 0.0, 4.0]],
                section_x: x,
                miter_limit,
            },
            caps: CapMode::Both,
        })
        .expect("valid rail recipe");
    b.finish(root).expect("recipe")
}

#[test]
fn recipe_roundtrips_and_cache_preserve_orientation_and_evidence() {
    let r = recipe([1.0, 0.0, 0.0], 4.0);
    let text = crate::text::dump_recipe(&r);
    let parsed = crate::text::parse_recipe(&text).expect("text roundtrip");
    assert_eq!(r.recipe_fingerprint(), parsed.recipe_fingerprint());
    assert!(crate::text::parse_recipe(&text.replace("mitered_sweep", "sweep")).is_err());
    #[cfg(feature = "serde")]
    {
        let dto = crate::interchange::to_dto(&r);
        let json = serde_json::to_string(&dto).expect("json");
        let dto = serde_json::from_str(&json).expect("read json");
        let rebuilt = crate::interchange::from_dto(&dto).expect("interchange");
        assert_eq!(r.recipe_fingerprint(), rebuilt.recipe_fingerprint());
    }
    assert_ne!(
        r.recipe_fingerprint(),
        recipe([0.0, 1.0, 0.0], 4.0).recipe_fingerprint()
    );
    assert_ne!(
        r.recipe_fingerprint(),
        recipe([1.0, 0.0, 0.0], 3.0).recipe_fingerprint()
    );
    let policy = EvalPolicy::default();
    let pure = evaluate(&r, &policy).expect("evaluate");
    let mut cache = EvalCache::new();
    let cold = evaluate_with_cache(&r, &policy, &mut cache).expect("cold cache");
    let warm = evaluate_with_cache(&r, &policy, &mut cache).expect("warm cache");
    assert_eq!(warm.report.counters.cache_hits, 1);
    assert_eq!(warm.report.counters.tessellations, 0);
    for cached in [&cold, &warm] {
        assert_eq!(
            positions(&pure.bodies[0].body),
            positions(&cached.bodies[0].body)
        );
        assert_eq!(
            pure.bodies[0].body.sweep_checks,
            cached.bodies[0].body.sweep_checks
        );
        assert!(cached.bodies[0].body.sweep_checks.is_some());
    }
}

#[test]
fn collinear_runs_allow_unit_miter_limit_and_scaled_authored_directions() {
    let profile = asymmetric();
    let path = [[0.0; 3], [2.0, 4.0, 6.0], [4.0, 8.0, 12.0]];
    let mut reference = None;
    for x in [[1.0, 0.0, 0.0], [1e-300, 0.0, 0.0], [1e300, 0.0, 0.0]] {
        let body = tessellate_mitered_sweep(
            &profile,
            &Placement3::IDENTITY,
            &path,
            x,
            1.0,
            CapMode::Both,
            &EvalPolicy::default(),
        )
        .expect("straight runs need no miter stretch");
        assert_clean(&body);
        if let Some(reference) = &reference {
            assert_eq!(
                &positions(&body),
                reference,
                "orientation magnitude is irrelevant"
            );
        } else {
            reference = Some(positions(&body));
        }
    }
}

#[test]
fn evidence_does_not_claim_global_intersection_checks() {
    let profile = asymmetric();
    // The last run crosses the first at (0, 0, 2). Local miter checks cannot
    // certify this as a solid, even though mesh topology can be closed.
    let path = [
        [0.0; 3],
        [0.0, 0.0, 5.0],
        [5.0, 0.0, 5.0],
        [5.0, 0.0, -2.0],
        [-2.0, 0.0, -2.0],
        [-2.0, 0.0, 2.0],
        [2.0, 0.0, 2.0],
    ];
    let body = rail(&profile, &path).expect("locally valid, globally intersecting rail");
    assert_clean(&body);
    assert_eq!(body.sweep_checks.expect("local evidence only").bands, 6);
    let legacy = tessellate_sweep(
        &profile,
        &Placement3::IDENTITY,
        &path,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .expect("legacy unchecked sweep");
    assert_eq!(legacy.sweep_checks, None);
}

#[test]
fn evaluation_keeps_miter_failure_node_and_payload() {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(asymmetric());
    let root = b
        .add(NodeKind::Sweep {
            profile,
            path: Path3::MiteredPolyline {
                points: vec![[0.0; 3], [0.0, 0.0, 4.0], [3.0, 0.0, 4.0]],
                section_x: [1.0, 0.0, 0.0],
                miter_limit: 1.1,
            },
            caps: CapMode::Both,
        })
        .expect("structurally valid path");
    let r = b.finish(root).expect("recipe");
    let failure = evaluate(&r, &EvalPolicy::default()).expect_err("miter exceeds limit");
    assert_eq!(failure.node, root);
    assert!(matches!(
        failure.error,
        TessellateError::MiterLimitExceeded {
            point: 1,
            maximum: 1.1,
            ..
        }
    ));
}

#[test]
fn mesh_narrowing_must_not_silently_collapse_rail_walls() {
    let result = tessellate_mitered_sweep(
        &asymmetric(),
        &Placement3::translate(1e12, 0.0, 0.0),
        &[[0.0; 3], [0.0, 0.0, 4.0]],
        [1.0, 0.0, 0.0],
        2.0,
        CapMode::Both,
        &EvalPolicy::default(),
    );
    assert!(
        matches!(result, Err(TessellateError::CollapsedGeometry)),
        "f32 mesh lost the section width"
    );
}

#[test]
fn mesh_narrowing_must_not_silently_collapse_caps_with_distinct_edges() {
    let profile = Profile2::simple(
        crate::profile::Loop2::new(vec![
            crate::profile::Seg2::line((2.0, 0.0)),
            crate::profile::Seg2::line((1.0, 0.01)),
            crate::profile::Seg2::line((0.0, 0.0)),
        ])
        .expect("triangle"),
    )
    .expect("profile");
    let result = tessellate_mitered_sweep(
        &profile,
        &Placement3::translate(0.0, 1e8, 0.0),
        &[[0.0; 3], [0.0, 0.0, 4.0]],
        [1.0, 0.0, 0.0],
        2.0,
        CapMode::Both,
        &EvalPolicy::default(),
    );
    assert!(
        matches!(result, Err(TessellateError::CollapsedGeometry)),
        "cap becomes collinear although its edges remain distinct"
    );
}

#[test]
fn legacy_sweep_refuses_unrepresentable_triangulated_caps() {
    let result = tessellate_sweep(
        &builders::ring(1.0, 0.5).unwrap(),
        &Placement3::translate(1e12, 0.0, 0.0),
        &[[0.0; 3], [0.0, 0.0, 5.0]],
        CapMode::Both,
        &EvalPolicy::default(),
    );
    assert!(matches!(result, Err(TessellateError::CollapsedGeometry)));
}

#[test]
fn nearly_collinear_cap_triangulation_retries_without_losing_boundary() {
    // At this sampling the existing triangulator chooses an almost-collinear
    // triangle joining two outer vertices and one inner vertex. It has zero
    // area in f32. A different cover must retain the rim and positive cap area.
    let profile = builders::ring(1.0, 0.5).expect("ring");
    let body = rail(&profile, &[[0.0; 3], [0.0, 0.0, 5.0]]).expect("realizable cover");
    assert_clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    for face in body.mesh.faces() {
        let outward = match body.source_map.face_feature(face).unwrap() {
            Feature::CapStart => -1.0,
            Feature::CapEnd => 1.0,
            _ => continue,
        };
        let points: Vec<_> = body
            .mesh
            .face_loop(face)
            .map(|c| {
                body.mesh
                    .vertex_position(body.mesh.to_vertex(c).unwrap())
                    .unwrap()
                    .map(f64::from)
            })
            .collect();
        assert_eq!(points.len(), 3);
        assert!(cross(sub(points[1], points[0]), sub(points[2], points[0]))[2] * outward > 0.0);
    }
}
