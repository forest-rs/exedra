// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{body::Sense, geometry::Curve as NativeCurve, realize::RealizationPolicy};
use alloc::{vec, vec::Vec};
use exedra_curves::profile::{Bezier, Segment};
use exedra_math::{norm, sub};

const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);
const CROWN: SegTag = SegTag(42);

/// Along the chord from the origin to (4, 0), then back over a weighted
/// cubic crown, with a clockwise rectangular hole under it.
fn profile(weights: [f64; 4]) -> Profile {
    let outer = Loop::new(vec![
        Segment::line([4.0, 0.0]),
        Segment::bezier(
            [0.0, 0.0],
            Bezier::new(vec![[3.0, 2.0], [1.0, 2.0]], Some(weights.to_vec())),
        )
        .tagged(CROWN),
    ])
    .unwrap();
    let hole = Loop::new(vec![
        Segment::line([1.5, 0.6]),
        Segment::line([2.5, 0.6]),
        Segment::line([2.5, 0.3]),
        Segment::line([1.5, 0.3]),
    ])
    .unwrap();
    Profile::new(outer, vec![hole], &POLICY).unwrap()
}

/// Segment `k` of `loop_` at `u`, its fraction by length, angle or curve
/// parameter.
fn point_on(loop_: &Loop, k: usize, u: f64) -> [f64; 2] {
    let (from, segment) = loop_.iter_with_starts().nth(k).unwrap();
    match &segment.curve {
        Curve::Line => [
            from[0] + u * (segment.to[0] - from[0]),
            from[1] + u * (segment.to[1] - from[1]),
        ],
        Curve::Bezier(bezier) => {
            let curve = boundary::bezier_nurbs(from, segment.to, bezier, |p| p).unwrap();
            curve.view().evaluation(u).point
        }
        _ => loop_.arc(k).unwrap().point_at(u),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "display UVs are f32; the mapper checks finite output"
)]
fn texture(coordinates: [[f64; 2]; 3]) -> [[f32; 2]; 3] {
    coordinates.map(|uv| uv.map(|v| v as f32))
}

/// The crown's region under it, by a dense polygon.
fn crown_area(weights: [f64; 4]) -> f64 {
    let crown = profile(weights);
    let n = 1 << 16;
    // From (4, 0) over the crown to the origin, closed along the chord.
    let points: Vec<[f64; 2]> = (0..=n)
        .map(|i| point_on(crown.outer(), 1, f64::from(i) / f64::from(n)))
        .collect();
    let shoelace: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum();
    0.5 * shoelace
}

#[test]
fn bezier_walls_are_ruled_between_their_nurbs_edges() {
    let weights = [1.0, 1.4, 0.7, 1.0];
    let solid =
        Extrusion::new(profile(weights), Placement3::translate(7.0, 8.0, 9.0), 2.0).unwrap();
    assert!(solid.is_closed());
    let body = solid.body();
    assert_eq!(body.faces()[0].role, FaceRole::Cap(End::Start));
    assert_eq!(body.faces()[0].sense, Sense::Reverse);
    assert_eq!(body.faces()[0].loops.len(), 2);
    let crown: Vec<_> = solid.faces_tagged(CROWN).map(|(id, _)| id).collect();
    assert_eq!(crown.len(), 1);
    let wall = body.face(crown[0]).unwrap();
    assert!(matches!(wall.surface, Surface::Ruled(_)));
    assert_eq!(wall.sense, Sense::Forward);
    let nurbs = body
        .edges()
        .iter()
        .filter(|e| {
            e.regular()
                .is_some_and(|(c, _)| matches!(c, NativeCurve::Nurbs(_)))
        })
        .count();
    assert_eq!(nurbs, 2, "one Bezier edge on each cap");
    // The wall runs up from the crown's edge at its curve parameter.
    for (u, v) in [(0.25, 0.0), (0.5, 1.0), (0.75, 2.0)] {
        let p = solid.evaluate(crown[0], [u, v]).unwrap().point;
        let q = point_on(solid.profile().outer(), 1, u);
        assert!(norm(sub(p, [q[0], q[1], v])) < 1e-12, "{p:?} {q:?}");
    }
    // Volume and the crown wall's area agree with dense sums.
    let area = crown_area(weights) - 0.3;
    assert!(
        (solid.volume() - 2.0 * area).abs() < 1e-8,
        "{}",
        solid.volume()
    );
    let n = 1 << 16;
    let length: f64 = (0..n)
        .map(|i| {
            let (a, b) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
            let (p, q) = (
                point_on(solid.profile().outer(), 1, a),
                point_on(solid.profile().outer(), 1, b),
            );
            libm::hypot(q[0] - p[0], q[1] - p[1])
        })
        .sum();
    let wall_area = solid.face_area(crown[0]).unwrap();
    assert!(
        (wall_area - 2.0 * length).abs() < 1e-8,
        "{wall_area} {length}"
    );
}

