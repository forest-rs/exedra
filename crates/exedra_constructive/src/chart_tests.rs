// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    builders,
    cache::EvalCache,
    discretize::discretize_profile,
    evaluate::evaluate_with_cache,
    ir::{CapMode, NodeKind, Placement3, Recipe, RecipeBuilder, RecipeError},
    profile::{Loop2, Profile2, Seg2},
    tessellate::{
        EvalPolicy, Feature, TessellateError, TessellatedBody, tessellate_extrude,
        tessellate_extrude_with_chart, tessellate_revolve_with_chart,
    },
};
use alloc::{vec, vec::Vec};
use exedra_mesh::{FaceId, attr};

fn extrusion() -> SurfaceChart {
    SurfaceChart::Extrude {
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    }
}
fn revolution() -> SurfaceChart {
    SurfaceChart::Revolve {
        reference_radius: 2.0,
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    }
}
fn corners(body: &TessellatedBody, face: FaceId) -> Vec<([f64; 3], [f64; 2])> {
    let uv = body.mesh.attrs().sparse(attr::CORNER_UV).unwrap();
    body.mesh
        .face_loop(face)
        .map(|he| {
            (
                body.mesh
                    .vertex_position(body.mesh.to_vertex(he).unwrap())
                    .unwrap()
                    .map(f64::from),
                uv.get(he.as_id()).unwrap().map(f64::from),
            )
        })
        .collect()
}
fn clean(body: &TessellatedBody) {
    assert!(body.mesh.validate_deep().is_empty());
    body.source_map.check(&body.mesh).unwrap();
    for face in body.mesh.faces() {
        assert!(body.source_map.chart_sampling(face).is_some());
        assert!(
            corners(body, face)
                .iter()
                .all(|(_, uv)| uv.iter().all(|x| x.is_finite()))
        );
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
fn recipe(chart: SurfaceChart) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect_from_corner(2.0, 1.0).unwrap());
    let source = b.source_ref("authored-panel");
    let root = b
        .with_source(source)
        .with_surface_chart(chart)
        .add(NodeKind::Extrude {
            profile,
            height: 3.0,
            caps: CapMode::Both,
            placement: Placement3::IDENTITY,
        })
        .unwrap();
    b.finish(root).unwrap()
}

