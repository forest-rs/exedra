// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;
use core::f64::consts::{PI, TAU};
use exedra_curves::profile::{ProfilePolicy, Segment};
const BASE: SegTag = SegTag(1);
const SHELL: SegTag = SegTag(2);
const AXIS: SegTag = SegTag(3);

fn profile(segments: Vec<Segment>, scale: f64) -> Profile {
    Profile::new(
        Loop::new(segments).unwrap(),
        vec![],
        &ProfilePolicy::new(scale * 1e-9),
    )
    .unwrap()
}
fn sphere(radius: f64) -> Profile {
    profile(
        vec![
            Segment::line([0.0, -radius]).tagged(AXIS),
            Segment::arc([0.0, radius], 1.0).tagged(SHELL),
        ],
        radius,
    )
}
fn cone(radius: f64, height: f64, extra_axis: bool) -> Profile {
    let mut edges = vec![
        Segment::line([radius, 0.0]).tagged(BASE),
        Segment::line([0.0, height]).tagged(SHELL),
    ];
    if extra_axis {
        edges.push(Segment::line([0.0, height * 0.5]).tagged(AXIS));
    }
    edges.push(Segment::line([0.0, 0.0]).tagged(AXIS));
    profile(edges, radius.max(height))
}
fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= expected.abs() * 2e-13,
        "{actual} vs{expected}"
    );
}

#[test]
fn full_sphere_and_pointed_cone_are_closed_exact_supports_with_source_roles() {
    for scale in [1e-6, 1.0, 1e6] {
        let sphere = Revolution::new(sphere(2.0 * scale), Placement3::IDENTITY, TAU).unwrap();
        assert!(sphere.body().is_closed());
        let bounds = sphere.body().conservative_bounds().unwrap();
        assert!(sphere.body().vertices().iter().all(|p| {
            p.iter()
                .enumerate()
                .all(|(k, x)| *x >= bounds[0][k] && *x <= bounds[1][k])
        }));
        assert_eq!(sphere.body().vertices().len(), 2);
        assert_eq!(sphere.body().edges().len(), 3);
        assert_eq!(sphere.body().faces().len(), 1);
        assert!(matches!(
            sphere.body().faces()[0].surface,
            Surface::Sphere(_)
        ));
        assert_eq!(
            sphere
                .body()
                .edges()
                .iter()
                .filter(|e| e.is_collapsed())
                .count(),
            2
        );
        assert_eq!(sphere.faces_tagged(AXIS).count(), 0);
        assert_eq!(sphere.faces_tagged(SHELL).count(), 1);
        close(sphere.volume(), 4.0 * PI * (2.0 * scale).powi(3) / 3.0);
        close(sphere.surface_area(), 4.0 * PI * (2.0 * scale).powi(2));
        for extra_axis in [false, true] {
            let solid = Revolution::new(
                cone(2.0 * scale, 3.0 * scale, extra_axis),
                Placement3::IDENTITY,
                TAU,
            )
            .unwrap();
            assert!(solid.body().is_closed());
            let bounds = solid.body().conservative_bounds().unwrap();
            assert!(solid.body().vertices().iter().all(|p| {
                p.iter()
                    .enumerate()
                    .all(|(k, x)| *x >= bounds[0][k] && *x <= bounds[1][k])
            }));
            assert_eq!(solid.body().faces().len(), 2);
            assert_eq!(
                solid.body().vertices().len(),
                2,
                "disk center and axis-chain interior are omitted"
            );
            assert_eq!(solid.body().edges().len(), 3);
            assert_eq!(
                solid
                    .body()
                    .edges()
                    .iter()
                    .filter(|e| e.is_collapsed())
                    .count(),
                1
            );
            assert_eq!(solid.faces_tagged(AXIS).count(), 0);
            let base = solid.faces_tagged(BASE).next().unwrap();
            assert_eq!(
                solid.body().face(base).unwrap().loops.len(),
                1,
                "disk has no collapsed inner loop"
            );
            assert!(solid.body().planar_frame(base, [0.0; 2]).unwrap().rows[1][2] < 0.0);
            close(solid.volume(), PI * (2.0 * scale).powi(2) * scale);
            close(
                solid.surface_area(),
                PI * (2.0 * scale) * (2.0 * scale + libm::hypot(2.0 * scale, 3.0 * scale)),
            );
        }
    }
}

