// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cyclic planar frames with the ordinary rail's authored roll and miter cuts.

use super::{
    SweepFrame, TessellateError, initial_section_x, miter_frame, sweep_direction, unit_vector,
};
use alloc::vec::Vec;
use exedra_math::{add, cross, dot, scale, sub};

pub(super) fn frames(
    points: &[[f64; 3]],
    section_x: [f64; 3],
    normal: [f64; 3],
    miter_limit: f64,
) -> Result<Vec<SweepFrame>, TessellateError> {
    if points.len() < 3 {
        return Err(TessellateError::InvalidSweepPath);
    }
    let normal = unit_vector(normal).ok_or(TessellateError::InvalidSweepPlane)?;
    let coordinate_scale = points
        .iter()
        .flatten()
        .map(|v| v.abs())
        .fold(0.0_f64, f64::max);
    let tolerance = coordinate_scale * (64.0 * f64::EPSILON);
    for (point, position) in points.iter().enumerate() {
        let distance = dot(sub(*position, points[0]), normal).abs();
        if !distance.is_finite() {
            return Err(TessellateError::NonFiniteGeometry);
        }
        if distance > tolerance {
            return Err(TessellateError::NonPlanarSweep {
                point,
                distance,
                tolerance,
            });
        }
    }
    let directions = (0..points.len())
        .map(|i| sweep_direction(points[i], points[(i + 1) % points.len()]))
        .collect::<Result<Vec<_>, _>>()?;
    let first = directions[0];
    let first_x = initial_section_x(first, section_x)?;
    let radial = |t| unit_vector(cross(normal, t)).ok_or(TessellateError::InvalidSweepPlane);
    let first_radial = radial(first)?;
    let radial_component = dot(first_x, first_radial);
    let normal_component = dot(first_x, cross(first, first_radial));
    let mut frames = Vec::with_capacity(points.len() + 1);
    for (i, &point) in points.iter().enumerate() {
        let incoming = directions[(i + points.len() - 1) % points.len()];
        let outgoing = directions[i];
        let r = radial(incoming)?;
        // Reconstruct the same planar roll independently on each run. There
        // is no sequential integration error to hide with a final twist.
        let x = add(
            scale(r, radial_component),
            scale(cross(incoming, r), normal_component),
        );
        frames.push(miter_frame(point, i, incoming, outgoing, x, miter_limit)?);
    }
    // The duplicate is a construction/check interval only. Mesh emission
    // reuses ring zero's vertices at the end of the closing band.
    frames.push(frames[0]);
    Ok(frames)
}
