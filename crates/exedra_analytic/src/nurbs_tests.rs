// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! NURBS edges through the kernel: validation, trims, bounds, ruled walls
//! and realization.

use alloc::vec;
use alloc::vec::Vec;

use exedra_math::{Placement3, norm, sub};

use crate::body::{
    Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeGeometry, EdgeId, Face, LoopId, Sense,
    TrimCurve, ValidationPolicy, VertexId,
};
use crate::geometry::{
    Curve, CurveSpan, GeometryError, NurbsCurve, ParameterDomain, ParameterRange, Surface,
};
use crate::realize::RealizationPolicy;

/// A rational cubic from `(0, 0)` to `(4, 0)` bulging toward `+y`, at
/// height `z`, over the domain `[0, 2]`.
pub(crate) fn bulge(z: f64) -> NurbsCurve {
    NurbsCurve::new(
        4,
        vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0],
        vec![
            [0.0, 0.0, z],
            [0.5, 2.0, z],
            [2.0, 2.5, z],
            [3.5, 2.0, z],
            [4.0, 0.0, z],
        ],
        Some(vec![1.0, 0.8, 1.5, 0.8, 1.0]),
    )
    .unwrap()
}

fn coedge(edge: u32, reversed: bool) -> Coedge {
    Coedge {
        edge: EdgeId(edge),
        reversed,
    }
}

fn edge(curve: Curve, range: ParameterRange, start: u32, end: u32) -> Edge<()> {
    Edge {
        geometry: EdgeGeometry::Curve { curve, range },
        start: VertexId(start),
        end: VertexId(end),
        role: (),
    }
}

/// One planar face bounded by the bulge and the line back along `x`.
pub(crate) fn d_face() -> BodyParts<(), ()> {
    let spline = Curve::nurbs(bulge(0.0)).unwrap();
    let domain = ParameterRange::new(0.0, 2.0).unwrap();
    BodyParts {
        trims: Vec::new(),
        vertices: vec![[0.0; 3], [4.0, 0.0, 0.0]],
        edges: vec![
            edge(spline, domain, 0, 1),
            edge(
                Curve::line([4.0, 0.0, 0.0], [-4.0, 0.0, 0.0]).unwrap(),
                ParameterRange::UNIT,
                1,
                0,
            ),
        ],
        // Counterclockwise from above: along the line, back over the bulge.
        loops: vec![BoundaryLoop {
            coedges: vec![coedge(1, true), coedge(0, true)],
        }],
        faces: vec![Face {
            surface: Surface::plane(Placement3::IDENTITY).unwrap(),
            domain: ParameterDomain {
                u: ParameterRange::new(-1.0, 5.0).unwrap(),
                v: ParameterRange::new(-1.0, 3.0).unwrap(),
            },
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        }],
    }
}

const POLICY: ValidationPolicy = ValidationPolicy { max_gap: 1e-9 };

#[test]
fn nurbs_edges_need_continuous_curves_and_their_domain() {
    // A curve that may jump at its middle knot is refused as an edge.
    let jump = NurbsCurve::new(
        2,
        vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0],
        vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [2.0, 1.0, 0.0]],
        None,
    )
    .unwrap();
    assert_eq!(
        Curve::nurbs(jump),
        Err(GeometryError::Discontinuous { knot: 1.0 })
    );
    let curve = Curve::nurbs(bulge(0.0)).unwrap();
    assert!(curve.evaluate(-0.1).is_none() && curve.evaluate(2.1).is_none());
    assert_eq!(curve.evaluate(2.0).unwrap().point, [4.0, 0.0, 0.0]);
    assert_eq!(curve.period(), None);
    // Moving the curve moves its controls and keeps its weights.
    let frame = Placement3::translate(1.0, 2.0, 3.0);
    let moved = curve.transformed(frame).unwrap();
    for t in [0.0, 0.3, 1.0, 1.7] {
        let (a, b) = (curve.evaluate(t).unwrap(), moved.evaluate(t).unwrap());
        assert!(norm(sub(frame.transform_point(a.point), b.point)) < 1e-14);
    }
}