#[test]
fn bezier_extrusions_realize_watertight_within_tolerance() {
    let solid = Extrusion::new(
        profile([1.0, 1.4, 0.7, 1.0]),
        Placement3::translate(7.0, 8.0, 9.0),
        2.0,
    )
    .unwrap();
    let crown = solid.faces_tagged(CROWN).next().unwrap().0;
    for tolerance in [0.05, 0.005] {
        let result = solid
            .body()
            .realize(
                solid.frame(),
                &RealizationPolicy {
                    tolerance,
                    ..RealizationPolicy::default()
                },
            )
            .unwrap();
        assert!(result.mesh_faces.iter().any(|&(_, native)| native == crown));
        assert!(result.report.within_tolerance(), "{:?}", result.report);
        assert!(result.mesh.boundary_loops().unwrap().is_empty());
        assert!(result.mesh.validate_deep().is_empty());
        let shared = solid
            .tessellate(&TessellationPolicy {
                chord_tolerance: tolerance,
                ..TessellationPolicy::default()
            })
            .unwrap();
        assert!(shared.report.within_tolerance(), "{:?}", shared.report);
        assert!(shared.mesh.boundary_loops().unwrap().is_empty());
        // The crown takes more edges as the tolerance tightens; the
        // chord, one.
        assert_eq!(shared.report.segment_edges[0], 1);
        assert!(shared.report.segment_edges[1] >= 4);
    }
    // A budget too small for the crown names it.
    assert!(matches!(
        solid.tessellate(&TessellationPolicy {
            chord_tolerance: 1e-9,
            max_segment_edges: 64,
            ..TessellationPolicy::default()
        }),
        Err(TessellateExtrusionError::BezierSampling {
            segment: BoundarySegment { segment: 1, .. },
            required: Some(_),
            maximum: 64,
        })
    ));
}

#[test]
fn deviation_evidence_includes_sample_rounding() {
    // A unit square far from the origin, closed by a straight weighted
    // Bezier along x = 1e8, placed back at the origin: its samples round
    // by far more than a tight chord bound, and the evidence must say so.
    let outer = Loop::new(vec![
        Segment::line([1e8 + 1.0, 0.0]),
        Segment::line([1e8 + 1.0, 1.0]),
        Segment::line([1e8, 1.0]),
        Segment::bezier([1e8, 0.0], Bezier::new(vec![], Some(vec![1.1, 1.1]))),
    ])
    .unwrap();
    let solid = Extrusion::new(
        Profile::new(outer, vec![], &POLICY).unwrap(),
        Placement3::translate(-1e8, 0.0, 0.0),
        1.0,
    )
    .unwrap();
    let result = solid
        .tessellate(&TessellationPolicy {
            chord_tolerance: 1e-9,
            min_arc_edges: 128,
            ..TessellationPolicy::default()
        })
        .unwrap();
    // The Bezier's edges on both caps lie along x = 0 once placed.
    for edge in [3, 7] {
        for &vertex in &result.edge_chains[edge] {
            let x = f64::from(result.mesh.vertex_position(vertex).unwrap()[0]);
            assert!(
                x.abs() <= result.report.max_deviation,
                "{x} {:?}",
                result.report
            );
        }
    }
}

