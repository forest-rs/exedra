// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::geometry::Curve;
use crate::{
    body::{EdgeGeometry, Sense},
    extrusion::Extrusion,
    machining::{
        BoreDepth, BoreStep, BoreTool, CylinderTool, MachiningPolicy, subtract_bore,
        subtract_parallel_passages, subtract_through_cylinder,
    },
};
use alloc::{vec, vec::Vec};
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};

fn stock() -> Body {
    let outline = Loop::new(vec![
        Segment::line([30.0, -20.0]),
        Segment::line([30.0, 20.0]),
        Segment::line([-30.0, 20.0]),
        Segment::line([-30.0, -20.0]),
    ])
    .unwrap();
    Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        20.0,
    )
    .unwrap()
    .body()
    .clone()
    .map_roles(|_| (), |_| ())
}
fn valve() -> Body {
    let policy = MachiningPolicy::default();
    let tool = BoreTool::new(
        [15.0, 0.0, 20.0],
        [0.0, 0.0, -1.0],
        &[
            BoreStep {
                radius: 5.0,
                end: BoreDepth::ToDepth(6.0),
            },
            BoreStep {
                radius: 2.0,
                end: BoreDepth::ToDepth(12.0),
            },
        ],
    )
    .unwrap();
    let bore = subtract_bore(&stock(), &tool, &policy).unwrap().body;
    subtract_parallel_passages(
        &bore,
        &[
            CylinderTool::new([-13.0, 0.0, 0.0], [0.0, 0.0, 1.0], 4.0).unwrap(),
            CylinderTool::new([-7.0, 0.0, 0.0], [0.0, 0.0, 1.0], 4.0).unwrap(),
        ],
        &policy,
    )
    .unwrap()
    .body
}
fn location(query: &SolidQuery<'_>, point: [f64; 3]) -> PointLocation {
    query
        .query_point(point)
        .unwrap_or_else(|error| panic!("{point:?}: {error}"))
        .location
}
#[test]
fn stock_material_exterior_faces_edges_and_vertices() {
    let body = stock();
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    for (point, expected) in [
        ([0.0, 0.0, 10.0], PointLocation::Inside),
        ([31.0, 0.0, 10.0], PointLocation::Outside),
        ([0.0, 0.0, -1.0], PointLocation::Outside),
        ([30.0, 0.0, 10.0], PointLocation::Boundary),
        ([30.0, 20.0, 10.0], PointLocation::Boundary),
        ([30.0, 20.0, 20.0], PointLocation::Boundary),
        ([0.0, 0.0, 20.0], PointLocation::Boundary),
    ] {
        assert_eq!(location(&query, point), expected, "{point:?}");
    }
    let t = query.policy.boundary_tolerance;
    for point in [
        [30.0 + 0.4 * t, 0.0, 20.0 + 0.4 * t],
        [30.0 + 0.4 * t, 20.0 + 0.4 * t, 20.0 + 0.4 * t],
    ] {
        assert_eq!(location(&query, point), PointLocation::Boundary);
    }
    assert_eq!(
        location(&query, [30.0 + 2.0 * t, 20.0 + 2.0 * t, 20.0 + 2.0 * t]),
        PointLocation::Outside
    );
}
#[test]
fn joined_passages_and_stepped_blind_bore_are_classified_without_feature_history() {
    let body = valve();
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    for (point, expected) in [
        ([0.0, 0.0, 10.0], PointLocation::Inside),
        ([-13.0, 0.0, 10.0], PointLocation::Outside),
        ([-10.0, 0.0, 10.0], PointLocation::Outside),
        ([15.0, 0.0, 18.0], PointLocation::Outside),
        ([18.0, 0.0, 10.0], PointLocation::Inside),
        ([15.0, 0.0, 10.0], PointLocation::Outside),
        ([15.0, 0.0, 7.0], PointLocation::Inside),
        ([18.0, 0.0, 14.0], PointLocation::Boundary),
        ([15.0, 0.0, 8.0], PointLocation::Boundary),
        ([20.0, 0.0, 17.0], PointLocation::Boundary),
        ([-10.0, libm::sqrt(7.0), 10.0], PointLocation::Boundary),
    ] {
        assert_eq!(location(&query, point), expected, "{point:?}");
    }
    let t = query.policy.boundary_tolerance;
    assert_eq!(
        location(&query, [20.0 - 0.4 * t, 0.0, 20.0 + 0.4 * t]),
        PointLocation::Boundary
    );
    assert!(query.report().owned_capacity_bytes > 0);
    assert!(query.report().trim_curves > 0);
    // Independent membership oracle: the authored box minus two disks and a
    // finite stepped bore. No trim, ray, tessellation or query helper is reused.
    for x in -31..=31 {
        for y in -21..=21 {
            for z in [-1.3, 3.3, 9.3, 15.3, 19.3, 21.3] {
                let p = [f64::from(x) + 0.137, f64::from(y) + 0.219, z];
                let in_box = p[0].abs() < 30.0 && p[1].abs() < 20.0 && p[2] > 0.0 && p[2] < 20.0;
                let joined = (p[0] + 13.0).powi(2) + p[1] * p[1] < 16.0
                    || (p[0] + 7.0).powi(2) + p[1] * p[1] < 16.0;
                let radius2 = (p[0] - 15.0).powi(2) + p[1] * p[1];
                let bore = (p[2] > 14.0 && radius2 < 25.0) || (p[2] > 8.0 && radius2 < 4.0);
                assert_eq!(
                    location(&query, p),
                    if in_box && !joined && !bore {
                        PointLocation::Inside
                    } else {
                        PointLocation::Outside
                    },
                    "{p:?}"
                );
            }
        }
    }
}
#[test]
fn oblique_elliptic_rims_and_periodic_cylinder_seam() {
    let direction = [0.3, 0.1, 1.0];
    let tool = CylinderTool::new([0.0; 3], direction, 2.0).unwrap();
    let body = subtract_through_cylinder(&stock(), &tool, &MachiningPolicy::default())
        .unwrap()
        .body;
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    assert_eq!(location(&query, [3.0, 1.0, 10.0]), PointLocation::Outside);
    assert_eq!(location(&query, [0.0, 10.0, 10.0]), PointLocation::Inside);
    let wall = body
        .faces()
        .iter()
        .position(|f| matches!(f.surface, Surface::Cylinder(_)))
        .unwrap();
    let face = &body.faces()[wall];
    let band = body
        .cylindrical_band(
            FaceId(crate::usize_to_u32(wall)),
            &ValidationPolicy { max_gap: 1e-9 },
        )
        .unwrap();
    let uv = [
        face.domain.u.bounds()[0],
        band.axial_interval(face.domain.u.bounds()[0])
            .unwrap()
            .at(0.5)
            .unwrap(),
    ];
    assert_eq!(
        location(&query, face.surface.evaluate(uv).unwrap().point),
        PointLocation::Boundary
    );
    let rim = band.rim_edges()[0];
    assert_eq!(
        location(&query, body.edge_point(rim, 0.317).unwrap()),
        PointLocation::Boundary
    );
}
fn placed(mut body: Body, frame: Placement3) -> Body {
    let mut parts = body.into_parts();
    for vertex in &mut parts.vertices {
        *vertex = frame.transform_point(*vertex);
    }
    for edge in &mut parts.edges {
        if let EdgeGeometry::Curve { curve, .. } = &mut edge.geometry {
            *curve = match curve {
                Curve::Line(l) => Curve::line(
                    frame.transform_point(l.origin()),
                    frame.transform_vector(l.direction()),
                )
                .unwrap(),
                Curve::Circle(c) => Curve::circle(frame * c.frame(), c.radius()).unwrap(),
                Curve::Ellipse(e) => {
                    Curve::ellipse(frame * e.frame(), e.semi_major(), e.semi_minor()).unwrap()
                }
            };
        }
    }
    for face in &mut parts.faces {
        face.surface = match &face.surface {
            Surface::Plane(p) => Surface::plane(frame * p.frame()).unwrap(),
            Surface::Cylinder(c) => Surface::cylinder(frame * c.frame(), c.radius()).unwrap(),
            _ => unreachable!(),
        };
    }
    body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    body
}
#[test]
fn rotated_translated_geometry_keeps_membership_and_roles_are_irrelevant() {
    let body = valve();
    let frame = Placement3::rotate_y_then_translate(0.731, 90.0, -73.0, 41.0);
    let moved = placed(body, frame);
    let query = SolidQuery::prepare(&moved, &SolidQueryPolicy::default()).unwrap();
    for (p, expected) in [
        ([0.0, 0.0, 10.0], PointLocation::Inside),
        ([-10.0, 0.0, 10.0], PointLocation::Outside),
        ([15.0, 0.0, 7.0], PointLocation::Inside),
        ([18.0, 0.0, 14.0], PointLocation::Boundary),
        ([40.0, 0.0, 10.0], PointLocation::Outside),
    ] {
        assert_eq!(location(&query, frame.transform_point(p)), expected);
    }
    let roleless = moved.map_roles(|_| Vec::<u8>::new(), |_| "arbitrary");
    assert_eq!(
        SolidQuery::prepare(&roleless, &SolidQueryPolicy::default())
            .unwrap()
            .query_point(frame.transform_point([0.0, 0.0, 10.0]))
            .unwrap()
            .location,
        PointLocation::Inside
    );
}
#[test]
fn bounded_work_numeric_and_unsupported_refusals_are_explicit() {
    let body = stock();
    let policy = SolidQueryPolicy {
        max_query_work: 1,
        ..Default::default()
    };
    let query = SolidQuery::prepare(&body, &policy).unwrap();
    assert!(
        matches!(query.query_point([0.0,0.0,10.0]), Err(QueryError::WorkLimit { work, maximum: 1 }) if work.total()==1)
    );
    let mut policy = SolidQueryPolicy::default();
    policy.validation.max_coedges = 1;
    assert!(matches!(
        SolidQuery::prepare(&body, &policy),
        Err(QueryError::BoundaryValidation(
            BoundaryValidationError::WorkLimit { .. }
        ))
    ));
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    assert!(matches!(
        query.query_point([f64::NAN, 0.0, 0.0]),
        Err(QueryError::NonFinitePoint)
    ));
    assert!(matches!(
        query.query_point([1e12, 0.0, 0.0]),
        Err(QueryError::NumericLimit { .. })
    ));
    let t = query.policy.boundary_tolerance;
    assert!(matches!(
        query.query_point([30.0 + t, 0.0, 10.0]),
        Err(QueryError::Ambiguous {
            reason: Ambiguity::BoundaryTolerance,
            ..
        })
    ));
}
#[test]
fn a_reversed_shell_is_not_reported_as_material() {
    let mut parts = stock().into_parts();
    for face in &mut parts.faces {
        face.sense = match face.sense {
            Sense::Forward => Sense::Reverse,
            Sense::Reverse => Sense::Forward,
        };
    }
    for loop_ in &mut parts.loops {
        loop_.coedges.reverse();
        for co in &mut loop_.coedges {
            co.reversed = !co.reversed;
        }
    }
    let body = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap();
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    assert!(matches!(
        query.query_point([0.0, 0.0, 10.0]),
        Err(QueryError::InconsistentWinding { winding: -1, .. })
    ));
}