#[test]
fn ruled_patches_refuse_spans_that_turn_corners() {
    // Each span of this polyline is straight, so its second derivative is
    // zero, but its tangent jumps at 0.25: a curvature bound would let one
    // flat cell stand for a wall a unit away from it there.
    let kinked = NurbsCurve::new(
        2,
        vec![0.0, 0.0, 0.25, 1.0, 1.0],
        vec![[0.0; 3], [0.25, 1.0, 0.0], [1.0, 0.0, 0.0]],
        None,
    )
    .unwrap();
    let a = CurveSpan::new(Curve::nurbs(kinked).unwrap(), ParameterRange::UNIT, false).unwrap();
    let b = a.transformed(Placement3::translate(0.0, 0.0, 1.0)).unwrap();
    assert_eq!(
        Surface::ruled(a.clone(), b.clone()),
        Err(GeometryError::Kinked { knot: 0.25 })
    );
    // Either side of the corner alone is a regular patch.
    for range in [[0.0, 0.25], [0.25, 1.0]] {
        let range = ParameterRange::new(range[0], range[1]).unwrap();
        let side = |s: &CurveSpan| CurveSpan::new(s.curve().clone(), range, false).unwrap();
        assert!(Surface::ruled(side(&a), side(&b)).is_ok());
    }
}

#[test]
fn a_planar_face_bounded_by_a_spline_validates_with_its_trims() {
    let body = Body::try_from_parts(d_face(), &POLICY).unwrap();
    let mut parts = body.into_parts();
    crate::operation_trims::append_all_planar(&mut parts).unwrap();
    let TrimCurve::Nurbs { curve, range } = &parts.trims[0].curves[1] else {
        panic!("{:?}", parts.trims[0].curves[1]);
    };
    // On the XY plane the chart curve is the spline's own XY.
    assert_eq!(range.bounds(), [0.0, 2.0]);
    for (uv, p) in curve.points().iter().zip(bulge(0.0).points()) {
        assert_eq!(*uv, [p[0], p[1]]);
    }
    let body = Body::try_from_parts(parts.clone(), &POLICY).unwrap();
    let bounds = Curve::nurbs(bulge(0.0))
        .unwrap()
        .conservative_bounds(ParameterRange::new(0.0, 2.0).unwrap())
        .unwrap();
    assert!(bounds[0][1] <= 0.0 && bounds[1][1] >= 2.0 && bounds[1][1] <= 2.6);
    drop(body);
    // A trim that strays from its edge is refused.
    let TrimCurve::Nurbs { curve, range } = parts.trims[0].curves[1].clone() else {
        unreachable!()
    };
    let mut points = curve.points().to_vec();
    points[2][1] += 1e-3;
    parts.trims[0].curves[1] = TrimCurve::Nurbs {
        curve: NurbsCurve::new(
            curve.order(),
            curve.knots().to_vec(),
            points,
            curve.weights().map(<[f64]>::to_vec),
        )
        .unwrap(),
        range,
    };
    assert!(Body::try_from_parts(parts, &POLICY).is_err());
    // An edge lifted off its plane is refused.
    let mut lifted = d_face();
    lifted.edges[0].geometry = EdgeGeometry::Curve {
        curve: Curve::nurbs(bulge(1e-3)).unwrap(),
        range: ParameterRange::new(0.0, 2.0).unwrap(),
    };
    assert!(Body::try_from_parts(lifted, &POLICY).is_err());
}

#[test]
fn spline_edges_are_realized_within_tolerance() {
    let body = Body::try_from_parts(d_face(), &POLICY).unwrap();
    for tolerance in [0.05, 1e-3, 1e-4] {
        let policy = RealizationPolicy {
            tolerance,
            ..RealizationPolicy::default()
        };
        let realized = body.realize(Placement3::IDENTITY, &policy).unwrap();
        assert!(realized.report.within_tolerance(), "{:?}", realized.report);
        assert!(realized.report.max_curve_deviation <= tolerance);
        // Every boundary vertex of the spline's chain lies on the spline.
        let curve = Curve::nurbs(bulge(0.0)).unwrap();
        let chain = realized.edge_chains.get(EdgeId(0)).unwrap();
        assert!(chain.len() > 2);
        for (k, &v) in chain.iter().enumerate() {
            let t = 2.0 * k as f64 / (chain.len() - 1) as f64;
            let expected = curve.evaluate(t).unwrap().point;
            let got = realized.mesh.vertex_position(v).unwrap().map(f64::from);
            assert!(
                norm(sub(expected, got)) < 1e-5,
                "{k}: {got:?} against {expected:?}"
            );
        }
    }
}

