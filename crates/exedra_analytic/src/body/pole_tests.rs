// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use alloc::vec;
use core::f64::consts::{FRAC_PI_2, PI, TAU};

fn sphere_parts() -> BodyParts {
    let radius = 2.0;
    BodyParts {
        trims: Vec::new(),
        vertices: vec![[0.0, 0.0, -radius], [0.0, 0.0, radius]],
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
                    radius,
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
            surface: Surface::sphere(Placement3::IDENTITY, radius).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(0.0, TAU).unwrap(),
                v: ParameterRange::new(-FRAC_PI_2, FRAC_PI_2).unwrap(),
            },
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        }],
    }
}
const POLICY: ValidationPolicy = ValidationPolicy { max_gap: 1e-10 };

#[test]
fn collapsed_poles_are_authoritative_points_distinct_from_closed_regular_curves() {
    let body = Body::try_from_parts(sphere_parts(), &POLICY).unwrap();
    assert!(body.is_closed());
    for edge in [EdgeId(0), EdgeId(1)] {
        assert!(body.edge(edge).unwrap().is_collapsed());
        assert!(body.edge(edge).unwrap().regular().is_none());
        assert_eq!(body.edge_length(edge), Some(0.0));
        for fraction in [0.0, 0.3, 1.0] {
            assert_eq!(
                body.edge_point(edge, fraction),
                Some(body.vertices()[edge.index()])
            );
        }
        assert!(body.edge_point(edge, f64::NAN).is_none());
        assert!(body.edge_point(edge, -0.1).is_none());
    }
    let closed = Edge::new(
        Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
        ParameterRange::new(0.0, TAU).unwrap(),
        VertexId(0),
        VertexId(0),
        (),
    );
    assert!(!closed.is_collapsed());
    assert!(closed.regular().is_some());
}

#[test]
fn collapsed_boundary_refuses_distinct_endpoints_nonpole_faces_and_nonpole_domains() {
    let mut parts = sphere_parts();
    parts.edges[0].end = VertexId(1);
    assert_eq!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::CollapsedEndpoints { edge: EdgeId(0) })
    );
    let mut parts = sphere_parts();
    parts.faces[0].surface = Surface::plane(Placement3::IDENTITY).unwrap();
    assert!(matches!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::UnsupportedCollapsedBoundary {
            face: FaceId(0),
            edge: EdgeId(0)
        })
    ));
    let mut parts = sphere_parts();
    parts.faces[0].domain.v = ParameterRange::new(-0.4, 0.4).unwrap();
    assert!(matches!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::UnsupportedCollapsedBoundary {
            face: FaceId(0),
            edge: EdgeId(0)
        })
    ));
}

#[test]
fn collapsed_boundary_refuses_repeated_uses_and_support_disagreement() {
    let mut parts = sphere_parts();
    parts.loops[0].coedges.insert(
        0,
        Coedge {
            edge: EdgeId(0),
            reversed: false,
        },
    );
    assert!(matches!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::CollapsedIncidence {
            edge: EdgeId(0),
            uses: 2
        })
    ));
    let mut parts = sphere_parts();
    parts.faces[0].surface = Surface::sphere(Placement3::translate(0.0, 0.0, 1e-5), 2.0).unwrap();
    assert!(matches!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::SurfaceGap {
            face: FaceId(0),
            edge: EdgeId(0),
            ..
        })
    ));
}

#[test]
fn explicit_sphere_trims_keep_distinct_pole_parameters_and_seam_uses() {
    let mut parts = sphere_parts();
    let line = |start, end| TrimCurve::Line { start, end };
    parts.trims = vec![LoopTrims {
        loop_id: LoopId(0),
        curves: vec![
            line([0.0, -FRAC_PI_2], [TAU, -FRAC_PI_2]),
            line([TAU, -FRAC_PI_2], [TAU, FRAC_PI_2]),
            line([0.0, FRAC_PI_2], [TAU, FRAC_PI_2]),
            line([0.0, -FRAC_PI_2], [0.0, FRAC_PI_2]),
        ],
    }];
    let body = Body::try_from_parts(parts, &POLICY).unwrap();
    let report = body.validate(&POLICY).unwrap();
    assert_eq!(report.trimmed_coedges, 4);
    assert!(report.max_trim_gap <= POLICY.max_gap);
    assert!(body.is_closed());
    let output = body
        .realize(
            Placement3::IDENTITY,
            &crate::realize::RealizationPolicy::default(),
        )
        .unwrap();
    assert!(output.report.within_tolerance());
    assert!(output.mesh.boundary_loops().unwrap().is_empty());
    assert!(!output.pole_corners.is_empty());
}

#[test]
fn cone_apex_trim_is_a_collapsed_position_not_a_fictitious_curve() {
    let range = ParameterRange::new(0.0, TAU).unwrap();
    let line = |start, end| TrimCurve::Line { start, end };
    let mut parts = BodyParts {
        vertices: vec![[2.0, 0.0, 0.0], [0.0, 0.0, 3.0]],
        edges: vec![
            Edge::new(
                Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
                range,
                VertexId(0),
                VertexId(0),
                (),
            ),
            Edge::new(
                Curve::line([2.0, 0.0, 0.0], [-2.0, 0.0, 3.0]).unwrap(),
                ParameterRange::UNIT,
                VertexId(0),
                VertexId(1),
                (),
            ),
            Edge::collapsed(VertexId(1), ()),
        ],
        loops: vec![BoundaryLoop {
            coedges: vec![
                Coedge {
                    edge: EdgeId(0),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: false,
                },
                Coedge {
                    edge: EdgeId(2),
                    reversed: true,
                },
                Coedge {
                    edge: EdgeId(1),
                    reversed: true,
                },
            ],
        }],
        faces: vec![Face {
            surface: Surface::cone(Placement3::IDENTITY, 2.0, -2.0 / 3.0).unwrap(),
            domain: ParameterDomain {
                u: range,
                v: ParameterRange::new(0.0, 3.0).unwrap(),
            },
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        }],
        trims: vec![LoopTrims {
            loop_id: LoopId(0),
            curves: vec![
                line([0.0, 0.0], [TAU, 0.0]),
                line([TAU, 0.0], [TAU, 3.0]),
                line([0.0, 3.0], [TAU, 3.0]),
                line([0.0, 0.0], [0.0, 3.0]),
            ],
        }],
    };
    let body = Body::try_from_parts(parts.clone(), &POLICY).unwrap();
    assert_eq!(body.validate(&POLICY).unwrap().trimmed_coedges, 4);
    assert!(body.evaluate_support(FaceId(0), [1.0, 3.0]).is_none());
    parts.trims[0].curves[2] = line([0.0, 2.99], [TAU, 2.99]);
    assert!(matches!(
        Body::try_from_parts(parts, &POLICY),
        Err(BodyError::TrimCorrespondence { coedge: 2, .. }) | Err(BodyError::TrimClosure { .. })
    ));
}