#[test]
fn analytic_tangency_retries_instead_of_discarding_a_root() {
    let body = valve();
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    let direction = [1.0, 0.371, 0.529];
    let direction = scale(direction, 1.0 / norm(direction));
    let transverse = libm::hypot(direction[0], direction[1]);
    let tangent = [
        -13.0 - 4.0 * direction[1] / transverse,
        4.0 * direction[0] / transverse,
        10.0,
    ];
    let point = sub(tangent, scale(direction, 5.0));
    let result = query.query_point(point).unwrap();
    assert_eq!(result.location, PointLocation::Inside);
    assert!(
        result.work.ray_attempts > 1,
        "tangent support root must not disappear"
    );
    let limited = SolidQuery::prepare(
        &body,
        &SolidQueryPolicy {
            max_ray_attempts: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(
        limited.query_point(point),
        Err(QueryError::Ambiguous {
            reason: Ambiguity::RayDegeneracy,
            ..
        })
    ));
}

#[test]
fn closed_cavity_shells_follow_material_orientation() {
    use crate::rectilinear::{BooleanOperation, RectilinearPolicy, boolean_rectilinear};
    let outline = Loop::new(vec![
        Segment::line([5.0, -5.0]),
        Segment::line([5.0, 5.0]),
        Segment::line([-5.0, 5.0]),
        Segment::line([-5.0, -5.0]),
    ])
    .unwrap();
    let inner = Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        8.0,
    )
    .unwrap()
    .body()
    .clone()
    .map_roles(|_| (), |_| ());
    let inner = placed(inner, Placement3::translate(0.0, 0.0, 6.0));
    let cavity = boolean_rectilinear(
        &stock(),
        &inner,
        BooleanOperation::Difference,
        &RectilinearPolicy::default(),
    )
    .unwrap()
    .body;
    let query = SolidQuery::prepare(&cavity, &SolidQueryPolicy::default()).unwrap();
    assert_eq!(query.boundary_validation().shells.len(), 2);
    for (point, expected) in [
        ([0.0, 0.0, 10.0], PointLocation::Outside),
        ([10.0, 0.0, 10.0], PointLocation::Inside),
        ([0.0, 0.0, 3.0], PointLocation::Inside),
        ([5.0, 0.0, 10.0], PointLocation::Boundary),
    ] {
        assert_eq!(location(&query, point), expected);
    }
}

