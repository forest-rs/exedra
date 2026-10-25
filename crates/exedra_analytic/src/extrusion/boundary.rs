// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lower construction geometry into the shared boundary model once.

use crate::body::{BoundaryLoop, Coedge, LoopId, Sense};
use crate::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
use alloc::vec::Vec;
use exedra_math::Placement3;
use exedra_profile::Arc;

pub(super) fn line(from: [f64; 3], to: [f64; 3]) -> (Curve, ParameterRange) {
    (
        Curve::line(from, exedra_math::sub(to, from)).expect("validated line"),
        ParameterRange::UNIT,
    )
}

fn circle_frame(arc: Arc, z: f64) -> Placement3 {
    let (sin, cos) = (libm::sin(arc.start_angle), libm::cos(arc.start_angle));
    let sense = arc.sweep.signum();
    Placement3::from_axes(
        [cos, sin, 0.0],
        [-sense * sin, sense * cos, 0.0],
        [0.0, 0.0, sense],
        [arc.center[0], arc.center[1], z],
    )
}

pub(super) fn circle(arc: Arc, z: f64) -> (Curve, ParameterRange) {
    (
        Curve::circle(circle_frame(arc, z), arc.radius).expect("validated circle"),
        ParameterRange::new(0.0, arc.sweep.abs()).expect("nonzero arc"),
    )
}

pub(super) fn wall(
    from: [f64; 2],
    to: [f64; 2],
    arc: Option<Arc>,
    heights: [f64; 2],
) -> (Surface, ParameterDomain, Sense) {
    if let Some(arc) = arc {
        let sign = arc.sweep.signum();
        let mut v = heights.map(|h| h * sign);
        v.sort_by(f64::total_cmp);
        (
            Surface::cylinder(circle_frame(arc, 0.0), arc.radius).expect("validated cylinder"),
            ParameterDomain {
                u: ParameterRange::new(0.0, arc.sweep.abs()).expect("arc span"),
                v: ParameterRange::new(v[0], v[1]).expect("wall span"),
            },
            if sign > 0.0 {
                Sense::Forward
            } else {
                Sense::Reverse
            },
        )
    } else {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let span = libm::hypot(dx, dy);
        let frame = Placement3::from_axes(
            [dx / span, dy / span, 0.0],
            [0.0, 0.0, 1.0],
            [dy / span, -dx / span, 0.0],
            [from[0], from[1], 0.0],
        );
        (
            Surface::plane(frame).expect("validated wall plane"),
            ParameterDomain {
                u: ParameterRange::new(0.0, span).expect("segment span"),
                v: ParameterRange::new(heights[0], heights[1]).expect("wall span"),
            },
            Sense::Forward,
        )
    }
}

pub(super) fn push_loop(loops: &mut Vec<BoundaryLoop>, coedges: Vec<Coedge>) -> LoopId {
    let id = LoopId(crate::usize_to_u32(loops.len()));
    loops.push(BoundaryLoop { coedges });
    id
}
