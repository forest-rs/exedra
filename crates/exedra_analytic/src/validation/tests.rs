// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::body::{BodyParts, BoundaryLoop, Coedge, Edge, Face, Sense};
use crate::extrusion::Extrusion;
use crate::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
use crate::machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder};
use alloc::{vec, vec::Vec};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

fn cube(origin: [f64; 3], length: f64) -> Body {
    let ring = Loop::new(vec![
        Segment::line([0.0, 0.0]),
        Segment::line([length, 0.0]),
        Segment::line([length, length]),
        Segment::line([0.0, length]),
    ])
    .unwrap();
    let profile = Profile::new(ring, vec![], &ProfilePolicy::new(1e-9)).unwrap();
    let mut body = Extrusion::new(profile, Placement3::IDENTITY, length)
        .unwrap()
        .body()
        .clone()
        .map_roles(|_| (), |_| ());
    for p in &mut body.parts.vertices {
        *p = exedra_math::add(*p, origin);
    }
    for edge in &mut body.parts.edges {
        let (curve, _) = edge.regular_mut().unwrap();
        let Curve::Line(line) = curve else {
            unreachable!()
        };
        *curve = Curve::line(exedra_math::add(line.origin(), origin), line.direction()).unwrap();
    }
    for face in &mut body.parts.faces {
        let mut frame = face.surface.frame();
        for (i, offset) in origin.into_iter().enumerate() {
            frame.rows[i][3] += offset;
        }
        face.surface = Surface::plane(frame).unwrap();
    }
    body
}
fn explicit(mut body: Body) -> Body {
    crate::operation_trims::append_all_planar(&mut body.parts).unwrap();
    body
}
#[test]
fn cube_gets_complete_boundary_and_convex_solid_evidence() {
    for body in [cube([0.0; 3], 4.0), explicit(cube([0.0; 3], 4.0))] {
        let report = body
            .validate_convex_solid(&BoundaryValidationPolicy::default())
            .unwrap();
        assert!(report.boundary.all_trims_checked());
        assert_eq!(report.boundary.shells.len(), 1);
        assert_eq!(report.boundary.shells[0].euler_characteristic, 2);
        assert_eq!(report.boundary.shells[0].vertices, 8);
        assert_eq!(report.boundary.shells[0].regular_edges, 12);
        assert_eq!(report.interior, [2.0; 3]);
        assert!((report.volume - 64.0).abs() < 1e-12);
        assert!(report.interior_clearance > 1.99);
    }
}
#[test]
fn oblique_bore_checks_full_ellipses_and_cylinder_graphs() {
    let stock = cube([-10.0, -10.0, 0.0], 20.0);
    let cut = subtract_through_cylinder(
        &stock,
        &CylinderTool::new([0.0, 0.0, 0.0], [0.1, 0.0, 1.0], 1.0).unwrap(),
        &MachiningPolicy::default(),
    )
    .unwrap();
    let report = cut
        .body
        .validate_boundary(&BoundaryValidationPolicy::default())
        .unwrap();
    assert!(report.all_trims_checked(), "{:?}", report.face_trims);
    assert_eq!(report.shells[0].euler_characteristic, 0);
    assert_eq!(
        report.face_trims[cut.report.wall.index()],
        TrimValidation::Certified(TrimProof::CylindricalGraphs)
    );
    for face in cut.report.trimmed_faces {
        assert_eq!(
            report.face_trims[face.index()],
            TrimValidation::Certified(TrimProof::PlanarEllipses)
        );
    }
    assert!(matches!(
        cut.body
            .validate_convex_solid(&BoundaryValidationPolicy::default()),
        Err(ConvexSolidError::EulerCharacteristic { found: 0 })
    ));
}
fn join(mut a: Body, b: Body, merge: Option<(usize, usize)>) -> Body {
    let b = b.into_parts();
    let (vo, eo, lo) = (
        a.parts.vertices.len(),
        a.parts.edges.len(),
        a.parts.loops.len(),
    );
    a.parts.vertices.extend(b.vertices);
    a.parts.edges.extend(b.edges.into_iter().map(|mut e| {
        let vertex = |v: VertexId| {
            if let Some((av, bv)) = merge
                && v.index() == bv
            {
                return VertexId(crate::usize_to_u32(av));
            }
            VertexId(crate::usize_to_u32(v.index() + vo))
        };
        e.start = vertex(e.start);
        e.end = vertex(e.end);
        e
    }));
    a.parts.loops.extend(b.loops.into_iter().map(|mut l| {
        for co in &mut l.coedges {
            co.edge.0 += u32::try_from(eo).unwrap();
        }
        l
    }));
    a.parts.faces.extend(b.faces.into_iter().map(|mut f| {
        for id in &mut f.loops {
            id.0 += u32::try_from(lo).unwrap();
        }
        f
    }));
    a
}
#[test]
fn touching_vertex_shells_pass_incidence_but_fail_connected_link() {
    let a = cube([0.0; 3], 2.0);
    let b = cube([2.0; 3], 2.0);
    let av = a.vertices().iter().position(|p| *p == [2.0; 3]).unwrap();
    let bv = b.vertices().iter().position(|p| *p == [2.0; 3]).unwrap();
    let body = join(a, b, Some((av, bv)));
    body.validate(&ValidationPolicy { max_gap: 1e-9 }).unwrap();
    assert!(body.is_closed());
    assert!(
        matches!(body.validate_boundary(&BoundaryValidationPolicy::default()),Err(BoundaryValidationError::DisconnectedVertexLink{vertex}) if vertex.index()==av)
    );
}
#[test]
fn disjoint_shells_are_reported_and_convex_material_claim_is_refused() {
    let body = join(cube([0.0; 3], 2.0), cube([4.0; 3], 2.0), None);
    let report = body
        .validate_boundary(&BoundaryValidationPolicy::default())
        .unwrap();
    assert_eq!(report.shells.len(), 2);
    assert!(report.shells.iter().all(|s| s.euler_characteristic == 2));
    assert!(matches!(
        body.validate_convex_solid(&BoundaryValidationPolicy::default()),
        Err(ConvexSolidError::ShellCount { found: 2 })
    ));
}
#[test]
fn inward_shell_has_valid_arrangements_but_no_convex_material_certificate() {
    let mut body = cube([0.0; 3], 4.0);
    for f in &mut body.parts.faces {
        f.sense = match f.sense {
            Sense::Forward => Sense::Reverse,
            Sense::Reverse => Sense::Forward,
        };
    }
    for l in &mut body.parts.loops {
        l.coedges.reverse();
        for co in &mut l.coedges {
            co.reversed = !co.reversed;
        }
    }
    assert!(
        body.validate_boundary(&BoundaryValidationPolicy::default())
            .unwrap()
            .all_trims_checked()
    );
    assert!(matches!(
        body.validate_convex_solid(&BoundaryValidationPolicy::default()),
        Err(ConvexSolidError::Halfspace { .. }) | Err(ConvexSolidError::NoInterior)
    ));
}
#[test]
fn budget_refusal_precedes_expensive_arrangements() {
    let body = cube([0.0; 3], 4.0);
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy {
            max_pair_tests: 1,
            ..Default::default()
        }),
        Err(BoundaryValidationError::WorkLimit { maximum: 1, .. })
    ));
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy {
            max_coedges: 1,
            ..Default::default()
        }),
        Err(BoundaryValidationError::WorkLimit { maximum: 1, .. })
    ));
}