#[test]
fn extrusion_has_measured_walls_planar_caps_and_an_authored_seam() {
    let p = builders::rect_from_corner(2.0, 1.0).unwrap();
    let body = tessellate_extrude_with_chart(
        &p,
        &Placement3::IDENTITY,
        3.0,
        CapMode::Both,
        extrusion(),
        &EvalPolicy::default(),
    )
    .unwrap();
    clean(&body);
    let mut seam_values = Vec::new();
    for face in body.mesh.faces() {
        let feature = body.source_map.face_feature(face).unwrap();
        for (p, uv) in corners(&body, face) {
            match feature {
                Feature::Wall { seg, .. } => {
                    let s = match seg {
                        0 => p[0],
                        1 => 2.0 + p[1],
                        2 => 5.0 - p[0],
                        3 => 6.0 - p[1],
                        _ => unreachable!(),
                    };
                    near(uv[0], s);
                    near(uv[1], p[2]);
                    if p == [0.0; 3] {
                        seam_values.push(uv[0]);
                    }
                }
                Feature::CapStart | Feature::CapEnd => {
                    near(uv[0], p[0]);
                    near(uv[1], p[1]);
                }
                _ => panic!("unexpected feature"),
            }
        }
        let sampling = body.source_map.chart_sampling(face).unwrap();
        assert_eq!(sampling.loop_lengths, [6.0]);
    }
    seam_values.sort_by(f64::total_cmp);
    assert_eq!(seam_values, [0.0, 6.0]);
    let plain = tessellate_extrude(
        &p,
        &Placement3::IDENTITY,
        3.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    assert!(plain.mesh.attrs().sparse(attr::CORNER_UV).is_none());
    assert_eq!(plain.mesh.vertices().count(), body.mesh.vertices().count());
    assert_eq!(plain.mesh.faces().count(), body.mesh.faces().count());
    assert_eq!(
        plain
            .mesh
            .vertices()
            .map(|v| plain.mesh.vertex_position(v))
            .collect::<Vec<_>>(),
        body.mesh
            .vertices()
            .map(|v| body.mesh.vertex_position(v))
            .collect::<Vec<_>>()
    );
}

#[test]
fn moving_the_profile_seam_moves_the_uv_discontinuity_without_geometry_changes() {
    let p = builders::rect_from_corner(2.0, 1.0).unwrap();
    let p = Profile2::new(p.outer().with_seam(1).unwrap(), vec![]).unwrap();
    let body = tessellate_extrude_with_chart(
        &p,
        &Placement3::IDENTITY,
        3.0,
        CapMode::Both,
        extrusion(),
        &EvalPolicy::default(),
    )
    .unwrap();
    let zero_positions: Vec<_> = body
        .mesh
        .faces()
        .filter(|&f| matches!(body.source_map.face_feature(f), Some(Feature::Wall { .. })))
        .flat_map(|f| corners(&body, f))
        .filter(|(_, uv)| uv[0] == 0.0)
        .map(|(p, _)| p)
        .collect();
    assert!(!zero_positions.is_empty());
    assert!(zero_positions.iter().all(|p| p[0] == 2.0 && p[1] == 0.0));
}

#[test]
fn reflected_placement_keeps_coordinates_at_the_same_material_points() {
    let profile = builders::rect_from_corner(2.0, 1.0).unwrap();
    let chart = SurfaceChart::Extrude {
        wall: ChartTransform {
            matrix: [[0.0, 0.5], [-2.0, 0.0]],
            offset: [0.125, -0.25],
        },
        caps: ChartTransform::IDENTITY,
    };
    let policy = EvalPolicy::default();
    let original = tessellate_extrude_with_chart(
        &profile,
        &Placement3::IDENTITY,
        3.0,
        CapMode::Both,
        chart,
        &policy,
    )
    .unwrap();
    let reflected = tessellate_extrude_with_chart(
        &profile,
        &Placement3 {
            rows: [
                [-2.0, 0.0, 0.0, 7.0],
                [0.0, 3.0, 0.0, -4.0],
                [0.0, 0.0, 0.5, 1.0],
            ],
        },
        3.0,
        CapMode::Both,
        chart,
        &policy,
    )
    .unwrap();
    clean(&reflected);
    for (a, b) in original.mesh.faces().zip(reflected.mesh.faces()) {
        let placed = corners(&reflected, b);
        for (p, uv) in corners(&original, a) {
            let expected = [7.0 - 2.0 * p[0], 3.0 * p[1] - 4.0, 0.5 * p[2] + 1.0];
            assert!(
                placed
                    .iter()
                    .any(|&(q, mapped)| q == expected && mapped == uv)
            );
        }
    }
}

#[test]
fn sampled_circle_and_hole_lengths_are_reported_without_an_analytic_claim() {
    let profile = builders::ring(3.0, 1.0).unwrap();
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.1;
    let d = discretize_profile(&profile, &policy.discretize).unwrap();
    let body = tessellate_extrude_with_chart(
        &profile,
        &Placement3::IDENTITY,
        2.0,
        CapMode::Both,
        extrusion(),
        &policy,
    )
    .unwrap();
    clean(&body);
    let sampling = body
        .source_map
        .chart_sampling(body.mesh.faces().next().unwrap())
        .unwrap();
    for (i, (ring, radius)) in [(&d.outer, 3.0), (&d.holes[0], 1.0)]
        .into_iter()
        .enumerate()
    {
        let n = f64::from(u32::try_from(ring.points.len()).unwrap());
        near(
            sampling.loop_lengths[i],
            2.0 * radius * n * libm::sin(core::f64::consts::PI / n),
        );
        assert!(sampling.loop_lengths[i] < core::f64::consts::TAU * radius);
    }
    assert_eq!(sampling.profile, policy.discretize);
    for face in body.mesh.faces() {
        if matches!(
            body.source_map.face_feature(face),
            Some(Feature::CapStart | Feature::CapEnd)
        ) {
            for (p, uv) in corners(&body, face) {
                near(p[0], uv[0]);
                near(p[1], uv[1]);
            }
        }
    }
}

fn vessel_profile(pole: bool) -> Profile2 {
    let inner = if pole { 0.0 } else { 1.0 };
    Profile2::new(
        Loop2::new(vec![
            Seg2::line((3.0, 0.0)),
            Seg2::line((3.0, 2.0)),
            Seg2::line((inner, 2.0)),
            Seg2::line((inner, 0.0)),
        ])
        .unwrap(),
        vec![],
    )
    .unwrap()
}
#[test]
fn revolution_metric_matches_angle_and_profile_length_including_seams_and_poles() {
    for pole in [false, true] {
        let profile = vessel_profile(pole);
        for sweep in [core::f64::consts::TAU, core::f64::consts::FRAC_PI_2] {
            let body = tessellate_revolve_with_chart(
                &profile,
                &Placement3::IDENTITY,
                sweep,
                CapMode::Both,
                revolution(),
                &EvalPolicy::default(),
            )
            .unwrap();
            clean(&body);
            let inner = if pole { 0.0 } else { 1.0 };
            let width = 3.0 - inner;
            let mut end_uv = false;
            for face in body.mesh.faces() {
                for (p, uv) in corners(&body, face) {
                    let r = libm::hypot(p[0], p[2]);
                    match body.source_map.face_feature(face).unwrap() {
                        Feature::Wall { seg, .. } => {
                            let theta = uv[0] / 2.0;
                            near(p[0], r * libm::cos(theta));
                            near(p[2], -r * libm::sin(theta));
                            let s = match seg {
                                0 => r - inner,
                                1 => width + p[1],
                                2 => width + 2.0 + 3.0 - r,
                                3 => width * 2.0 + 4.0 - p[1],
                                _ => unreachable!(),
                            };
                            near(uv[1], s);
                            end_uv |= (uv[0] - sweep * 2.0).abs() < 2e-5;
                        }
                        Feature::CapStart | Feature::CapEnd => {
                            near(uv[0], r);
                            near(uv[1], p[1]);
                        }
                        _ => panic!("unexpected feature"),
                    }
                }
            }
            assert!(end_uv);
            assert!(body.mesh.boundary_loops().unwrap().is_empty());
        }
    }
}

#[test]
fn chart_intent_roundtrips_and_warm_cache_retains_uvs_and_ancestry() {
    let original = recipe(extrusion());
    let dump = crate::text::dump_recipe(&original);
    assert!(dump.contains("charted extrude"));
    let parsed = crate::text::parse_recipe(&dump).unwrap();
    assert_eq!(original.nodes(), parsed.nodes(), "{dump}");
    assert_eq!(original.recipe_fingerprint(), parsed.recipe_fingerprint());
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&crate::interchange::to_dto(&original)).unwrap();
        assert!(json.contains("\"op\":\"charted\""));
        let roundtrip =
            crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(
            original.recipe_fingerprint(),
            roundtrip.recipe_fingerprint()
        );
    }
    let mut cache = EvalCache::new();
    for warm in [false, true] {
        let output = evaluate_with_cache(&parsed, &EvalPolicy::default(), &mut cache).unwrap();
        if warm {
            assert_eq!(output.report.counters.tessellations, 0);
        }
        let body = &output.bodies[0].body;
        clean(body);
        for face in body.mesh.faces() {
            assert_eq!(
                body.source_map
                    .surface_origin(face)
                    .unwrap()
                    .source
                    .as_deref(),
                Some("authored-panel")
            );
        }
    }
    let changed = recipe(SurfaceChart::Extrude {
        wall: ChartTransform {
            offset: [0.5, 0.0],
            ..ChartTransform::IDENTITY
        },
        caps: ChartTransform::IDENTITY,
    });
    assert_ne!(original.recipe_fingerprint(), changed.recipe_fingerprint());
}