#[test]
fn construction_coordinates_map_uvs_without_mesh_projection() {
    // Nonunit end weights exercise the wall's curve parameter.
    let profile = profile([1.25, 0.7, 1.4, 1.5]);
    let frame = Placement3::translate(11.0, 12.0, 13.0);
    let solid = Extrusion::new(profile.clone(), frame, 2.0).unwrap();
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
                    tag: Some(CROWN), ..
                } => 42,
                _ => 0,
            },
            |role, coordinates| {
                let expected = coordinates.map(|[u, z]| {
                    let local = match role {
                        FaceRole::Cap(end) => [u, z, if end == End::Start { 0.0 } else { 2.0 }],
                        FaceRole::Wall { segment, tag } => {
                            assert!((0.0..=1.0).contains(&u), "u={u}");
                            assert!((0.0..=2.0).contains(&z), "z={z}");
                            let loop_ = match segment.boundary {
                                Boundary::Profile(LoopRef::Outer) => profile.outer(),
                                Boundary::Profile(LoopRef::Hole(i)) => &profile.holes()[i],
                                Boundary::Pocket(_) => unreachable!("no pockets"),
                            };
                            let p = point_on(loop_, segment.segment, u);
                            if tag == Some(CROWN)
                                && (u == 0.0 || u == 1.0)
                                && (z == 0.0 || z == 2.0)
                            {
                                crown_corners[usize::from(u == 1.0) + 2 * usize::from(z == 2.0)] =
                                    true;
                            }
                            [p[0], p[1], z]
                        }
                        FaceRole::Floor { .. } => unreachable!("no pockets"),
                    };
                    frame.transform_point(local)
                });
                expected_triangles.push(expected);
                Ok::<_, usize>(texture(coordinates))
            },
        )
        .unwrap();
    assert!(crown_corners.into_iter().all(|seen| seen));
    assert_eq!(expected_triangles.len(), result.mesh_faces.len());
    for ((mesh_face, _), expected) in result.mesh_faces.iter().zip(expected_triangles) {
        for (corner, point) in result.mesh.face_loop(*mesh_face).zip(expected) {
            let vertex = result.mesh.to_vertex(corner).unwrap();
            let emitted = result.mesh.vertex_position(vertex).unwrap().map(f64::from);
            let gap = norm(sub(emitted, point));
            assert!(
                gap <= result.report.max_vertex_displacement + result.report.geometry_gap,
                "{emitted:?} vs {point:?}, gap {gap}"
            );
        }
    }
    assert!(matches!(
        solid.realize_with_uvs(
            &RealizationPolicy::default(),
            |_, _| 0,
            |_, _| -> Result<_, usize> { Err(17) }
        ),
        Err(ExtrusionUvError::Mapping { error: 17, .. })
    ));
    assert!(matches!(
        solid.realize_with_uvs(
            &RealizationPolicy::default(),
            |_, _| 0,
            |_, _| -> Result<_, usize> { Ok([[f32::NAN; 2]; 3]) }
        ),
        Err(ExtrusionUvError::NonFiniteUv { .. })
    ));
}