// Two plane faces share a complete polygon boundary. This is intentionally not
// a volume: it isolates arrangement checks from constructor profile validation.
fn double_face(points: Vec<[f64; 2]>, holes: Vec<Vec<[f64; 2]>>) -> Body {
    let mut parts = BodyParts::default();
    let mut loops = Vec::new();
    for points in core::iter::once(points).chain(holes) {
        let vo = parts.vertices.len();
        let eo = parts.edges.len();
        parts
            .vertices
            .extend(points.iter().map(|p| [p[0], p[1], 0.0]));
        for i in 0..points.len() {
            let a = [points[i][0], points[i][1], 0.0];
            let b = [
                points[(i + 1) % points.len()][0],
                points[(i + 1) % points.len()][1],
                0.0,
            ];
            parts.edges.push(Edge::new(
                Curve::line(a, exedra_math::sub(b, a)).unwrap(),
                ParameterRange::UNIT,
                VertexId(crate::usize_to_u32(vo + i)),
                VertexId(crate::usize_to_u32(vo + (i + 1) % points.len())),
                (),
            ));
        }
        let coedges = (0..points.len())
            .map(|i| Coedge {
                edge: EdgeId(crate::usize_to_u32(eo + i)),
                reversed: false,
            })
            .collect::<Vec<_>>();
        loops.push(coedges);
    }
    let mut face_loops = [Vec::new(), Vec::new()];
    for (f, ids) in face_loops.iter_mut().enumerate() {
        for coedges in &loops {
            let mut coedges = coedges.clone();
            if f == 1 {
                coedges.reverse();
                for co in &mut coedges {
                    co.reversed = true;
                }
            }
            ids.push(LoopId(crate::usize_to_u32(parts.loops.len())));
            parts.loops.push(BoundaryLoop { coedges });
        }
    }
    for (f, loops) in face_loops.into_iter().enumerate() {
        parts.faces.push(Face {
            surface: Surface::plane(Placement3::IDENTITY).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(-20.0, 20.0).unwrap(),
                v: ParameterRange::new(-20.0, 20.0).unwrap(),
            },
            sense: if f == 0 {
                Sense::Forward
            } else {
                Sense::Reverse
            },
            loops,
            role: (),
        });
    }
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}
#[test]
fn self_crossing_polygon_is_rejected_independently_of_incidence() {
    let body = double_face(vec![[0.0, 0.0], [5.0, 0.0], [0.0, 4.0], [4.0, 4.0]], vec![]);
    assert!(body.is_closed());
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::PolygonArrangement { .. })
            | Err(BoundaryValidationError::Winding { .. })
    ));
}
#[test]
fn nested_and_outside_holes_are_not_certified() {
    let outer = vec![[-10.0, -10.0], [10.0, -10.0], [10.0, 10.0], [-10.0, 10.0]];
    let hole = vec![[-4.0, -4.0], [-4.0, 4.0], [4.0, 4.0], [4.0, -4.0]];
    let nested = vec![[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
    let body = double_face(outer.clone(), vec![hole, nested]);
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::PolygonArrangement {
            error: exedra_profile::ProfileError::NestedHole { .. },
            ..
        })
    ));
    let outside = vec![[12.0, -1.0], [12.0, 1.0], [14.0, 1.0], [14.0, -1.0]];
    let body = double_face(outer, vec![outside]);
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::PolygonArrangement {
            error: exedra_profile::ProfileError::HoleOutside { .. },
            ..
        })
    ));
}
#[test]
fn duplicate_loop_ownership_is_rejected() {
    let mut body = cube([0.0; 3], 4.0);
    body.parts.faces.push(body.parts.faces[0].clone());
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::LoopOwnership { owners: 2, .. })
    ));
}

