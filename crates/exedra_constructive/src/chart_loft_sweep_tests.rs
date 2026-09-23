// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    ir::{FramePolicy, LoftPolicy, LoftSection, Path3, PathClosure, PathJoin},
    path::{PathSegment3, discretize_path},
    tessellate::{tessellate_loft, tessellate_loft_with_chart, tessellate_sweep_with_chart},
};

fn loft(reference_section: u32, rest_length: f64) -> SurfaceChart {
    SurfaceChart::Loft {
        reference_section,
        rest_length,
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    }
}
fn sweep() -> SurfaceChart {
    SurfaceChart::Sweep {
        wall: ChartTransform::IDENTITY,
        caps: ChartTransform::IDENTITY,
    }
}
fn geometry_eq(a: &TessellatedBody, b: &TessellatedBody) {
    assert_eq!(a.mesh.vertices().count(), b.mesh.vertices().count());
    assert_eq!(a.mesh.faces().count(), b.mesh.faces().count());
    for (af, bf) in a.mesh.faces().zip(b.mesh.faces()) {
        assert_eq!(a.source_map.face_feature(af), b.source_map.face_feature(bf));
        let positions = |body: &TessellatedBody, face| {
            body.mesh
                .face_loop(face)
                .map(|he| {
                    *body
                        .mesh
                        .vertex_position(body.mesh.to_vertex(he).unwrap())
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(positions(a, af), positions(b, bf));
    }
}
fn assert_wall_continuity(body: &TessellatedBody) {
    // A vertex may have two values only at the profile seam or the closed path
    // seam. At all other edges, corners must agree regardless of face ordering.
    for vertex in body.mesh.vertices() {
        let mut us = Vec::new();
        let mut vs = Vec::new();
        for face in body.mesh.faces() {
            if !matches!(
                body.source_map.face_feature(face),
                Some(Feature::LoftWall { .. } | Feature::SweepWall { .. })
            ) {
                continue;
            }
            let chart = body.source_map.chart_sampling(face).unwrap();
            let end = *chart.station_distances.last().unwrap();
            let layer = body.mesh.attrs().sparse(attr::CORNER_UV).unwrap();
            for edge in body.mesh.face_loop(face) {
                if body.mesh.to_vertex(edge) == Some(vertex) {
                    let uv = layer.get(edge.as_id()).unwrap();
                    us.push(uv[0]);
                    vs.push(uv[1]);
                }
            }
            if vs.iter().any(|v| *v != vs[0]) {
                assert!(
                    vs.iter()
                        .all(|v| *v == 0.0 || (f64::from(*v) - end).abs() < 2e-5)
                );
            }
        }
        us.sort_by(f32::total_cmp);
        us.dedup();
        if us.len() > 1 {
            assert_eq!(us[0], 0.0, "unintended profile seam");
        }
        assert!(us.len() <= 2);
    }
}

#[test]
fn loft_rest_metric_uses_the_selected_profile_and_uniform_parameter_not_placed_length() {
    let profiles = [
        builders::rect_from_corner(1.0, 0.5).unwrap(),
        builders::rect_from_corner(2.0, 1.0).unwrap(),
        builders::rect_from_corner(3.0, 1.5).unwrap(),
    ];
    let sections = [
        (Placement3::IDENTITY, &profiles[0]),
        (Placement3::translate(0.0, 0.0, 2.0), &profiles[1]),
        (Placement3::translate(0.0, 0.0, 5.0), &profiles[2]),
    ];
    for interpolation in [LoftPolicy::Ruled, LoftPolicy::Smooth] {
        let policy = EvalPolicy::default();
        let body = tessellate_loft_with_chart(
            &sections,
            interpolation,
            CapMode::Both,
            loft(1, 12.0),
            &policy,
        )
        .unwrap();
        clean(&body);
        geometry_eq(
            &body,
            &tessellate_loft(&sections, interpolation, CapMode::Both, &policy).unwrap(),
        );
        let mut middle = 0;
        for face in body.mesh.faces() {
            let sampling = body.source_map.chart_sampling(face).unwrap();
            assert_eq!(sampling.loop_lengths, [6.0]);
            assert_eq!(sampling.station_distances[0], 0.0);
            assert_eq!(*sampling.station_distances.last().unwrap(), 12.0);
            for (p, uv) in corners(&body, face) {
                match body.source_map.face_feature(face).unwrap() {
                    Feature::CapStart | Feature::CapEnd => {
                        near(uv[0], p[0]);
                        near(uv[1], p[1]);
                    }
                    Feature::LoftWall { seg, .. } => {
                        if p[2] == 2.0 {
                            near(uv[1], 6.0);
                            middle += 1;
                        }
                        if p[0] == 0.0 && p[1] == 0.0 {
                            near(uv[0], if seg == 3 { 6.0 } else { 0.0 });
                        }
                        if p[1] == 0.0 && p[0] != 0.0 {
                            near(uv[0], 2.0);
                        }
                    }
                    _ => panic!("unexpected feature"),
                }
            }
        }
        assert!(middle > 0);
        assert_wall_continuity(&body);
    }
}

fn arc_path(closed: bool) -> Path3 {
    Path3::Curves {
        start: [3.0, 0.0, 0.0],
        segments: vec![PathSegment3::Arc {
            axis_origin: [0.0; 3],
            axis: [0.0, 0.0, 1.0],
            sweep: if closed {
                core::f64::consts::TAU
            } else {
                core::f64::consts::FRAC_PI_2
            },
        }],
        section_x: [1.0, 0.0, 0.0],
        section_origin: [0.1, 0.2],
        closure: if closed {
            PathClosure::ClosedPlanar {
                normal: [0.0, 0.0, 1.0],
            }
        } else {
            PathClosure::Open
        },
        joins: PathJoin::Smooth,
    }
}

#[test]
fn sweep_centerline_metric_is_preplacement_and_independent_of_offset_rail_lengths() {
    let profile = builders::rect_from_corner(0.2, 0.4).unwrap();
    let path = arc_path(false);
    let policy = EvalPolicy::default();
    let body = tessellate_sweep_with_chart(
        &profile,
        &Placement3::IDENTITY,
        &path,
        CapMode::Both,
        sweep(),
        &policy,
    )
    .unwrap();
    clean(&body);
    let Path3::Curves {
        start,
        segments,
        closure,
        joins,
        ..
    } = &path
    else {
        unreachable!()
    };
    let sampled = discretize_path(*start, segments, *closure, *joins, &policy.sweep_path).unwrap();
    let steps = sampled.stations.len() - 1;
    let expected = 2.0
        * 3.0
        * libm::sin(core::f64::consts::FRAC_PI_2 / (2.0 * f64::from(crate::len_u32(steps))))
        * f64::from(crate::len_u32(steps));
    for face in body.mesh.faces() {
        let sampling = body.source_map.chart_sampling(face).unwrap();
        near(*sampling.station_distances.last().unwrap(), expected);
        if matches!(
            body.source_map.face_feature(face),
            Some(Feature::CapStart | Feature::CapEnd)
        ) {
            for (p, uv) in corners(&body, face) {
                near(uv[0], libm::hypot(p[0], p[1]) - 2.9);
                near(uv[1], 0.2 - p[2]);
            }
        }
        if let Some(Feature::SweepWall { band, .. }) = body.source_map.face_feature(face) {
            for (_, uv) in corners(&body, face) {
                assert!(
                    sampling.station_distances[usize::from(band)..=usize::from(band) + 1]
                        .iter()
                        .any(|v| (uv[1] - v).abs() < 2e-5)
                );
            }
        }
    }
    assert_wall_continuity(&body);
    let mirror = Placement3 {
        rows: [
            [-2.0, 0.0, 0.0, 7.0],
            [0.0, 3.0, 0.0, -4.0],
            [0.0, 0.0, 0.5, 1.0],
        ],
    };
    let mirrored =
        tessellate_sweep_with_chart(&profile, &mirror, &path, CapMode::Both, sweep(), &policy)
            .unwrap();
    clean(&mirrored);
    for (a, b) in body.mesh.faces().zip(mirrored.mesh.faces()) {
        let placed = corners(&mirrored, b);
        for (p, uv) in corners(&body, a) {
            let expected = [7.0 - 2.0 * p[0], 3.0 * p[1] - 4.0, 0.5 * p[2] + 1.0];
            assert!(placed.iter().any(|(q, mapped)| {
                q.iter().zip(expected).all(|(x, y)| (x - y).abs() < 2e-6) && *mapped == uv
            }));
        }
    }
}

#[test]
fn closed_sweeps_have_only_authored_profile_and_path_seams_including_holes() {
    let profile = builders::ring(0.2, 0.1).unwrap();
    let polyline = Path3::MiteredPolyline {
        points: vec![[0.0; 3], [3.0, 0.0, 0.0], [3.0, 2.0, 0.0], [0.0, 2.0, 0.0]],
        section_x: [0.0, 0.0, 1.0],
        section_origin: [0.03, -0.04],
        closure: PathClosure::ClosedPlanar {
            normal: [0.0, 0.0, 1.0],
        },
        miter_limit: 2.0,
    };
    for path in [polyline, arc_path(true)] {
        let body = tessellate_sweep_with_chart(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::None,
            sweep(),
            &EvalPolicy::default(),
        )
        .unwrap();
        clean(&body);
        assert!(body.mesh.boundary_loops().unwrap().is_empty());
        assert_wall_continuity(&body);
        for ring in [0, 1] {
            let mut zeros = 0;
            let mut ends = 0;
            for face in body.mesh.faces() {
                let Some(Feature::SweepWall { loop_index, .. }) =
                    body.source_map.face_feature(face)
                else {
                    panic!("no caps expected")
                };
                if loop_index != ring {
                    continue;
                }
                let sampling = body.source_map.chart_sampling(face).unwrap();
                let end = *sampling.station_distances.last().unwrap();
                assert_eq!(sampling.loop_lengths.len(), 2);
                for (_, uv) in corners(&body, face) {
                    zeros += usize::from(uv[1] == 0.0);
                    ends += usize::from((uv[1] - end).abs() < 2e-5);
                }
            }
            assert!(zeros > 0 && zeros == ends);
        }
    }
}

fn retained(chart: SurfaceChart, smooth: bool) -> Recipe {
    let mut b = RecipeBuilder::new();
    let profile = b.add_profile(builders::rect_from_corner(0.3, 0.1).unwrap());
    let source = b.source_ref("draped-strip");
    let kind = if matches!(chart, SurfaceChart::Loft { .. }) {
        NodeKind::Loft {
            sections: vec![
                LoftSection::new(Placement3::IDENTITY, profile),
                LoftSection::new(Placement3::translate(0.2, 0.0, 1.0), profile),
                LoftSection::new(Placement3::translate(0.0, 0.0, 2.0), profile),
            ],
            policy: if smooth {
                LoftPolicy::Smooth
            } else {
                LoftPolicy::Ruled
            },
            caps: CapMode::Both,
        }
    } else {
        NodeKind::Sweep {
            section: crate::ir::SectionLaw::IDENTITY,
            profile,
            path: arc_path(false),
            caps: CapMode::Both,
        }
    };
    let root = b
        .with_source(source)
        .with_surface_chart(chart)
        .add(kind)
        .unwrap();
    b.finish(root).unwrap()
}

#[test]
fn retained_loft_and_sweep_metrics_roundtrip_and_warm_cache_without_losing_evidence() {
    for chart in [loft(1, 9.3), sweep()] {
        let original = retained(chart, true);
        let parsed = crate::text::parse_recipe(&crate::text::dump_recipe(&original)).unwrap();
        assert_eq!(original.recipe_fingerprint(), parsed.recipe_fingerprint());
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string(&crate::interchange::to_dto(&original)).unwrap();
            let decoded =
                crate::interchange::from_dto(&serde_json::from_str(&json).unwrap()).unwrap();
            assert_eq!(original.recipe_fingerprint(), decoded.recipe_fingerprint());
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
                let evidence = body.source_map.chart_sampling(face).unwrap();
                assert_eq!(evidence.chart, chart);
                assert!(!evidence.station_distances.is_empty());
                assert_eq!(
                    body.source_map
                        .surface_origin(face)
                        .unwrap()
                        .source
                        .as_deref(),
                    Some("draped-strip")
                );
            }
        }
    }
    let original = retained(loft(1, 9.3), true).recipe_fingerprint();
    assert_ne!(original, retained(loft(0, 9.3), true).recipe_fingerprint());
    assert_ne!(original, retained(loft(1, 9.4), true).recipe_fingerprint());
}

#[test]
fn invalid_loft_metrics_reference_and_chart_rounding_fail_typed() {
    let profile = builders::rect_from_corner(1.0, 0.5).unwrap();
    let sections = [
        (Placement3::IDENTITY, &profile),
        (Placement3::translate(0.0, 0.0, 1.0), &profile),
    ];
    for length in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            tessellate_loft_with_chart(
                &sections,
                LoftPolicy::Ruled,
                CapMode::Both,
                loft(0, length),
                &EvalPolicy::default()
            ),
            Err(TessellateError::Chart(ChartError::InvalidRestLength))
        ));
    }
    assert!(matches!(
        tessellate_loft_with_chart(
            &sections,
            LoftPolicy::Ruled,
            CapMode::Both,
            loft(2, 1.0),
            &EvalPolicy::default()
        ),
        Err(TessellateError::Chart(
            ChartError::InvalidReferenceSection { section: 2 }
        ))
    ));
    let mut b = RecipeBuilder::new();
    let p = b.add_profile(profile.clone());
    assert!(matches!(
        b.with_surface_chart(loft(2, 1.0)).add(NodeKind::Loft {
            sections: vec![
                LoftSection::new(sections[0].0, p),
                LoftSection::new(sections[1].0, p)
            ],
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both
        }),
        Err(RecipeError::SurfaceChart(
            ChartError::InvalidReferenceSection { section: 2 }
        ))
    ));
    let chart = SurfaceChart::Sweep {
        wall: ChartTransform {
            offset: [1e30, 0.0],
            ..ChartTransform::IDENTITY
        },
        caps: ChartTransform::IDENTITY,
    };
    let path = Path3::Polyline {
        points: vec![[0.0; 3], [0.0, 0.0, 1.0]],
        frame: FramePolicy::RotationMinimizing,
    };
    assert!(matches!(
        tessellate_sweep_with_chart(
            &profile,
            &Placement3::IDENTITY,
            &path,
            CapMode::Both,
            chart,
            &EvalPolicy::default()
        ),
        Err(TessellateError::Chart(ChartError::NumericLimit))
    ));
}