#[test]
fn missing_cylindrical_charts_are_not_replaced_by_the_domain_rectangle() {
    let circle = Loop::new(vec![
        Segment::arc([-5.0, 0.0], 1.0),
        Segment::arc([5.0, 0.0], 1.0),
    ])
    .unwrap();
    let cylinder = Extrusion::new(
        Profile::new(circle, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        10.0,
    )
    .unwrap();
    assert!(matches!(
        SolidQuery::prepare(cylinder.body(), &SolidQueryPolicy::default()),
        Err(QueryError::UnsupportedTrim { .. })
    ));
}

#[test]
fn infinite_support_extensions_do_not_create_boundary_degeneracies() {
    let tool = CylinderTool::new([0.0; 3], [0.0, 0.0, 1.0], 2.0).unwrap();
    let body = subtract_through_cylinder(&stock(), &tool, &MachiningPolicy::default())
        .unwrap()
        .body;
    let query = SolidQuery::prepare(&body, &SolidQueryPolicy::default()).unwrap();
    for p in [
        [2.0, 0.0, 100.0],
        [2.0, 0.0, -100.0],
        [100.0, 0.0, 20.0],
        [100.0, 0.0, 0.0],
        [30.0, 100.0, 10.0],
    ] {
        assert_eq!(location(&query, p), PointLocation::Outside, "{p:?}");
    }
}

#[test]
fn eccentric_partial_ellipse_near_tangent_endpoint_refuses_uncertain_crossing() {
    let end = core::f64::consts::FRAC_PI_2 - 1e-4;
    let arc = Curve2::Conic {
        center: [0.0; 2],
        cosine: [100.0, 0.0],
        sine: [0.0, 1.0],
        range: crate::geometry::ParameterRange::new(0.0, end).unwrap(),
        reversed: false,
    };
    // The computed root is ~5e-9 radians beyond the end, although its Y
    // displacement is within the 1e-12 allowance. allowance/radius would skip
    // it as certainly outside; sensitivity requires an ambiguous retry.
    let point = [-200.0, libm::sin(end) + 5e-13];
    assert_eq!(arc.crossing(point, [1.0, 0.0], 1e-12), None);
    let start = core::f64::consts::FRAC_PI_2 + 1e-4;
    let arc = Curve2::Conic {
        center: [0.0; 2],
        cosine: [100.0, 0.0],
        sine: [0.0, 1.0],
        range: crate::geometry::ParameterRange::new(start, core::f64::consts::PI).unwrap(),
        reversed: false,
    };
    assert_eq!(
        arc.crossing([-200.0, libm::sin(start) + 5e-13], [1.0, 0.0], 1e-12),
        None
    );
}

#[test]
fn prepared_inverse_preserves_native_inverse_arithmetic() {
    for frame in [
        Placement3::IDENTITY,
        Placement3::rotate_y_then_translate(0.731, 90.0, -73.0, 41.0),
        Placement3::from_axes(
            [1.0, 2e-14, 0.0],
            [-1e-14, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1e6, -2e6, 3e6],
        ),
    ] {
        let inverse = LocalFrame::new(frame).unwrap();
        for point in [[0.0, -0.0, 0.0], [1.3, -5.7, 9.1], [1e6, 2e6, -1e6]] {
            let native = crate::geometry::frame_coordinates(frame, point).unwrap();
            assert_eq!(
                inverse.point(point).unwrap().map(f64::to_bits),
                native.map(f64::to_bits)
            );
            let zero = Placement3::from_axes(
                frame.rows.map(|r| r[0]),
                frame.rows.map(|r| r[1]),
                frame.rows.map(|r| r[2]),
                [0.0; 3],
            );
            let native = crate::geometry::frame_coordinates(zero, point).unwrap();
            assert_eq!(
                inverse.vector(point).unwrap().map(f64::to_bits),
                native.map(f64::to_bits)
            );
        }
    }
}