fn elliptic_double_face(ellipses: &[([f64; 2], [f64; 2])]) -> Body {
    let mut parts = BodyParts::default();
    for (i, (center, radii)) in ellipses.iter().enumerate() {
        let frame = Placement3::translate(center[0], center[1], 0.0);
        let curve = Curve::ellipse(frame, radii[0], radii[1]).unwrap();
        parts.vertices.push(curve.evaluate(0.0).unwrap().point);
        let vertex = VertexId(crate::usize_to_u32(i));
        parts.edges.push(Edge::new(
            curve,
            ParameterRange::new(0.0, core::f64::consts::TAU).unwrap(),
            vertex,
            vertex,
            (),
        ));
    }
    for f in 0..2 {
        let mut loops = Vec::new();
        for i in 0..ellipses.len() {
            loops.push(LoopId(crate::usize_to_u32(parts.loops.len())));
            parts.loops.push(BoundaryLoop {
                coedges: vec![Coedge {
                    edge: EdgeId(crate::usize_to_u32(i)),
                    reversed: (f == 1) != (i > 0),
                }],
            });
        }
        parts.faces.push(Face {
            surface: Surface::plane(Placement3::IDENTITY).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(-20.0, 20.0).unwrap(),
                v: ParameterRange::new(-20.0, 20.0).unwrap(),
            },
            sense: if f == 0 {
                Sense::Forward
            } else {
                Sense::Reverse
            },
            loops,
            role: (),
        });
    }
    let mut body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    crate::operation_trims::append_all_planar(&mut body.parts).unwrap();
    body
}
#[test]
fn elliptic_annulus_checks_containment_using_the_boundary_not_its_center() {
    let body = elliptic_double_face(&[([0.0; 2], [8.0, 4.0]), ([0.5, 0.0], [2.0, 1.0])]);
    assert!(
        body.validate_boundary(&BoundaryValidationPolicy::default())
            .unwrap()
            .all_trims_checked()
    );
    // Both centers are inside the outer ellipse, but the hole encloses the
    // supposed outer boundary. A center-only classifier would accept this.
    let wrong = elliptic_double_face(&[([0.0; 2], [2.0, 1.0]), ([0.0; 2], [8.0, 4.0])]);
    assert!(matches!(
        wrong.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::HoleContainment { .. })
    ));
}
#[test]
fn nested_elliptic_holes_and_crossing_ellipse_bounds_remain_distinct() {
    let nested = elliptic_double_face(&[
        ([0.0; 2], [10.0, 8.0]),
        ([0.0; 2], [5.0, 4.0]),
        ([0.0; 2], [2.0, 1.0]),
    ]);
    assert!(matches!(
        nested.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::HoleContainment { .. })
    ));
    let crossing = elliptic_double_face(&[
        ([0.0; 2], [10.0, 8.0]),
        ([-2.0, 0.0], [3.0, 2.0]),
        ([2.0, 0.0], [3.0, 2.0]),
    ]);
    let report = crossing
        .validate_boundary(&BoundaryValidationPolicy::default())
        .unwrap();
    assert!(!report.all_trims_checked());
    assert!(report.face_trims.iter().all(
        |status| *status == TrimValidation::Unsupported(UnsupportedTrim::SeparationUnresolved)
    ));
}

