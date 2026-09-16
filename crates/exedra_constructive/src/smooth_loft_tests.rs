// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::tests::{assert_clean, mesh_volume};
use super::*;
use crate::cache::{EvalCache, policy_fingerprint};
use crate::evaluate::{Fidelity, evaluate, evaluate_with_cache};
use crate::ir::{NodeKind, Recipe, RecipeBuilder};
use crate::profile::{Loop2, Seg2};
use alloc::vec;

fn section(x: f64, y: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((-0.5 * x, -0.3 * y)),
            Seg2::line((0.4 * x, -0.35 * y)),
            Seg2::line((0.65 * x, 0.05 * y)),
            Seg2::line((0.15 * x, 0.4 * y)),
            Seg2::line((-0.45 * x, 0.2 * y)),
        ])
        .unwrap(),
    )
    .unwrap()
}
fn recipe(mode: LoftPolicy) -> Recipe {
    let mut b = RecipeBuilder::new();
    let a = b.add_profile(section(1.0, 1.0));
    let middle = b.add_profile(section(1.5, 0.8));
    let c = b.add_profile(section(0.7, 1.2));
    let node = b
        .add(NodeKind::Loft {
            sections: vec![
                (Placement3::IDENTITY, a),
                (Placement3::translate(0.3, 0.1, 1.5), middle),
                (Placement3::translate(-0.1, 0.2, 3.0), c),
            ],
            policy: mode,
            caps: CapMode::Both,
        })
        .unwrap();
    b.finish(node).unwrap()
}

#[test]
fn asymmetric_smooth_loft_retains_sections_caps_and_source_bands() {
    let recipe = recipe(LoftPolicy::Smooth);
    let policy = EvalPolicy::default();
    let evaluation = evaluate(&recipe, &policy).unwrap();
    assert_eq!(
        evaluation.report.fidelity_of(recipe.root()),
        Some(Fidelity::Exact),
        "{:?}",
        evaluation.report.diagnostics
    );
    let body = &evaluation.bodies[0].body;
    assert_clean(body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    assert!(mesh_volume(&body.mesh) > 0.0);
    let sampling = body.loft_sampling.as_ref().unwrap();
    assert_eq!(sampling.section_vertices, 5);
    assert!(sampling.spans.len() > 2);
    assert_eq!(body.mesh.vertices().count(), (sampling.spans.len() + 1) * 5);
    let NodeKind::Loft { sections, .. } = &recipe.node(recipe.root()).unwrap().kind else {
        unreachable!()
    };
    let positions: Vec<_> = body
        .mesh
        .vertices()
        .map(|v| *body.mesh.vertex_position(v).unwrap())
        .collect();
    for (placement, profile) in sections {
        let discretized =
            discretize_profile(recipe.profile(*profile).unwrap(), &policy.discretize).unwrap();
        for p in discretized.outer.points {
            assert!(positions.contains(&narrow(apply_placement(placement, [p[0], p[1], 0.0]))));
        }
    }
    let mut caps = [0, 0];
    for face in body.mesh.faces() {
        match body.source_map.face_feature(face).unwrap() {
            Feature::CapStart => caps[0] += 1,
            Feature::CapEnd => caps[1] += 1,
            Feature::LoftWall { band, seg, .. } => {
                assert!(band < 2);
                assert!(seg < 5);
            }
            feature => panic!("unexpected {feature:?}"),
        }
        let mut triangles = Vec::new();
        assert!(!body.mesh.face_triangles_into(
            face,
            exedra_mesh::FaceTriangulation::Robust,
            &mut triangles
        ));
        assert!(!triangles.is_empty());
    }
    assert_eq!(caps, [3, 3]);
    // An intermediate horizontal ring is a sampling boundary, not a crease.
    for edge in body.mesh.faces().flat_map(|face| body.mesh.face_loop(face)) {
        let from = body.mesh.to_vertex(body.mesh.prev(edge).unwrap()).unwrap();
        let to = body.mesh.to_vertex(edge).unwrap();
        let a = body.mesh.vertex_position(from).unwrap();
        let b = body.mesh.vertex_position(to).unwrap();
        if a[2] == b[2] && a[2] > 0.0 && a[2] < 3.0 {
            assert_eq!(body.mesh.edge_sharpness(edge).unwrap_or(0.0), 0.0);
        }
    }
}

#[test]
fn smooth_loft_identity_wire_formats_and_cache_retain_policy_and_evidence() {
    let recipe = recipe(LoftPolicy::Smooth);
    let ruled = self::recipe(LoftPolicy::Ruled);
    assert_ne!(recipe.recipe_fingerprint(), ruled.recipe_fingerprint());
    let text = crate::text::dump_recipe(&recipe);
    assert!(text.contains("loft smooth"));
    assert_eq!(
        crate::text::parse_recipe(&text)
            .unwrap()
            .recipe_fingerprint(),
        recipe.recipe_fingerprint()
    );
    #[cfg(feature = "serde")]
    {
        let dto = crate::interchange::to_dto(&recipe);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("smooth"));
        let dto = serde_json::from_str(&json).unwrap();
        assert_eq!(
            crate::interchange::from_dto(&dto)
                .unwrap()
                .recipe_fingerprint(),
            recipe.recipe_fingerprint()
        );
    }
    let mut cache = EvalCache::new();
    let policy = EvalPolicy::default();
    let cold = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    let warm = evaluate_with_cache(&recipe, &policy, &mut cache).unwrap();
    assert_eq!(warm.report.counters.tessellations, 0);
    assert_eq!(
        cold.bodies[0].body.loft_sampling,
        warm.bodies[0].body.loft_sampling
    );
    for loft in [
        crate::loft::LoftSamplingPolicy {
            chord_tolerance: 0.001,
            ..policy.loft
        },
        crate::loft::LoftSamplingPolicy {
            max_band_edges: 12,
            ..policy.loft
        },
        crate::loft::LoftSamplingPolicy {
            max_vertices: 200,
            ..policy.loft
        },
    ] {
        assert_ne!(
            policy_fingerprint(&policy),
            policy_fingerprint(&EvalPolicy { loft, ..policy })
        );
    }
}

