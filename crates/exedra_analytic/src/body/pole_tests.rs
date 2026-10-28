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