#[test]
fn stepped_bore_shoulders_and_floors_receive_independent_arrangement_proofs() {
    use crate::machining::{BoreDepth, BoreStep, BoreTool, subtract_bore};
    let stock = cube([-10.0, -10.0, 0.0], 20.0);
    let wedge = crate::halfspace::intersect_half_space(
        &stock,
        &crate::halfspace::HalfSpace::new([0.0, 0.0, 14.0], [0.15, -0.1, 1.0]).unwrap(),
        &crate::halfspace::ClipPolicy::default(),
    )
    .unwrap()
    .body;
    for reverse in [false, true] {
        for through in [false, true] {
            let entry = if reverse { [0.0, 0.0, 14.0] } else { [0.0; 3] };
            let direction = if reverse {
                [-0.1, 0.0, -1.0]
            } else {
                [0.1, 0.0, 1.0]
            };
            let steps = [
                BoreStep {
                    radius: 3.0,
                    end: BoreDepth::ToDepth(4.0),
                },
                BoreStep {
                    radius: 1.5,
                    end: if through {
                        BoreDepth::Through
                    } else {
                        BoreDepth::ToDepth(10.0)
                    },
                },
            ];
            let result = subtract_bore(
                &wedge,
                &BoreTool::new(entry, direction, &steps).unwrap(),
                &MachiningPolicy::default(),
            )
            .unwrap();
            let report = result
                .body
                .validate_boundary(&BoundaryValidationPolicy::default())
                .unwrap();
            assert!(report.all_trims_checked(), "{:?}", report.face_trims);
            assert_eq!(report.shells.len(), 1);
            assert_eq!(
                report.shells[0].euler_characteristic,
                if through { 0 } else { 2 }
            );
            for face in &result.report.walls {
                assert_eq!(
                    report.face_trims[face.index()],
                    TrimValidation::Certified(TrimProof::CylindricalGraphs)
                );
            }
            for face in result
                .report
                .shoulders
                .iter()
                .chain(result.report.floor.iter())
            {
                assert_eq!(
                    report.face_trims[face.index()],
                    TrimValidation::Certified(TrimProof::PlanarEllipses)
                );
            }
        }
    }
}

