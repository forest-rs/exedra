// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    End,
    body::Sense,
    geometry::{Curve, NurbsCurve, ParameterRange},
    realize::RealizationPolicy,
};
use alloc::{vec, vec::Vec};

fn line(from: [f64; 3], to: [f64; 3]) -> PlanarSegment {
    CurveSpan::new(
        Curve::line(from, exedra_math::sub(to, from)).unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap()
    .into()
}
fn spline_outline() -> Vec<PlanarSegment> {
    let curve = Curve::nurbs(
        NurbsCurve::new(
            4,
            vec![2., 2., 2., 2., 5., 5., 5., 5.],
            vec![[0., 0., 0.], [1., 2., 0.], [3., 2., 0.], [4., 0., 0.]],
            Some(vec![1., 0.7, 1.4, 1.]),
        )
        .unwrap(),
    )
    .unwrap();
    // CCW: along the chord, then back over the weighted cubic.
    vec![
        line([0.; 3], [4., 0., 0.]),
        PlanarSegment::new(
            CurveSpan::new(curve, ParameterRange::new(2., 5.).unwrap(), true).unwrap(),
        )
        .tagged(SegTag(42)),
    ]
}
fn hole() -> Vec<PlanarSegment> {
    let p = [
        [1.5, 0.3, 0.],
        [1.5, 0.6, 0.],
        [2.5, 0.6, 0.],
        [2.5, 0.3, 0.],
    ];
    (0..4).map(|i| line(p[i], p[(i + 1) % 4])).collect()
}
#[test]
fn weighted_spline_hole_roles_ranges_and_watertight_realization() {
    let solid = PlanarExtrusion::new(
        spline_outline(),
        vec![hole()],
        Placement3::translate(7., 8., 9.),
        2.,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap();
    assert_eq!(solid.frame(), Placement3::translate(7., 8., 9.));
    assert_eq!(solid.length(), 2.);
    assert_eq!(solid.report().source_segments, 6);
    assert!(solid.report().checked_pieces >= 6);
    assert!(solid.report().pair_tests > 0);
    let body = solid.body();
    assert!(body.is_closed());
    assert_eq!(body.faces()[0].role, FaceRole::Cap(End::Start));
    assert_eq!(body.faces()[0].sense, Sense::Reverse);
    assert_eq!(body.faces()[1].role, FaceRole::Cap(End::End));
    assert_eq!(body.faces()[0].loops.len(), 2);
    let tagged: Vec<_> = solid.faces_tagged(SegTag(42)).map(|(id, _)| id).collect();
    assert_eq!(tagged.len(), 1);
    assert_eq!(solid.faces_tagged(SegTag(999)).count(), 0);
    assert_eq!(
        body.face(solid.cap(End::Start)).unwrap().role,
        FaceRole::Cap(End::Start)
    );
    assert_eq!(
        body.face(solid.cap(End::End)).unwrap().role,
        FaceRole::Cap(End::End)
    );
    assert!(
        body.edges().iter().any(|e| e
            .regular()
            .is_some_and(|(c, r)| matches!(c, Curve::Nurbs(_)) && r.bounds() == [2., 5.]))
    );
    assert_eq!(body.trims().len(), 4);
    for tolerance in [0.05, 0.005] {
        let result = body
            .realize(
                solid.frame(),
                &RealizationPolicy {
                    tolerance,
                    ..RealizationPolicy::default()
                },
            )
            .unwrap();
        for id in [solid.cap(End::Start), solid.cap(End::End), tagged[0]] {
            assert!(
                result.mesh_faces.iter().any(|&(_, native)| native == id),
                "native feature IDs survive each realization"
            );
        }
        assert_eq!(solid.faces_tagged(SegTag(42)).next().unwrap().0, tagged[0]);
        assert!(result.report.within_tolerance(), "{:?}", result.report);
        assert!(result.mesh.boundary_loops().unwrap().is_empty());
        assert!(result.mesh.validate_deep().is_empty());
    }
}
#[test]
fn invalid_open_nonplanar_winding_holes_and_work_are_refused() {
    let policy = PlanarExtrusionPolicy::default();
    let build = |outer, holes, policy: &PlanarExtrusionPolicy| {
        PlanarExtrusion::new(outer, holes, Placement3::IDENTITY, 2., policy)
    };
    let mut open = spline_outline();
    open[0] = line([0.125, 0., 0.], [4., 0., 0.]);
    assert!(matches!(
        build(open, vec![], &policy),
        Err(PlanarExtrusionError::OpenLoop { .. })
    ));
    let clockwise = spline_outline()
        .into_iter()
        .rev()
        .map(|mut s| {
            s.span =
                CurveSpan::new(s.span.curve().clone(), s.span.range(), !s.span.reversed()).unwrap();
            s
        })
        .collect();
    assert!(matches!(
        build(clockwise, vec![], &policy),
        Err(PlanarExtrusionError::Profile(_))
    ));
    let outside: Vec<_> = hole()
        .into_iter()
        .map(|mut s| {
            s.span = CurveSpan::new(
                s.span
                    .curve()
                    .transformed(Placement3::translate(20., 0., 0.))
                    .unwrap(),
                s.span.range(),
                false,
            )
            .unwrap();
            s
        })
        .collect();
    assert!(matches!(
        build(spline_outline(), vec![outside], &policy),
        Err(PlanarExtrusionError::Profile(_))
    ));
    assert!(matches!(
        build(
            spline_outline(),
            vec![],
            &PlanarExtrusionPolicy {
                max_pair_tests: 1,
                ..policy
            }
        ),
        Err(PlanarExtrusionError::Budget { .. })
    ));
    assert!(matches!(
        build(
            spline_outline(),
            vec![],
            &PlanarExtrusionPolicy {
                max_pieces: 1,
                ..policy
            }
        ),
        Err(PlanarExtrusionError::Budget { .. })
    ));
}
#[test]
fn crossings_and_nonadjacent_shared_vertices_are_never_accepted() {
    let p = [[0., 0., 0.], [3., 2., 0.], [0., 2., 0.], [2., 0., 0.]];
    let crossed = (0..4).map(|i| line(p[i], p[(i + 1) % 4])).collect();
    assert!(matches!(
        PlanarExtrusion::new(
            crossed,
            vec![],
            Placement3::IDENTITY,
            1.,
            &PlanarExtrusionPolicy {
                max_subdivisions: 3,
                ..Default::default()
            }
        ),
        Err(PlanarExtrusionError::Unresolved { .. })
    ));
    let p = [
        [0., 0., 0.],
        [2., 0., 0.],
        [1., 1., 0.],
        [2., 2., 0.],
        [0., 2., 0.],
        [1., 1., 0.],
    ];
    let touching = (0..p.len())
        .map(|i| line(p[i], p[(i + 1) % p.len()]))
        .collect();
    assert!(
        PlanarExtrusion::new(
            touching,
            vec![],
            Placement3::IDENTITY,
            1.,
            &PlanarExtrusionPolicy {
                max_subdivisions: 3,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the test deliberately maps native parameters to f32 texture coordinates"
)]
fn construction_parameters_map_uvs_without_mesh_projection() {
    let mut outer = spline_outline();
    // Nonunit endpoint weights and a reversed nonunit domain exercise both
    // canonical edge direction and authored wall traversal.
    let Curve::Nurbs(n) = outer[1].span.curve() else {
        unreachable!()
    };
    let curve = Curve::nurbs(
        NurbsCurve::new(
            n.curve().order(),
            n.curve().knots().to_vec(),
            n.curve().points().to_vec(),
            Some(vec![1.25, 0.7, 1.4, 1.5]),
        )
        .unwrap(),
    )
    .unwrap();
    outer[1].span = CurveSpan::new(curve, outer[1].span.range(), true).unwrap();
    let holes = vec![hole()];
    let frame = Placement3::translate(11., 12., 13.);
    let solid = PlanarExtrusion::new(
        outer.clone(),
        holes.clone(),
        frame,
        2.,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap();
    let mut expected_triangles = Vec::new();
    let mut crown_corners = [false; 4];
    let result = solid
        .realize_with_uvs(
            &RealizationPolicy {
                tolerance: 0.02,
                ..Default::default()
            },
            |_, f| match f.role {
                FaceRole::Wall {
                    tag: Some(SegTag(42)),
                    ..
                } => 42,
                _ => 0,
            },
            |role, coordinates| {
                for [u, v] in coordinates {
                    assert!(u.is_finite() && v.is_finite());
                    if matches!(role, FaceRole::Wall { .. }) {
                        assert!((0.0..=1.0).contains(&u), "u={u}");
                        assert!((0.0..=2.0).contains(&v), "v={v}");
                    }
                }
                let expected = coordinates.map(|[u, z]| {
                    let local = match role {
                        FaceRole::Cap(end) => [u, z, if end == End::Start { 0.0 } else { 2.0 }],
                        FaceRole::Wall { segment, tag } => {
                            let loop_ = match segment.boundary {
                                crate::extrusion::Boundary::Profile(LoopRef::Outer) => &outer,
                                crate::extrusion::Boundary::Profile(LoopRef::Hole(i)) => &holes[i],
                                _ => unreachable!("this constructor has no pockets"),
                            };
                            let mut point = loop_[segment.segment].span.evaluate(u).unwrap().point;
                            point[2] += z;
                            if tag == Some(SegTag(42))
                                && (u == 0.0 || u == 1.0)
                                && (z == 0.0 || z == 2.0)
                            {
                                crown_corners[usize::from(u == 1.0) + 2 * usize::from(z == 2.0)] =
                                    true;
                            }
                            point
                        }
                        FaceRole::Floor { .. } => unreachable!("this constructor has no pockets"),
                    };
                    frame.transform_point(local)
                });
                expected_triangles.push(expected);
                Ok::<_, usize>(coordinates.map(|uv| uv.map(|v| v as f32)))
            },
        )
        .unwrap();
    assert!(
        crown_corners.into_iter().all(|seen| seen),
        "both spline rails and both endpoints must be exercised"
    );
    assert_eq!(expected_triangles.len(), result.mesh_faces.len());
    for ((mesh_face, _), expected) in result.mesh_faces.iter().zip(expected_triangles) {
        for (corner, point) in result.mesh.face_loop(*mesh_face).zip(expected) {
            let vertex = result.mesh.to_vertex(corner).unwrap();
            let emitted = result.mesh.vertex_position(vertex).unwrap().map(f64::from);
            let gap = exedra_math::norm(exedra_math::sub(emitted, point));
            assert!(
                gap <= result.report.max_vertex_displacement + result.report.geometry_gap,
                "construction coordinates must reconstruct emitted geometry: {emitted:?} vs {point:?}, gap {gap}"
            );
        }
    }
    assert!(
        result
            .mesh
            .attrs()
            .sparse(exedra_mesh::attr::CORNER_UV)
            .is_some()
    );
    assert!(result.mesh.boundary_loops().unwrap().is_empty());
    assert!(matches!(
        solid.realize_with_uvs(
            &RealizationPolicy::default(),
            |_, _| 0,
            |_, _| -> Result<_, usize> { Err(17) }
        ),
        Err(PlanarExtrusionUvError::Mapping { error: 17, .. })
    ));
    assert!(matches!(
        solid.realize_with_uvs(
            &RealizationPolicy::default(),
            |_, _| 0,
            |_, _| -> Result<_, usize> { Ok([[f32::NAN; 2]; 3]) }
        ),
        Err(PlanarExtrusionUvError::NonFiniteUv { .. })
    ));
}

#[test]
fn close_crossings_scale_and_source_restrictions_are_explicit() {
    for scale in [1e-6_f64, 1.0, 1e6] {
        let policy = PlanarExtrusionPolicy {
            max_gap: (1e-9 * scale).max(1e-12),
            chord_separation: 1e-9 * scale,
            max_subdivisions: 8,
            ..Default::default()
        };
        let make = |dy| {
            let curve = Curve::nurbs(
                NurbsCurve::new(
                    4,
                    vec![0., 0., 0., 0., 1., 1., 1., 1.],
                    vec![
                        [0., 0., 0.],
                        [scale * 1e-10, scale * dy, 0.],
                        [scale * 3., scale * 2., 0.],
                        [scale * 4., 0., 0.],
                    ],
                    Some(vec![1., 0.7, 1.4, 1.]),
                )
                .unwrap(),
            )
            .unwrap();
            vec![
                line([0.; 3], [4. * scale, 0., 0.]),
                CurveSpan::new(curve, ParameterRange::UNIT, true)
                    .unwrap()
                    .into(),
            ]
        };
        assert!(
            PlanarExtrusion::new(make(-1e-10), vec![], Placement3::IDENTITY, scale, &policy)
                .is_err(),
            "close crossing at scale {scale}"
        );
        let mut valid = spline_outline();
        for segment in &mut valid {
            let c = segment.span.curve();
            let scaled = match c {
                Curve::Line(l) => Curve::line(
                    l.origin().map(|v| v * scale),
                    l.direction().map(|v| v * scale),
                )
                .unwrap(),
                Curve::Nurbs(n) => Curve::nurbs(
                    NurbsCurve::new(
                        n.curve().order(),
                        n.curve().knots().to_vec(),
                        n.curve()
                            .points()
                            .iter()
                            .map(|p| p.map(|v| v * scale))
                            .collect(),
                        n.curve().weights().map(<[f64]>::to_vec),
                    )
                    .unwrap(),
                )
                .unwrap(),
                _ => unreachable!(),
            };
            segment.span =
                CurveSpan::new(scaled, segment.span.range(), segment.span.reversed()).unwrap();
        }
        let result = PlanarExtrusion::new(valid, vec![], Placement3::IDENTITY, scale, &policy);
        assert!(result.is_ok(), "scale {scale}: {result:?}");
    }
    let mut partial = spline_outline();
    partial[1].span = CurveSpan::new(
        partial[1].span.curve().clone(),
        ParameterRange::new(2., 4.).unwrap(),
        true,
    )
    .unwrap();
    assert!(matches!(
        PlanarExtrusion::new(
            partial,
            vec![],
            Placement3::IDENTITY,
            1.,
            &PlanarExtrusionPolicy::default()
        ),
        Err(PlanarExtrusionError::UnsupportedSource { .. })
    ));
    let mut lifted = spline_outline();
    for s in &mut lifted {
        s.span = CurveSpan::new(
            s.span
                .curve()
                .transformed(Placement3::translate(0., 0., 1e-12))
                .unwrap(),
            s.span.range(),
            s.span.reversed(),
        )
        .unwrap();
    }
    assert!(matches!(
        PlanarExtrusion::new(
            lifted,
            vec![],
            Placement3::IDENTITY,
            1.,
            &PlanarExtrusionPolicy::default()
        ),
        Err(PlanarExtrusionError::NonPlanar { .. })
    ));
}

#[test]
fn repeated_wall_labels_select_every_native_feature() {
    let mut outline = spline_outline();
    outline[0].tag = Some(SegTag(42));
    let solid = PlanarExtrusion::new(
        outline,
        vec![],
        Placement3::IDENTITY,
        1.,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap();
    let matches: Vec<_> = solid.faces_tagged(SegTag(42)).collect();
    assert_eq!(matches.len(), 2);
    assert!(matches.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(matches.iter().all(|(id, face)| *id != solid.cap(End::Start)
        && *id != solid.cap(End::End)
        && matches!(face.role, FaceRole::Wall { .. })));
}

#[test]
fn decimal_line_endpoints_are_retained_without_weakening_source_checks() {
    let segment = PlanarSegment::line([0.1, 0.2], [0.3, 0.7]).unwrap();
    assert!(matches!(segment.span.curve(), Curve::Line(_)));
    assert_eq!(segment.span.evaluate(0.0).unwrap().point, [0.1, 0.2, 0.0]);
    assert_eq!(segment.span.evaluate(1.0).unwrap().point, [0.3, 0.7, 0.0]);
    let fallback = PlanarSegment::line([0.1, 0.2], [1.0, 0.7]).unwrap();
    let Curve::Nurbs(n) = fallback.span.curve() else {
        panic!("inexact endpoint coefficient arithmetic must retain endpoint controls");
    };
    assert_eq!(n.curve().order(), 2);
    assert_eq!(n.curve().points(), &[[0.1, 0.2, 0.0], [1.0, 0.7, 0.0]]);
    assert_eq!(fallback.span.evaluate(0.0).unwrap().point, [0.1, 0.2, 0.0]);
    assert_eq!(fallback.span.evaluate(1.0).unwrap().point, [1.0, 0.7, 0.0]);
    let p = [[0.1, 0.2], [1.0, 0.2], [1.0, 0.7], [0.1, 0.7]];
    let outer = (0..4)
        .map(|i| PlanarSegment::line(p[i], p[(i + 1) % 4]).unwrap())
        .collect();
    let solid = PlanarExtrusion::new(
        outer,
        vec![],
        Placement3::IDENTITY,
        0.5,
        &PlanarExtrusionPolicy::default(),
    )
    .unwrap();
    assert!(solid.body().is_closed());
    let realized = solid
        .body()
        .realize(solid.frame(), &RealizationPolicy::default())
        .unwrap();
    assert!(realized.report.within_tolerance());
    assert!(realized.mesh.boundary_loops().unwrap().is_empty());
    assert!(realized.mesh.validate_deep().is_empty());
    assert!(matches!(
        PlanarSegment::line([0.1, 0.2], [0.1, 0.2]),
        Err(GeometryError::ZeroDirection)
    ));
    assert!(matches!(
        PlanarSegment::line([f64::NAN, 0.0], [1.0, 0.0]),
        Err(GeometryError::NonFinite)
    ));
}