#[test]
fn arc_walls_and_pocket_floors_map_uvs_too() {
    // Lines, a center-defined arc and a pocket: every wall surface and
    // planar face kind reconstructs its emitted geometry.
    let outer = Loop::new(vec![
        Segment::line([4.0, 0.0]),
        Segment::arc_centered([4.0, 4.0], [4.0, 2.0], false),
        Segment::line([0.0, 4.0]),
        Segment::line([0.0, 0.0]),
    ])
    .unwrap();
    let pocket = Loop::new(vec![
        Segment::line([2.0, 1.0]),
        Segment::line([2.0, 2.0]),
        Segment::line([1.0, 2.0]),
        Segment::line([1.0, 1.0]),
    ])
    .unwrap();
    // A clockwise circular hole, and a pocket from the start end.
    let center = [1.0, 3.0];
    let hole = Loop::new(vec![
        Segment::arc_centered([0.6, 3.0], center, true),
        Segment::arc_centered([1.4, 3.0], center, true),
    ])
    .unwrap();
    let under = Loop::new(vec![
        Segment::line([3.2, 0.5]),
        Segment::line([3.2, 1.2]),
        Segment::line([2.5, 1.2]),
        Segment::line([2.5, 0.5]),
    ])
    .unwrap();
    let frame = Placement3::translate(1.0, -2.0, 0.5);
    let solid = Extrusion::new(
        Profile::new(outer, vec![hole], &POLICY).unwrap(),
        frame,
        3.0,
    )
    .unwrap()
    .with_pocket(Pocket::from_region(&pocket, End::End, 1.0), &POLICY)
    .unwrap()
    .with_pocket(Pocket::from_region(&under, End::Start, 1.0), &POLICY)
    .unwrap();
    let mut seen = [false; 5];
    let mut expected_triangles = Vec::new();
    let result = solid
        .realize_with_uvs(
            &RealizationPolicy {
                tolerance: 0.01,
                // Refinement adds vertices inside faces.
                quality: Some(crate::realize::TriangleQuality::new(0.2)),
                ..Default::default()
            },
            |_, _| 0,
            |role, coordinates| {
                let expected = coordinates.map(|[u, v]| {
                    let local = match role {
                        FaceRole::Cap(end) => [u, v, if end == End::Start { 0.0 } else { 3.0 }],
                        FaceRole::Floor { pocket } => {
                            seen[usize::from(pocket == 1) * 3] = true;
                            [u, v, if pocket == 0 { 2.0 } else { 1.0 }]
                        }
                        FaceRole::Wall { segment, .. } => {
                            seen[1] |= matches!(segment.boundary, Boundary::Pocket(_));
                            seen[2] |= segment.boundary == Boundary::Profile(LoopRef::Outer)
                                && segment.segment == 1;
                            seen[4] |= segment.boundary == Boundary::Profile(LoopRef::Hole(0));
                            let p =
                                point_on(solid.boundary_loop(segment.boundary), segment.segment, u);
                            [p[0], p[1], v]
                        }
                    };
                    frame.transform_point(local)
                });
                expected_triangles.push(expected);
                Ok::<_, usize>(texture(coordinates))
            },
        )
        .unwrap();
    assert_eq!(
        seen, [true; 5],
        "both floors, pocket walls, the arc's wall and the hole's"
    );
    assert!(
        result
            .mesh_vertices
            .iter()
            .any(|(_, source)| matches!(source, VertexSource::Face { .. })),
        "refinement must exercise face-interior coordinates"
    );
    for ((mesh_face, _), expected) in result.mesh_faces.iter().zip(expected_triangles) {
        for (corner, point) in result.mesh.face_loop(*mesh_face).zip(expected) {
            let vertex = result.mesh.to_vertex(corner).unwrap();
            let emitted = result.mesh.vertex_position(vertex).unwrap().map(f64::from);
            let gap = norm(sub(emitted, point));
            assert!(
                gap <= result.report.max_vertex_displacement + result.report.geometry_gap,
                "{emitted:?} vs {point:?}, gap {gap}"
            );
        }
    }
}

#[test]
fn measures_say_whether_they_are_exact() {
    let place = Placement3::IDENTITY;
    // Rational: neither the area under the crown nor its length is exact.
    let rational = Extrusion::new(profile([1.0, 1.4, 0.7, 1.0]), place, 2.0).unwrap();
    assert!(!rational.volume_is_exact());
    assert!(!rational.surface_area_is_exact());
    // Polynomial: the area is, the length still is not.
    let polynomial = Extrusion::new(profile([1.0; 4]), place, 2.0).unwrap();
    assert!(
        !polynomial.volume_is_exact(),
        "weights given, even unit ones"
    );
    let plain = Profile::new(
        Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::bezier([0.0, 0.0], Bezier::new(vec![[3.0, 2.0], [1.0, 2.0]], None)),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap();
    let plain = Extrusion::new(plain, place, 2.0).unwrap();
    assert!(plain.volume_is_exact());
    assert!(!plain.surface_area_is_exact());
    // Lines and arcs: both are.
    let block = Profile::new(
        Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::arc([0.0, 0.0], 0.5),
        ])
        .unwrap(),
        vec![],
        &POLICY,
    )
    .unwrap();
    let block = Extrusion::new(block, place, 2.0).unwrap();
    assert!(block.volume_is_exact() && block.surface_area_is_exact());
}