#[test]
fn collapsed_spherical_poles_do_not_create_extra_link_cycles_or_edges() {
    use crate::body::{LoopTrims, TrimCurve};
    use core::f64::consts::{FRAC_PI_2, PI, TAU};
    let line = |start, end| TrimCurve::Line { start, end };
    let parts = BodyParts {
        vertices: vec![[0.0, 0.0, -2.0], [0.0, 0.0, 2.0]],
        edges: vec![
            Edge::collapsed(VertexId(0), ()),
            Edge::collapsed(VertexId(1), ()),
            Edge::new(
                Curve::circle(
                    Placement3::from_axes(
                        [0.0, 0.0, -1.0],
                        [1.0, 0.0, 0.0],
                        [0.0, -1.0, 0.0],
                        [0.0; 3],
                    ),
                    2.0,
                )
                .unwrap(),
                ParameterRange::new(0.0, PI).unwrap(),
                VertexId(0),
                VertexId(1),
                (),
            ),
        ],
        loops: vec![BoundaryLoop {
            coedges: vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(2),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: true,
                },
                Coedge {
                    edge: EdgeId(2),
                    reversed: true,
                },
            ],
        }],
        faces: vec![Face {
            surface: Surface::sphere(Placement3::IDENTITY, 2.0).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(0.0, TAU).unwrap(),
                v: ParameterRange::new(-FRAC_PI_2, FRAC_PI_2).unwrap(),
            },
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        }],
        trims: vec![LoopTrims {
            loop_id: LoopId(0),
            curves: vec![
                line([0.0, -FRAC_PI_2], [TAU, -FRAC_PI_2]),
                line([TAU, -FRAC_PI_2], [TAU, FRAC_PI_2]),
                line([0.0, FRAC_PI_2], [TAU, FRAC_PI_2]),
                line([0.0, -FRAC_PI_2], [0.0, FRAC_PI_2]),
            ],
        }],
    };
    let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    let report = body
        .validate_boundary(&BoundaryValidationPolicy::default())
        .unwrap();
    assert!(report.all_trims_checked());
    assert_eq!(report.shells[0].vertices, 2);
    assert_eq!(report.shells[0].regular_edges, 1);
    assert_eq!(report.shells[0].euler_characteristic, 2);
    assert!(matches!(
        body.validate_convex_solid(&BoundaryValidationPolicy::default()),
        Err(ConvexSolidError::UnsupportedFace { .. })
    ));
}

#[test]
fn legacy_domain_and_finite_chart_precision_are_checked_explicitly() {
    let mut body = cube([0.0; 3], 4.0);
    body.parts.faces[0].domain.u = ParameterRange::new(0.25, 0.75).unwrap();
    body.validate(&ValidationPolicy { max_gap: 1e-9 }).unwrap();
    assert!(matches!(
        body.validate_boundary(&BoundaryValidationPolicy::default()),
        Err(BoundaryValidationError::ChartDomain { .. })
    ));
    let body = cube([0.0; 3], 4.0);
    let report = body
        .validate_boundary(&BoundaryValidationPolicy {
            chart_separation: 1e-15,
            ..Default::default()
        })
        .unwrap();
    assert!(!report.all_trims_checked());
    assert!(
        report
            .face_trims
            .iter()
            .all(|status| *status == TrimValidation::Unsupported(UnsupportedTrim::NumericLimit))
    );
}

