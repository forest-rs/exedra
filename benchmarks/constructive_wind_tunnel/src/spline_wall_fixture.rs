// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A cubic spline closed by its chord, with exact ruled walls between two sections.
use exedra_analytic::{
    body::{Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, VertexId},
    geometry::{Curve, CurveSpan, NurbsCurve, ParameterDomain, ParameterRange, Surface},
};
use exedra_math::Placement3;
#[derive(Clone)]
struct Fixture {
    weights: Vec<f64>,
    points: Vec<[f64; 3]>,
}
impl Fixture {
    fn owned(&self) -> NurbsCurve {
        NurbsCurve::new(
            4,
            vec![0., 0., 0., 0., 1., 1., 1., 1.],
            self.points.clone(),
            Some(self.weights.clone()),
        )
        .unwrap()
    }
}
/// Two similar sections, preserving shared native edges and exact spline geometry.
pub(crate) fn body(weighted: bool, top_scale: f64) -> Body {
    let f = Fixture {
        weights: if weighted {
            vec![1.0, 0.7, 1.4, 1.0]
        } else {
            vec![1.0; 4]
        },
        points: vec![
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [4.0, 0.0, 0.0],
        ],
    };
    let curve = f.owned();
    let range = ParameterRange::UNIT;
    use exedra_analytic::body::{EdgeGeometry, ValidationPolicy};
    let below = Curve::nurbs(curve.clone()).unwrap();
    let mut above_fixture = f.clone();
    above_fixture.points = f
        .points
        .iter()
        .map(|p| [2.0 + top_scale * (p[0] - 2.0), top_scale * p[1], 1.0])
        .collect();
    let above = Curve::nurbs(above_fixture.owned()).unwrap();
    let wall = Surface::ruled(
        CurveSpan::new(below.clone(), range, false).unwrap(),
        CurveSpan::new(above.clone(), range, false).unwrap(),
    )
    .unwrap();
    let edge = |curve: Curve, range: ParameterRange, start: u32, end: u32| Edge {
        geometry: EdgeGeometry::Curve { curve, range },
        start: VertexId(start),
        end: VertexId(end),
        role: (),
    };
    let line = |a: [f64; 3], b: [f64; 3]| {
        let curve = Curve::line(a, core::array::from_fn(|k| b[k] - a[k])).unwrap();
        (curve, ParameterRange::UNIT)
    };
    let coedge = |edge: u32, reversed: bool| Coedge {
        edge: EdgeId(edge),
        reversed,
    };
    let plane = |frame: Placement3, v: [f64; 2], sense: Sense, loop_id: u32| Face {
        surface: Surface::plane(frame).unwrap(),
        domain: ParameterDomain {
            u: ParameterRange::new(-1.0, 5.0).unwrap(),
            v: ParameterRange::new(v[0], v[1]).unwrap(),
        },
        sense,
        loops: vec![LoopId(loop_id)],
        role: (),
    };
    let [a, b, c, d] = [
        [0.0; 3],
        [4.0, 0.0, 0.0],
        [2.0 - 2.0 * top_scale, 0.0, 1.0],
        [2.0 + 2.0 * top_scale, 0.0, 1.0],
    ];
    let edges = vec![
        edge(below, range, 0, 1),
        edge(above, range, 2, 3),
        {
            let (curve, range) = line(b, a);
            edge(curve, range, 1, 0)
        },
        {
            let (curve, range) = line(d, c);
            edge(curve, range, 3, 2)
        },
        {
            let (curve, range) = line(a, c);
            edge(curve, range, 0, 2)
        },
        {
            let (curve, range) = line(b, d);
            edge(curve, range, 1, 3)
        },
    ];
    let parts: BodyParts<(), ()> = BodyParts {
        trims: Vec::new(),
        vertices: vec![a, b, c, d],
        edges,
        loops: vec![
            BoundaryLoop {
                coedges: vec![coedge(0, false), coedge(2, false)],
            },
            BoundaryLoop {
                coedges: vec![coedge(3, true), coedge(1, true)],
            },
            BoundaryLoop {
                coedges: vec![
                    coedge(4, false),
                    coedge(1, false),
                    coedge(5, true),
                    coedge(0, true),
                ],
            },
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
            plane(Placement3::IDENTITY, [-1.0, 3.0], Sense::Reverse, 0),
            plane(
                Placement3::translate(0.0, 0.0, 1.0),
                [-1.0, 3.0],
                Sense::Forward,
                1,
            ),
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
            plane(
                Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]),
                [-1.0, 2.0],
                Sense::Forward,
                3,
            ),
        ],
    };
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}