#[test]
fn authored_seams_preserve_geometry_and_restore_rotated_correspondence() {
    let profile = section(1.0, 1.0);
    let rotated = profile.outer().with_seam(2).unwrap();
    assert_eq!(rotated.with_seam(3).unwrap(), *profile.outer());
    assert!(profile.outer().with_seam(5).is_none());
    let a = Profile2::simple(rotated).unwrap();
    let b = Profile2::simple(a.outer().with_seam(3).unwrap()).unwrap();
    let sections = [
        (Placement3::IDENTITY, &profile),
        (Placement3::translate(0.0, 0.0, 2.0), &b),
    ];
    let smooth = tessellate_loft(
        &sections,
        LoftPolicy::Smooth,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let ruled = tessellate_loft(
        &sections,
        LoftPolicy::Ruled,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!((mesh_volume(&smooth.mesh) - mesh_volume(&ruled.mesh)).abs() < 1e-8);
    assert_eq!(smooth.loft_sampling.unwrap().spans.len(), 1);
}

#[test]
fn smooth_loft_refusals_are_typed_and_do_not_claim_exact_geometry() {
    let recipe = recipe(LoftPolicy::Smooth);
    let policy = EvalPolicy {
        loft: crate::loft::LoftSamplingPolicy {
            max_vertices: 10,
            ..crate::loft::LoftSamplingPolicy::default()
        },
        ..EvalPolicy::default()
    };
    assert!(matches!(
        evaluate(&recipe, &policy),
        Err(crate::evaluate::EvalError {
            error: TessellateError::Loft(crate::loft::LoftError::BudgetExceeded),
            ..
        })
    ));
    let p = section(1.0, 1.0);
    let backward = [
        (Placement3::IDENTITY, &p),
        (Placement3::translate(0.0, 0.0, 1.0), &p),
        (Placement3::IDENTITY, &p),
    ];
    assert!(matches!(
        tessellate_loft(
            &backward,
            LoftPolicy::Smooth,
            CapMode::Both,
            &EvalPolicy::default()
        ),
        Err(TessellateError::Loft(
            crate::loft::LoftError::Foldover { .. }
        ))
    ));
    let tiny = [
        (Placement3::translate(1e12, 0.0, 0.0), &p),
        (Placement3::translate(1e12, 0.0, 2.0), &p),
    ];
    let policy = EvalPolicy {
        loft: crate::loft::LoftSamplingPolicy {
            chord_tolerance: 1.0,
            ..crate::loft::LoftSamplingPolicy::default()
        },
        ..EvalPolicy::default()
    };
    assert!(matches!(
        tessellate_loft(&tiny, LoftPolicy::Smooth, CapMode::Both, &policy),
        Err(TessellateError::CollapsedGeometry)
    ));
}

#[test]
fn smooth_loft_degenerate_sections_never_claim_a_section_envelope() {
    // On the second band, right-side x is 2 + t*(1-t)^2/2, exceeding
    // every authored section's x bound. Even a refused loft cannot use
    // the section union as its interpolation envelope.
    for height in [0.0, 1e-15] {
        let mut builder = RecipeBuilder::new();
        let profile = builder.add_profile(crate::builders::rect(1.0, 1.0).unwrap());
        let root = builder
            .add(NodeKind::Loft {
                sections: vec![
                    (Placement3::IDENTITY, profile),
                    (Placement3::translate(1.0, 0.0, height), profile),
                    (Placement3::translate(1.0, 1.0, 2.0 * height), profile),
                ],
                policy: LoftPolicy::Smooth,
                caps: CapMode::Both,
            })
            .unwrap();
        let recipe = builder.finish(root).unwrap();
        assert!(matches!(
            evaluate(&recipe, &EvalPolicy::default()),
            Err(crate::evaluate::EvalError {
                error: TessellateError::DegenerateLoft,
                ..
            })
        ));
    }
}

#[test]
fn smooth_square_volume_converges_to_the_integrated_cubic_area() {
    let small = crate::builders::rect(1.0, 1.0).unwrap();
    let large = crate::builders::rect(2.0, 2.0).unwrap();
    let sections = [
        (Placement3::IDENTITY, &small),
        (Placement3::translate(0.0, 0.0, 1.0), &large),
        (Placement3::translate(0.0, 0.0, 2.0), &small),
    ];
    // On the first unit-height band, side length is 1+t+t²-t³.
    // The second band is its reflection. Integrating twice its squared
    // side length gives 548/105, independently of mesh triangulation.
    let expected = 548.0 / 105.0;
    let mut errors = Vec::new();
    for tolerance in [0.01, 0.0001] {
        let policy = EvalPolicy {
            loft: crate::loft::LoftSamplingPolicy {
                chord_tolerance: tolerance,
                ..crate::loft::LoftSamplingPolicy::default()
            },
            ..EvalPolicy::default()
        };
        let body = tessellate_loft(&sections, LoftPolicy::Smooth, CapMode::Both, &policy).unwrap();
        assert_clean(&body);
        assert!(body.mesh.boundary_loops().unwrap().is_empty());
        errors.push((mesh_volume(&body.mesh) - expected).abs());
    }
    assert!(errors[1] < errors[0] / 20.0, "volume errors {errors:?}");
    assert!(errors[1] < 0.001, "volume errors {errors:?}");
}

#[test]
fn smooth_holed_sections_support_all_cap_modes_and_refuse_structure_mismatch() {
    let a = crate::builders::ring(0.5, 0.2).unwrap();
    let b = crate::builders::ring(0.6, 0.25).unwrap();
    let c = crate::builders::ring(0.45, 0.15).unwrap();
    let sections = [
        (Placement3::IDENTITY, &a),
        (Placement3::translate(0.0, 0.0, 1.0), &b),
        (Placement3::translate(0.0, 0.0, 2.0), &c),
    ];
    for (caps, boundaries) in [
        (CapMode::Both, 0),
        (CapMode::Start, 2),
        (CapMode::End, 2),
        (CapMode::None, 4),
    ] {
        let body =
            tessellate_loft(&sections, LoftPolicy::Smooth, caps, &EvalPolicy::default()).unwrap();
        assert_clean(&body);
        assert_eq!(body.mesh.boundary_loops().unwrap().len(), boundaries);
    }
    let rectangle = crate::builders::rect(1.0, 1.0).unwrap();
    let mismatched = [
        (Placement3::IDENTITY, &a),
        (Placement3::translate(0.0, 0.0, 2.0), &rectangle),
    ];
    assert!(matches!(
        tessellate_loft(
            &mismatched,
            LoftPolicy::Smooth,
            CapMode::Both,
            &EvalPolicy::default()
        ),
        Err(TessellateError::SectionMismatch { section: 1 })
    ));
}
