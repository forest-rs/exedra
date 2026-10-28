// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction of explicit parameter correspondence for native edit results.
use crate::body::{BodyParts, FaceId, LoopId, LoopTrims, TrimCurve};
use crate::geometry::{Curve, GeometryError, Surface};
use alloc::vec::Vec;
use exedra_math::dot;

pub(crate) fn append_planar_loop<F, E>(
    parts: &mut BodyParts<F, E>,
    face: FaceId,
    loop_id: LoopId,
) -> Result<(), GeometryError> {
    if parts.trims.iter().any(|t| t.loop_id == loop_id) {
        return Ok(());
    }
    let surface = &parts.faces[face.index()].surface;
    let Surface::Plane(plane) = surface else {
        return Err(GeometryError::NonFinite);
    };
    let pf = plane.frame();
    let mut curves = Vec::with_capacity(parts.loops[loop_id.index()].coedges.len());
    for co in &parts.loops[loop_id.index()].coedges {
        let edge = &parts.edges[co.edge.index()];
        let (curve, range) = edge.regular().ok_or(GeometryError::NonFinite)?;
        let trim = match curve {
            Curve::Line(_) => TrimCurve::Line {
                start: surface
                    .parameters(parts.vertices[edge.start.index()])
                    .ok_or(GeometryError::NonFinite)?,
                end: surface
                    .parameters(parts.vertices[edge.end.index()])
                    .ok_or(GeometryError::NonFinite)?,
            },
            Curve::Circle(c) => {
                let f = c.frame();
                let radius = c.radius();
                TrimCurve::Harmonic {
                    origin: surface
                        .parameters(f.rows.map(|r| r[3]))
                        .ok_or(GeometryError::NonFinite)?,
                    linear: [0.0; 2],
                    cosine: core::array::from_fn(|i| {
                        radius * dot(f.rows.map(|r| r[0]), pf.rows.map(|r| r[i]))
                    }),
                    sine: core::array::from_fn(|i| {
                        radius * dot(f.rows.map(|r| r[1]), pf.rows.map(|r| r[i]))
                    }),
                    range,
                }
            }
            Curve::Ellipse(e) => {
                let f = e.frame();
                TrimCurve::Harmonic {
                    origin: surface
                        .parameters(f.rows.map(|r| r[3]))
                        .ok_or(GeometryError::NonFinite)?,
                    linear: [0.0; 2],
                    cosine: core::array::from_fn(|i| {
                        e.semi_major() * dot(f.rows.map(|r| r[0]), pf.rows.map(|r| r[i]))
                    }),
                    sine: core::array::from_fn(|i| {
                        e.semi_minor() * dot(f.rows.map(|r| r[1]), pf.rows.map(|r| r[i]))
                    }),
                    range,
                }
            }
        };
        curves.push(trim);
    }
    parts.trims.push(LoopTrims { loop_id, curves });
    Ok(())
}
pub(crate) fn append_all_planar<F, E>(parts: &mut BodyParts<F, E>) -> Result<(), GeometryError> {
    for index in 0..parts.faces.len() {
        let face = FaceId(crate::usize_to_u32(index));
        for i in 0..parts.faces[index].loops.len() {
            let loop_id = parts.faces[index].loops[i];
            append_planar_loop(parts, face, loop_id)?;
        }
    }
    Ok(())
}