// Fingerprints evaluated independently at pre-extension commit 6a89e76.
#[test]
fn existing_metric_fingerprints_stay_unchanged() {
    for (metric, expected) in [
        (None, 0xd1191c3620938d795e1b049ea49213ff_u128),
        (Some(extrusion()), 0x1765bb836a352ffcde43ae0a553c770e),
        (Some(revolution()), 0x95b07bd475f1172d96d05bc2da49acc2),
    ] {
        let mut b = RecipeBuilder::new();
        let profile = b.add_profile(builders::rect_from_corner(2.0, 1.0).unwrap());
        let kind = if matches!(metric, Some(SurfaceChart::Revolve { .. })) {
            NodeKind::Revolve {
                profile,
                placement: Placement3::IDENTITY,
                sweep: core::f64::consts::TAU,
                caps: CapMode::Both,
            }
        } else {
            NodeKind::Extrude {
                profile,
                placement: Placement3::IDENTITY,
                height: 3.0,
                caps: CapMode::Both,
            }
        };
        if let Some(chart) = metric {
            b.with_surface_chart(chart);
        }
        let root = b.add(kind).unwrap();
        let recipe = b.finish(root).unwrap();
        assert_eq!(
            recipe.recipe_fingerprint().0,
            expected,
            "pre-extension schema40 identity"
        );
    }
}