#[test]
fn invalid_metrics_and_uv_overflow_fail_explicitly() {
    let mut builder = RecipeBuilder::new();
    let profile = builder.add_profile(builders::rect_from_corner(2.0, 1.0).unwrap());
    let kind = NodeKind::Extrude {
        profile,
        placement: Placement3::IDENTITY,
        height: 3.0,
        caps: CapMode::Both,
    };
    assert_eq!(
        builder.with_surface_chart(revolution()).add(kind.clone()),
        Err(RecipeError::SurfaceChart(ChartError::WrongOperation))
    );
    let invalid = SurfaceChart::Extrude {
        wall: ChartTransform {
            matrix: [[1.0, 2.0], [2.0, 4.0]],
            offset: [0.0; 2],
        },
        caps: ChartTransform::IDENTITY,
    };
    assert_eq!(
        builder.with_surface_chart(invalid).add(kind),
        Err(RecipeError::SurfaceChart(ChartError::InvalidTransform))
    );
    let p = builders::rect_from_corner(2.0, 1.0).unwrap();
    let huge = SurfaceChart::Extrude {
        wall: ChartTransform {
            matrix: [[1e100, 0.0], [0.0, 1.0]],
            offset: [0.0; 2],
        },
        caps: ChartTransform::IDENTITY,
    };
    assert!(matches!(
        tessellate_extrude_with_chart(
            &p,
            &Placement3::IDENTITY,
            3.0,
            CapMode::Both,
            huge,
            &EvalPolicy::default()
        ),
        Err(TessellateError::Chart(ChartError::NumericLimit))
    ));
}

