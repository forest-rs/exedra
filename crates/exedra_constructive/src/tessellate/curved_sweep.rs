// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Analytic tangent frames with authored corner cuts and cyclic planar roll.

use super::{
    SweepFrame, TessellateError, initial_section_x, miter_frame, sweep_direction, unit_vector,
};
use crate::path::{DiscretizedPath, PathClosure, PathJoin, PathStation, same_tangent};
use alloc::vec::Vec;
use exedra_math::{add, cross, dot, scale, sub};

fn station_frame(
    station: PathStation,
    index: usize,
    incoming_x: [f64; 3],
    joins: PathJoin,
) -> Result<SweepFrame, TessellateError> {
    let incoming = station.incoming_tangent;
    let outgoing = station.tangent;
    if same_tangent(incoming, outgoing) {
        let x = initial_section_x(outgoing, incoming_x)?;
        return Ok((station.point, x, cross(outgoing, x), outgoing));
    }
    let PathJoin::Miter { limit } = joins else {
        unreachable!("sampling checks continuity")
    };
    miter_frame(station.point, index, incoming, outgoing, incoming_x, limit)
}

pub(super) fn frames(
    path: &DiscretizedPath,
    section_x: [f64; 3],
) -> Result<Vec<SweepFrame>, TessellateError> {
    let first = path.stations[0];
    let first_x = initial_section_x(first.tangent, section_x)?;
    let joins = path.sampling.joins;
    let mut frames = Vec::with_capacity(path.stations.len());
    if let PathClosure::ClosedPlanar { normal } = path.sampling.closure {
        let normal = unit_vector(normal).ok_or(TessellateError::InvalidSweepPlane)?;
        let radial = |t| unit_vector(cross(normal, t)).ok_or(TessellateError::InvalidSweepPlane);
        let first_radial = radial(first.tangent)?;
        let r = dot(first_x, first_radial);
        let n = dot(first_x, cross(first.tangent, first_radial));
        for (index, &station) in path.stations[..path.stations.len() - 1].iter().enumerate() {
            let t = station.incoming_tangent;
            let radial = radial(t)?;
            let x = add(scale(radial, r), scale(cross(t, radial), n));
            frames.push(station_frame(station, index, x, joins)?);
        }
        frames.push(frames[0]);
        return Ok(frames);
    }
    let mut x = first_x;
    frames.push(station_frame(first, 0, x, joins)?);
    for (index, stations) in path.stations.windows(2).enumerate() {
        let previous = stations[0];
        let next = stations[1];
        // Double-reflection transport within an analytic span, to the incoming
        // tangent at its endpoint. An authored corner then uses shortest rotation.
        let chord = sweep_direction(previous.point, next.point)?;
        let reflected_x = sub(x, scale(chord, 2.0 * dot(chord, x)));
        let reflected_t = sub(
            previous.tangent,
            scale(chord, 2.0 * dot(chord, previous.tangent)),
        );
        x = if let Some(normal) = unit_vector(sub(next.incoming_tangent, reflected_t)) {
            sub(reflected_x, scale(normal, 2.0 * dot(normal, reflected_x)))
        } else {
            reflected_x
        };
        x = initial_section_x(next.incoming_tangent, x)?;
        let frame = station_frame(next, index + 1, x, joins)?;
        if !same_tangent(next.incoming_tangent, next.tangent) {
            x = sub(x, scale(frame.3, 2.0 * dot(x, frame.3)));
        }
        x = initial_section_x(next.tangent, x)?;
        frames.push(frame);
    }
    Ok(frames)
}
