// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared lowering of a checked profile arc into supporting 3D geometry.

use crate::geometry::{Curve, GeometryError};
use exedra_math::Placement3;
use exedra_profile::Arc;

pub(crate) fn circle(a: Arc, frame: Placement3) -> Result<Curve, GeometryError> {
    let (s, c) = (libm::sin(a.start_angle), libm::cos(a.start_angle));
    let sign = a.sweep.signum();
    let plane = Placement3::from_axes(
        [c, s, 0.0],
        [-s * sign, c * sign, 0.0],
        [0.0, 0.0, sign],
        [a.center[0], a.center[1], 0.0],
    );
    Curve::circle(frame * plane, a.radius)
}

/// Integral of positive radial coordinate along a generating profile arc.
pub(crate) fn radial_integral(a: Arc) -> f64 {
    let h = 0.5 * a.sweep;
    a.length() * (a.center[0] + a.radius * libm::cos(a.start_angle + h) * (libm::sin(h) / h))
}
