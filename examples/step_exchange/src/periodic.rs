// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_analytic::{
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, ValidationPolicy,
        VertexId,
    },
    geometry::{Curve, ParameterDomain, ParameterRange, Surface},
};
use exedra_math::Placement3;

/// A single periodic wall with two closed circle edges and a shared seam.
pub(super) fn cylinder() -> Body {
    let bottom = Placement3::IDENTITY;
    let top = Placement3::translate(0.0, 0.0, 5.0);
    let angular = ParameterRange::new(0.0, std::f64::consts::TAU).unwrap();
    let edges = vec![
        Edge {
            geometry: exedra_analytic::body::EdgeGeometry::Curve {
                curve: Curve::circle(bottom, 2.0).unwrap(),
                range: angular,
            },
            start: VertexId(0),
            end: VertexId(0),
            role: (),
        },
        Edge {
            geometry: exedra_analytic::body::EdgeGeometry::Curve {
                curve: Curve::circle(top, 2.0).unwrap(),
                range: angular,
            },
            start: VertexId(1),
            end: VertexId(1),
            role: (),
        },
        Edge {
            geometry: exedra_analytic::body::EdgeGeometry::Curve {
                curve: Curve::line([2.0, 0.0, 0.0], [0.0, 0.0, 5.0]).unwrap(),
                range: ParameterRange::UNIT,
            },
            start: VertexId(0),
            end: VertexId(1),
            role: (),
        },
    ];
    let loops = [
        vec![(0, false), (2, false), (1, true), (2, true)],
        vec![(0, true)],
        vec![(1, false)],
    ]
    .into_iter()
    .map(|ring| BoundaryLoop {
        coedges: ring
            .into_iter()
            .map(|(id, reversed)| Coedge {
                edge: EdgeId(id),
                reversed,
            })
            .collect(),
    })
    .collect();
    let cap_domain = ParameterDomain {
        u: ParameterRange::new(-2.0, 2.0).unwrap(),
        v: ParameterRange::new(-2.0, 2.0).unwrap(),
    };
    let faces = vec![
        Face {
            surface: Surface::cylinder(bottom, 2.0).unwrap(),
            domain: ParameterDomain {
                u: angular,
                v: ParameterRange::new(0.0, 5.0).unwrap(),
            },
            sense: Sense::Forward,
            loops: vec![LoopId(0)],
            role: (),
        },
        Face {
            surface: Surface::plane(bottom).unwrap(),
            domain: cap_domain,
            sense: Sense::Reverse,
            loops: vec![LoopId(1)],
            role: (),
        },
        Face {
            surface: Surface::plane(top).unwrap(),
            domain: cap_domain,
            sense: Sense::Forward,
            loops: vec![LoopId(2)],
            role: (),
        },
    ];
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices: vec![[2.0, 0.0, 0.0], [2.0, 0.0, 5.0]],
            edges,
            loops,
            faces,
        },
        &ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap()
}

/// One toroidal face with both closed seam curves shared twice in its loop.
pub(super) fn torus() -> Body {
    let angular = ParameterRange::new(0.0, std::f64::consts::TAU).unwrap();
    let meridian = Placement3::from_axes(
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [10.0, 0.0, 0.0],
    );
    let edges = [
        Curve::circle(Placement3::IDENTITY, 12.0).unwrap(),
        Curve::circle(meridian, 2.0).unwrap(),
    ]
    .map(|curve| Edge::new(curve, angular, VertexId(0), VertexId(0), ()))
    .to_vec();
    Body::try_from_parts(
        BodyParts {
            trims: Vec::new(),
            vertices: vec![[12.0, 0.0, 0.0]],
            edges,
            loops: vec![BoundaryLoop {
                coedges: [(0, false), (1, false), (0, true), (1, true)]
                    .map(|(edge, reversed)| Coedge {
                        edge: EdgeId(edge),
                        reversed,
                    })
                    .to_vec(),
            }],
            faces: vec![Face {
                surface: Surface::torus(Placement3::IDENTITY, 10.0, 2.0).unwrap(),
                domain: ParameterDomain {
                    u: angular,
                    v: angular,
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
        },
        &ValidationPolicy { max_gap: 1e-12 },
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra_step::{StepError, StepOptions, export_body};

    #[test]
    fn full_periodic_seam_exports_but_multiple_turns_are_refused() {
        let body = cylinder();
        let exported = export_body(&body, Placement3::IDENTITY, &StepOptions::default()).unwrap();
        assert_eq!(
            (
                exported.report.counts.faces,
                exported.report.counts.edges,
                exported.report.counts.vertices,
                exported.report.counts.coedges
            ),
            (3, 3, 2, 6)
        );
        let mut parts = body.clone().into_parts();
        *parts.edges[0].regular_mut().unwrap().1 =
            ParameterRange::new(0.0, 2.0 * std::f64::consts::TAU).unwrap();
        assert!(matches!(
            Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }),
            Err(exedra_analytic::body::BodyError::RepeatedCurve { edge: EdgeId(0) })
        ));
        let mut parts = body.into_parts();
        parts.faces[0].domain.u = ParameterRange::new(0.0, 2.0 * std::f64::consts::TAU).unwrap();
        let multiple = Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-12 }).unwrap();
        assert!(matches!(
            export_body(&multiple, Placement3::IDENTITY, &StepOptions::default()),
            Err(StepError::CylinderDomain { .. })
        ));
    }
}