#[test]
fn refined_cap_interior_vertices_receive_the_planar_chart() {
    let p = builders::circle(3.0).unwrap();
    let policy =
        EvalPolicy::default().with_cap_refinement(exedra_triangulate::RefineParams::default());
    let body = tessellate_extrude_with_chart(
        &p,
        &Placement3::IDENTITY,
        2.0,
        CapMode::Both,
        extrusion(),
        &policy,
    )
    .unwrap();
    clean(&body);
    assert!(body.refinement.is_some());
    let mut interior = false;
    for face in body.mesh.faces() {
        if matches!(
            body.source_map.face_feature(face),
            Some(Feature::CapStart | Feature::CapEnd)
        ) {
            for (p, uv) in corners(&body, face) {
                near(p[0], uv[0]);
                near(p[1], uv[1]);
                interior |= libm::hypot(p[0], p[1]) < 2.0;
            }
        }
    }
    assert!(interior);
}

#[test]
fn retained_revolution_chart_roundtrips_all_metric_fields() {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(vessel_profile(false));
    let chart = SurfaceChart::Revolve {
        reference_radius: 1.3,
        wall: ChartTransform {
            matrix: [[0.0, -0.3], [2.1, 0.0]],
            offset: [0.17, -0.29],
        },
        caps: ChartTransform {
            offset: [0.23, 0.39],
            ..ChartTransform::IDENTITY
        },
    };
    let root = b
        .with_surface_chart(chart)
        .add(NodeKind::Revolve {
            profile,
            placement: Placement3::IDENTITY,
            sweep: core::f64::consts::TAU,
            caps: CapMode::Both,
        })
        .unwrap();
    let recipe = b.finish(root).unwrap();
    let parsed = crate::text::parse_recipe(&crate::text::dump_recipe(&recipe)).unwrap();
    assert_eq!(recipe.recipe_fingerprint(), parsed.recipe_fingerprint());
    #[cfg(feature = "serde")]
    {
        let json = serde_json::to_string(&crate::interchange::to_dto(&recipe)).unwrap();
        let decoded = crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(recipe.recipe_fingerprint(), decoded.recipe_fingerprint());
    }
    let mut cache = EvalCache::new();
    for _ in 0..2 {
        let output = evaluate_with_cache(&parsed, &EvalPolicy::default(), &mut cache).unwrap();
        clean(&output.bodies[0].body);
        let body = &output.bodies[0].body;
        assert_eq!(
            body.source_map
                .chart_sampling(body.mesh.faces().next().unwrap())
                .unwrap()
                .chart,
            chart
        );
    }
}

#[test]
fn retained_mirror_preserves_chart_ancestry_and_corner_coordinates() {
    use crate::ir::Plane3;
    let recipe = recipe(extrusion());
    let reflected = recipe
        .mirrored(Plane3 {
            normal: [1.0, 0.0, 0.0],
            distance: 0.0,
        })
        .unwrap();
    let original = crate::evaluate::evaluate(&recipe, &EvalPolicy::default()).unwrap();
    let output = crate::evaluate::evaluate(&reflected, &EvalPolicy::default()).unwrap();
    let a = &original.bodies[0].body;
    let b = &output.bodies[0].body;
    clean(b);
    for (af, bf) in a.mesh.faces().zip(b.mesh.faces()) {
        let placed = corners(b, bf);
        for (p, uv) in corners(a, af) {
            assert!(
                placed
                    .iter()
                    .any(|&(q, mapped)| q == [-p[0], p[1], p[2]] && uv == mapped)
            );
        }
    }
}