#[test]
fn a_prism_over_a_spline_is_a_closed_solid() {
    let range = ParameterRange::new(0.0, 2.0).unwrap();
    let span = |z| CurveSpan::new(Curve::nurbs(bulge(z)).unwrap(), range, false).unwrap();
    let wall = Surface::ruled(span(0.0), span(1.0)).unwrap();
    let line =
        |a: [f64; 3], b: [f64; 3]| Curve::line(a, core::array::from_fn(|k| b[k] - a[k])).unwrap();
    let parts: BodyParts<(), ()> = BodyParts {
        trims: Vec::new(),
        vertices: vec![[0.0; 3], [4.0, 0.0, 0.0], [0.0, 0.0, 1.0], [4.0, 0.0, 1.0]],
        edges: vec![
            edge(Curve::nurbs(bulge(0.0)).unwrap(), range, 0, 1),
            edge(Curve::nurbs(bulge(1.0)).unwrap(), range, 2, 3),
            edge(line([4.0, 0.0, 0.0], [0.0; 3]), ParameterRange::UNIT, 1, 0),
            edge(
                line([4.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
                ParameterRange::UNIT,
                3,
                2,
            ),
            edge(line([0.0; 3], [0.0, 0.0, 1.0]), ParameterRange::UNIT, 0, 2),
            edge(
                line([4.0, 0.0, 0.0], [4.0, 0.0, 1.0]),
                ParameterRange::UNIT,
                1,
                3,
            ),
        ],
        loops: vec![
            // Bottom, counterclockwise seen from below.
            BoundaryLoop {
                coedges: vec![coedge(0, false), coedge(2, false)],
            },
            // Top, counterclockwise seen from above.
            BoundaryLoop {
                coedges: vec![coedge(3, true), coedge(1, true)],
            },
            // The curved wall: its patch's normal points in, so the face
            // is reversed and its loop runs clockwise in the patch's chart.
            BoundaryLoop {
                coedges: vec![
                    coedge(4, false),
                    coedge(1, false),
                    coedge(5, true),
                    coedge(0, true),
                ],
            },
            // The flat wall under the line, facing -y.
            BoundaryLoop {
                coedges: vec![
                    coedge(2, true),
                    coedge(5, false),
                    coedge(3, false),
                    coedge(4, true),
                ],
            },
        ],
        faces: vec![
            Face {
                surface: Surface::plane(Placement3::IDENTITY).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(-1.0, 5.0).unwrap(),
                    v: ParameterRange::new(-1.0, 3.0).unwrap(),
                },
                sense: Sense::Reverse,
                loops: vec![LoopId(0)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::translate(0.0, 0.0, 1.0)).unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(-1.0, 5.0).unwrap(),
                    v: ParameterRange::new(-1.0, 3.0).unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(1)],
                role: (),
            },
            Face {
                surface: wall,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Reverse,
                loops: vec![LoopId(2)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::from_axes(
                    [1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0],
                    [0.0, -1.0, 0.0],
                    [0.0; 3],
                ))
                .unwrap(),
                domain: ParameterDomain {
                    u: ParameterRange::new(-1.0, 5.0).unwrap(),
                    v: ParameterRange::new(-1.0, 2.0).unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(3)],
                role: (),
            },
        ],
    };
    let body = Body::try_from_parts(parts, &POLICY).unwrap();
    let realized = body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    assert!(realized.report.within_tolerance(), "{:?}", realized.report);
    assert!(
        realized.mesh.boundary_loops().unwrap().is_empty(),
        "watertight"
    );
}