#[test]
fn loft_holes_use_reference_loop_metrics_and_cap_profiles_independently() {
    let profiles = [
        builders::ring(0.6, 0.2).unwrap(),
        builders::ring(1.0, 0.4).unwrap(),
        builders::ring(0.8, 0.3).unwrap(),
    ];
    let sections = [
        (Placement3::IDENTITY, &profiles[0]),
        (Placement3::translate(0.0, 0.0, 2.0), &profiles[1]),
        (Placement3::translate(0.0, 0.0, 4.0), &profiles[2]),
    ];
    let body = tessellate_loft_with_chart(
        &sections,
        LoftPolicy::Smooth,
        CapMode::Both,
        loft(1, 7.0),
        &EvalPolicy::default(),
    )
    .unwrap();
    clean(&body);
    assert!(body.mesh.boundary_loops().unwrap().is_empty());
    assert_wall_continuity(&body);
    let mut seams = [0, 0];
    for face in body.mesh.faces() {
        let sampling = body.source_map.chart_sampling(face).unwrap();
        assert!(
            sampling.loop_lengths[0] > 6.0 && sampling.loop_lengths[0] < core::f64::consts::TAU
        );
        assert!(
            sampling.loop_lengths[1] > 2.4
                && sampling.loop_lengths[1] < 0.4 * core::f64::consts::TAU
        );
        for (p, uv) in corners(&body, face) {
            match body.source_map.face_feature(face).unwrap() {
                Feature::CapStart | Feature::CapEnd => {
                    near(uv[0], p[0]);
                    near(uv[1], p[1]);
                }
                Feature::LoftWall { loop_index, .. } => {
                    if uv[0] == 0.0 {
                        seams[usize::from(loop_index)] += 1;
                    }
                    assert!(
                        uv[0] >= 0.0
                            && uv[0] <= sampling.loop_lengths[usize::from(loop_index)] + 2e-5
                    );
                }
                _ => unreachable!(),
            }
        }
    }
    assert!(seams.into_iter().all(|n| n > 0));
}

#[test]
fn reflected_retained_lofts_and_sweeps_preserve_rest_coordinates() {
    use crate::ir::Plane3;
    for chart in [loft(1, 9.3), sweep()] {
        let recipe = retained(chart, true);
        let mirrored = recipe
            .mirrored(Plane3 {
                normal: [1.0, 0.0, 0.0],
                distance: 0.0,
            })
            .unwrap();
        let original = crate::evaluate::evaluate(&recipe, &EvalPolicy::default()).unwrap();
        let reflected = crate::evaluate::evaluate(&mirrored, &EvalPolicy::default()).unwrap();
        let a = &original.bodies[0].body;
        let b = &reflected.bodies[0].body;
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
}