#[test]
fn isolated_axis_touch_partial_turn_and_near_axis_torus_are_distinct_refusals() {
    let isolated = profile(
        vec![
            Segment::line([0.0, 0.0]),
            Segment::line([1.0, -1.0]),
            Segment::line([1.0, 1.0]),
        ],
        1.0,
    );
    assert!(matches!(
        Revolution::new(isolated, Placement3::IDENTITY, TAU),
        Err(RevolutionError::AxisTopology { .. })
    ));
    assert!(matches!(
        Revolution::new(sphere(2.0), Placement3::IDENTITY, PI),
        Err(RevolutionError::AxisContact { .. })
    ));
    let epsilon = 1e-6;
    let near = profile(
        vec![
            Segment::line([epsilon, -2.0]),
            Segment::arc([epsilon, 2.0], 1.0),
        ],
        1.0,
    );
    assert!(matches!(
        Revolution::new(near, Placement3::IDENTITY, TAU),
        Err(RevolutionError::NonRegularArc { .. })
    ));
}

#[test]
fn cone_apex_requires_explicit_boundary_and_keeps_regular_evaluation_strict() {
    let solid = Revolution::new(cone(2.0, 3.0, false), Placement3::IDENTITY, TAU).unwrap();
    let wall = solid.faces_tagged(SHELL).next().unwrap();
    let face = solid.body().face(wall).unwrap();
    let Surface::Cone(support) = &face.surface else {
        panic!("cone wall");
    };
    let apex = support.apex_parameter().unwrap();
    assert_eq!(support.radius_at(apex), None);
    assert_eq!(face.surface.evaluate([0.5, apex]), None);
    assert!(support.apex_point().is_some());
    let mut parts = solid.body().clone().into_parts();
    let loop_id = parts.faces[wall.index()].loops[0];
    parts.loops[loop_id.index()]
        .coedges
        .retain(|c| !parts.edges[c.edge.index()].is_collapsed());
    assert!(matches!(
        Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }),
        Err(BodyError::NonRegularSurface { .. })
    ));
    let mut parts = solid.body().clone().into_parts();
    parts.faces[wall.index()].domain.v = ParameterRange::new(0.0, apex + 1e-6).unwrap();
    assert!(
        matches!(
            Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1.0 }),
            Err(BodyError::NonRegularSurface { .. })
        ),
        "a loose gap never authorizes crossing the apex"
    );
}

#[test]
fn center_authored_domes_keep_exact_spherical_support_and_planar_base() {
    for height in [0.25_f64, 1.0, 2.0, 3.5] {
        let radius = 2.0;
        let base = radius - height;
        let radial = libm::sqrt(height) * libm::sqrt(2.0 * radius - height);
        let profile = profile(
            vec![
                Segment::line([radial, base]).tagged(BASE),
                Segment::arc_centered([0.0, radius], [0.0, 0.0], false).tagged(SHELL),
                Segment::line([0.0, base]).tagged(AXIS),
            ],
            radius,
        );
        let source = Revolution::new(profile, Placement3::IDENTITY, TAU).unwrap();
        assert!(source.body().is_closed());
        assert_eq!(source.body().faces().len(), 2);
        assert_eq!(
            source
                .body()
                .edges()
                .iter()
                .filter(|e| e.is_collapsed())
                .count(),
            1
        );
        let shell = source.faces_tagged(SHELL).next().unwrap();
        assert!(matches!(
            source.body().faces()[shell.index()].surface,
            Surface::Sphere(_)
        ));
        close(
            source.volume(),
            PI * height * height * (radius - height / 3.0),
        );
        close(
            source.surface_area(),
            2.0 * PI * radius * height + PI * height * (2.0 * radius - height),
        );
        let mesh = source
            .realize(&RealizationPolicy {
                tolerance: 0.005,
                ..Default::default()
            })
            .unwrap();
        assert!(mesh.mesh.boundary_loops().unwrap().is_empty());
        assert!(mesh.mesh.validate_deep().is_empty());
        assert!(mesh.report.within_tolerance(), "{:?}", mesh.report);
    }
}