#[test]
fn overlapping_passages_preserve_manifold_links_under_permutation_and_later_edits() {
    use crate::machining::{
        BoreDepth, BoreStep, BoreTool, subtract_bore, subtract_parallel_passages,
    };
    let source = cube([-10.0, -10.0, 0.0], 20.0);
    let specs = [
        ([-2.0, 0.0, 0.0], 3.0),
        ([2.0, 0.0, 0.0], 3.0),
        ([7.0, 7.0, 0.0], 1.0),
        ([-2.0, 0.0, 0.0], 0.5),
    ];
    let overlap = 18.0 * libm::acos(2.0 / 3.0) - 2.0 * libm::sqrt(20.0);
    let expected = 19.0 * core::f64::consts::PI - overlap;
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2]] {
        for sign in [-1.0, 1.0] {
            let tools =
                order.map(|i| CylinderTool::new(specs[i].0, [0.0, 0.0, sign], specs[i].1).unwrap());
            let result =
                subtract_parallel_passages(&source, &tools, &MachiningPolicy::default()).unwrap();
            assert!((result.report.removed_section_area - expected).abs() < 1e-10);
            assert_eq!(result.report.opening_components, 2);
            let report = result
                .body
                .validate_boundary(&BoundaryValidationPolicy::default())
                .unwrap();
            assert_eq!(report.shells.len(), 1);
            assert_eq!(report.shells[0].euler_characteristic, -2);
            for face in &result.report.walls {
                assert!(matches!(
                    report.face_trims[face.index()],
                    TrimValidation::Certified(_)
                ));
            }
            // Source mixed circular-arc arrangements deliberately remain unknown
            // rather than certifying a fitted profile as the native boundary.
            for face in result.report.trimmed_faces {
                assert_eq!(
                    report.face_trims[face.index()],
                    TrimValidation::Unsupported(UnsupportedTrim::CurveFamily)
                );
            }
            let blind = BoreTool::new(
                [-7.0, -7.0, 0.0],
                [0.0, 0.0, 1.0],
                &[BoreStep {
                    radius: 1.0,
                    end: BoreDepth::ToDepth(5.0),
                }],
            )
            .unwrap();
            let next = subtract_bore(&result.body, &blind, &MachiningPolicy::default()).unwrap();
            assert_eq!(
                next.body
                    .validate_boundary(&BoundaryValidationPolicy::default())
                    .unwrap()
                    .shells[0]
                    .euler_characteristic,
                -2
            );
        }
    }
}

#[test]
fn legacy_native_line_crossing_near_a_corner_is_not_hidden_by_vertex_gap_slack() {
    let mut body = cube([0.0; 3], 1.0);
    let edge = body
        .edges()
        .iter()
        .position(|edge| {
            body.vertices()[edge.start.index()] == [0.0; 3]
                && body.vertices()[edge.end.index()] == [1.0, 0.0, 0.0]
        })
        .unwrap();
    let (curve, _) = body.parts.edges[edge].regular_mut().unwrap();
    *curve = Curve::line([-2e-10, 2e-10, 0.0], [1.0 + 2e-10, -2e-10, 0.0]).unwrap();
    let geometry = body.validate(&ValidationPolicy { max_gap: 1e-9 }).unwrap();
    assert!(geometry.max_gap > 2e-10);
    let report = body
        .validate_boundary(&BoundaryValidationPolicy::default())
        .unwrap();
    assert!(!report.all_trims_checked());
    assert!(
        report
            .face_trims
            .contains(&TrimValidation::Unsupported(UnsupportedTrim::NumericLimit))
    );
    assert!(matches!(
        body.validate_convex_solid(&BoundaryValidationPolicy::default()),
        Err(ConvexSolidError::UnsupportedFace { .. })
    ));
}
