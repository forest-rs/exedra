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

pub(crate) fn prism() -> Body {
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
    Body::try_from_parts(parts, &POLICY).unwrap()
}

#[test]
fn a_prism_over_a_spline_is_a_closed_solid() {
    let body = prism();
    let realized = body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())
        .unwrap();
    assert!(realized.report.within_tolerance(), "{:?}", realized.report);
    assert!(
        realized.mesh.boundary_loops().unwrap().is_empty(),
        "watertight"
    );
}

#[test]
fn ruled_patches_become_b_spline_surfaces_where_one_is_exact() {
    let range = ParameterRange::new(0.0, 2.0).unwrap();
    let spline = |z| CurveSpan::new(Curve::nurbs(bulge(z)).unwrap(), range, false).unwrap();
    let line = |y: f64, z: f64| {
        CurveSpan::new(
            Curve::line([0.0, y, z], [4.0, 0.0, 0.0]).unwrap(),
            ParameterRange::UNIT,
            false,
        )
        .unwrap()
    };
    let arc = |radius: f64, z: f64, reversed: bool| {
        CurveSpan::new(
            Curve::circle(Placement3::translate(0.0, 0.0, z), radius).unwrap(),
            ParameterRange::new(0.3, 2.1).unwrap(),
            reversed,
        )
        .unwrap()
    };
    // Each with the patch's U at a row parameter: the same for splines and
    // lines; for arcs, whose rational parameter is not proportional to
    // angle, the angle the rows reach there.
    let same = |_: [f64; 3], s: f64| s;
    let angle = |p: [f64; 3], _: f64| ((libm::atan2(p[1], p[0]) - 0.3) / 1.8).clamp(0.0, 1.0);
    type PatchU<'a> = &'a dyn Fn([f64; 3], f64) -> f64;
    let exact: [(Surface, PatchU<'_>); 3] = [
        (Surface::ruled(spline(0.0), spline(1.0)).unwrap(), &same),
        (
            Surface::ruled(line(0.0, 0.0), line(1.0, 2.0)).unwrap(),
            &same,
        ),
        (
            Surface::ruled(arc(1.0, 0.0, false), arc(0.5, 2.0, false)).unwrap(),
            &angle,
        ),
    ];
    for (surface, patch_u) in exact {
        let Surface::Ruled(ruled) = &surface else {
            unreachable!()
        };
        let [a, b] = ruled.nurbs_rows().unwrap();
        assert_eq!((a.order(), a.knots()), (b.order(), b.knots()));
        let (a, b) = (a.view(), b.view());
        for i in 0..=20 {
            for j in 0..=4 {
                let (s, v) = (f64::from(i) / 20.0, f64::from(j) / 4.0);
                let p = a.evaluate(s);
                let q = b.evaluate(s);
                let expected = surface.evaluate([patch_u(p, s), v]).unwrap().point;
                // The rows share their weights, so the rational surface
                // through them is the interpolation of its rows.
                let got: [f64; 3] = core::array::from_fn(|k| (1.0 - v) * p[k] + v * q[k]);
                assert!(norm(sub(got, expected)) < 1e-12, "{s} {v}");
            }
        }
    }
    // Between a line and an arc, or arcs of different sweep, no surface is
    // exact.
    // The chord under the arc, a unit below it.
    let (start, end) = (
        [libm::cos(0.3), libm::sin(0.3)],
        [libm::cos(2.1), libm::sin(2.1)],
    );
    let chord = CurveSpan::new(
        Curve::line(
            [start[0], start[1], -1.0],
            [end[0] - start[0], end[1] - start[1], 0.0],
        )
        .unwrap(),
        ParameterRange::UNIT,
        false,
    )
    .unwrap();
    let line_arc = Surface::ruled(chord, arc(1.0, 0.0, false)).unwrap();
    let Surface::Ruled(ruled) = line_arc else {
        unreachable!()
    };
    assert!(ruled.nurbs_rows().is_none());
}

#[test]
fn b_spline_rows_trace_conic_patches_but_not_their_parameters() {
    let quarter = ParameterRange::new(0.0, core::f64::consts::FRAC_PI_2).unwrap();
    let arc = |z| {
        CurveSpan::new(
            Curve::circle(Placement3::translate(0.0, 0.0, z), 1.0).unwrap(),
            quarter,
            false,
        )
        .unwrap()
    };
    let surface = Surface::ruled(arc(0.0), arc(1.0)).unwrap();
    let Surface::Ruled(ruled) = &surface else {
        unreachable!()
    };
    let [a, b] = ruled.nurbs_rows().unwrap();
    let (s, v) = (0.25, 0.5);
    let (p, q) = (a.view().evaluate(s), b.view().evaluate(s));
    let row: [f64; 3] = core::array::from_fn(|k| (1.0 - v) * p[k] + v * q[k]);
    // The ruling at the rows' parameter is the patch's at the angle it
    // reaches, a point of the patch ...
    let angle = libm::atan2(p[1], p[0]) / core::f64::consts::FRAC_PI_2;
    let traced = surface.evaluate([angle, v]).unwrap().point;
    assert!(norm(sub(row, traced)) < 1e-12);
    // ... but not the patch's point at the same parameter.
    let same = surface.evaluate([s, v]).unwrap().point;
    assert!(norm(sub(row, same)) > 1e-2, "{}", norm(sub(row, same)));
}