#[test]
fn charted_extrusion_stretch_uses_the_mapped_mesh_path() {
    use crate::ir::Plane3;
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect_from_corner(2.0, 1.0).unwrap());
    let child = b
        .with_surface_chart(extrusion())
        .add(NodeKind::Extrude {
            profile,
            placement: Placement3::IDENTITY,
            height: 3.0,
            caps: CapMode::Both,
        })
        .unwrap();
    let root = b
        .add(NodeKind::Stretch {
            child,
            plane: Plane3 {
                normal: [0.0, 0.0, 1.0],
                distance: 1.5,
            },
            length: 1.0,
        })
        .unwrap();
    let output =
        crate::evaluate::evaluate(&b.finish(root).unwrap(), &EvalPolicy::default()).unwrap();
    assert_eq!(output.report.counters.stretch_exact, 0);
    assert_eq!(output.bodies.len(), 1);
    let body = &output.bodies[0].body;
    assert!(body.mesh.validate_deep().is_empty());
    for face in body.mesh.faces() {
        assert!(!corners(body, face).is_empty());
    }
}

#[test]
fn finite_uv_rounding_collapse_is_a_numeric_refusal() {
    let p = builders::rect_from_corner(2.0, 1.0).unwrap();
    for wall in [
        ChartTransform {
            matrix: [[1e-50, 0.0], [0.0, 1.0]],
            offset: [0.0; 2],
        },
        ChartTransform {
            offset: [1e20; 2],
            ..ChartTransform::IDENTITY
        },
        ChartTransform {
            matrix: [[1.0, 1.0], [1.0, 1.0 + 1e-12]],
            offset: [0.0; 2],
        },
    ] {
        let chart = SurfaceChart::Extrude {
            wall,
            caps: ChartTransform::IDENTITY,
        };
        chart.validate().unwrap();
        assert!(matches!(
            tessellate_extrude_with_chart(
                &p,
                &Placement3::IDENTITY,
                3.0,
                CapMode::Both,
                chart,
                &EvalPolicy::default()
            ),
            Err(TessellateError::Chart(ChartError::NumericLimit))
        ));
    }
}

#[test]
fn cap_rounding_cannot_hide_a_flipped_alternative_diagonal() {
    let p = builders::rect_from_corner(2.0, 3.0).unwrap();
    let caps = ChartTransform {
        matrix: [[1.0, -1.0], [1.0, -1.0 + 2.3083567486870317e-8]],
        offset: [0.5642887819047453, 0.22567397273930734],
    };
    let chart = SurfaceChart::Extrude {
        wall: ChartTransform::IDENTITY,
        caps,
    };
    for placement in [
        Placement3::IDENTITY,
        Placement3 {
            rows: [
                [-1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        },
    ] {
        assert!(matches!(
            tessellate_extrude_with_chart(
                &p,
                &placement,
                1.0,
                CapMode::End,
                chart,
                &EvalPolicy::default()
            ),
            Err(TessellateError::Chart(ChartError::NumericLimit))
        ));
    }
}

#[test]
fn underflowed_angular_metric_cannot_succeed_as_a_collapsed_chart() {
    let p = builders::rect_from_corner(2.0, 1.0).unwrap();
    let chart = SurfaceChart::Revolve {
        reference_radius: f64::from_bits(1),
        wall: ChartTransform {
            matrix: [[1e308, 0.0], [0.0, 1.0]],
            offset: [0.0; 2],
        },
        caps: ChartTransform::IDENTITY,
    };
    assert!(matches!(
        tessellate_revolve_with_chart(
            &p,
            &Placement3::IDENTITY,
            0.1,
            CapMode::Both,
            chart,
            &EvalPolicy::default()
        ),
        Err(TessellateError::Chart(ChartError::NumericLimit))
    ));
}

#[path = "chart_loft_sweep_tests.rs"]
mod loft_sweep;
