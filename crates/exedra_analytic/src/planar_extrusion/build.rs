// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::outline::loop_ref;
use super::{PlanarExtrusionError as Error, PlanarExtrusionPolicy, PlanarSegment};
use crate::End;
use crate::body::{
    Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeGeometry, EdgeId, Face, LoopId, Sense,
    ValidationPolicy, VertexId,
};
use crate::extrusion::{Boundary, BoundarySegment, EdgeRole, FaceRole, Level};
use crate::geometry::{Curve, CurveSpan, GeometryError, ParameterDomain, ParameterRange, Surface};
use alloc::{vec, vec::Vec};
use exedra_math::Placement3;

pub(super) fn body(
    loops: &[Vec<PlanarSegment>],
    length: f64,
    policy: &PlanarExtrusionPolicy,
) -> Result<Body<FaceRole, EdgeRole>, Error> {
    // Outline validation has already bounded the total source count to fit all
    // native IDs. Every source contributes two vertices, three edges, one wall
    // face and one wall loop; each outline contributes two cap loops/trims.
    let segments: usize = loops.iter().map(Vec::len).sum();
    let boundaries = loops.len();
    let mut parts = BodyParts {
        vertices: Vec::with_capacity(2 * segments),
        edges: Vec::with_capacity(3 * segments),
        loops: Vec::with_capacity(segments + 2 * boundaries),
        faces: Vec::with_capacity(segments + 2),
        trims: Vec::with_capacity(2 * boundaries),
    };
    let mut cap_loops = [
        Vec::with_capacity(boundaries),
        Vec::with_capacity(boundaries),
    ];
    let mut bounds = [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]];
    // Cap IDs stay 0 and 1, matching the other extrusion constructor.
    for end in [End::Start, End::End] {
        parts.faces.push(Face {
            surface: Surface::plane(Placement3::translate(
                0.0,
                0.0,
                if end == End::Start { 0.0 } else { length },
            ))
            .map_err(numeric)?,
            domain: ParameterDomain {
                u: ParameterRange::UNIT,
                v: ParameterRange::UNIT,
            },
            sense: if end == End::Start {
                Sense::Reverse
            } else {
                Sense::Forward
            },
            loops: Vec::new(),
            role: FaceRole::Cap(end),
        });
    }
    for (li, loop_) in loops.iter().enumerate() {
        let n = loop_.len();
        let vertex_base = parts.vertices.len();
        let edge_base = parts.edges.len();
        let boundary = Boundary::Profile(loop_ref(li));
        let source = |segment| BoundarySegment { boundary, segment };
        for z in [0.0, length] {
            for segment in loop_ {
                let p = segment
                    .span
                    .evaluate(0.0)
                    .ok_or_else(|| numeric(GeometryError::NonFinite))?
                    .point;
                parts.vertices.push([p[0], p[1], p[2] + z]);
            }
        }
        for (slot, z) in [0.0, length].into_iter().enumerate() {
            let mut ring = Vec::with_capacity(n);
            for (si, segment) in loop_.iter().enumerate() {
                let span = &segment.span;
                let start = vertex_base + slot * n + si;
                let end = vertex_base + slot * n + (si + 1) % n;
                let (start, end) = if span.reversed() {
                    (end, start)
                } else {
                    (start, end)
                };
                let edge = edge_id(parts.edges.len());
                parts.edges.push(Edge {
                    geometry: EdgeGeometry::Curve {
                        curve: span
                            .curve()
                            .transformed(Placement3::translate(0.0, 0.0, z))
                            .map_err(numeric)?,
                        range: span.range(),
                    },
                    start: vertex_id(start),
                    end: vertex_id(end),
                    role: EdgeRole::Boundary {
                        segment: source(si),
                        level: if slot == 0 { Level::Start } else { Level::End },
                    },
                });
                ring.push(Coedge {
                    edge,
                    reversed: span.reversed() ^ (slot == 0),
                });
                if slot == 0 {
                    let b = span
                        .curve()
                        .conservative_bounds(span.range())
                        .map_err(|_| numeric(GeometryError::NonFinite))?;
                    for k in 0..2 {
                        bounds[0][k] = bounds[0][k].min(b[0][k]);
                        bounds[1][k] = bounds[1][k].max(b[1][k]);
                    }
                }
            }
            if slot == 0 {
                ring.reverse();
            }
            let id = push_loop(&mut parts.loops, ring);
            cap_loops[slot].push(id);
        }
        for si in 0..n {
            let start = vertex_base + si;
            let end = start + n;
            let a = parts.vertices[start];
            parts.edges.push(Edge {
                geometry: EdgeGeometry::Curve {
                    curve: Curve::line(a, [0.0, 0.0, length]).map_err(numeric)?,
                    range: ParameterRange::UNIT,
                },
                start: vertex_id(start),
                end: vertex_id(end),
                // A seam belongs to the source segment ending at that vertex.
                role: EdgeRole::Seam {
                    segment: source((si + n - 1) % n),
                },
            });
        }
        for (si, segment) in loop_.iter().enumerate() {
            let below = segment.span.clone();
            let above = CurveSpan::new(
                below
                    .curve()
                    .transformed(Placement3::translate(0.0, 0.0, length))
                    .map_err(numeric)?,
                below.range(),
                below.reversed(),
            )
            .map_err(numeric)?;
            let surface = Surface::ruled(below, above).map_err(|source| Error::Wall {
                loop_ref: loop_ref(li),
                segment: si,
                source,
            })?;
            let ring = vec![
                Coedge {
                    edge: edge_id(edge_base + si),
                    reversed: segment.span.reversed(),
                },
                Coedge {
                    edge: edge_id(edge_base + 2 * n + (si + 1) % n),
                    reversed: false,
                },
                Coedge {
                    edge: edge_id(edge_base + n + si),
                    reversed: !segment.span.reversed(),
                },
                Coedge {
                    edge: edge_id(edge_base + 2 * n + si),
                    reversed: true,
                },
            ];
            let id = push_loop(&mut parts.loops, ring);
            parts.faces.push(Face {
                surface,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![id],
                role: FaceRole::Wall {
                    segment: source(si),
                    tag: segment.tag,
                },
            });
        }
    }
    for (slot, loops) in cap_loops.into_iter().enumerate() {
        parts.faces[slot].loops = loops;
        parts.faces[slot].domain = ParameterDomain {
            u: ParameterRange::new(bounds[0][0], bounds[1][0]).map_err(numeric)?,
            v: ParameterRange::new(bounds[0][1], bounds[1][1]).map_err(numeric)?,
        };
        for i in 0..parts.faces[slot].loops.len() {
            let id = parts.faces[slot].loops[i];
            crate::operation_trims::append_planar_loop(
                &mut parts,
                crate::body::FaceId(crate::usize_to_u32(slot)),
                id,
            )
            .map_err(numeric)?;
        }
    }
    Body::try_from_parts(
        parts,
        &ValidationPolicy {
            max_gap: policy.max_gap,
        },
    )
    .map_err(Error::Body)
}
fn numeric(source: GeometryError) -> Error {
    Error::Geometry {
        loop_ref: exedra_curves::profile::LoopRef::Outer,
        segment: 0,
        source,
    }
}
fn vertex_id(index: usize) -> VertexId {
    VertexId(crate::usize_to_u32(index))
}
fn edge_id(index: usize) -> EdgeId {
    EdgeId(crate::usize_to_u32(index))
}
fn push_loop(loops: &mut Vec<BoundaryLoop>, coedges: Vec<Coedge>) -> LoopId {
    let id = LoopId(crate::usize_to_u32(loops.len()));
    loops.push(BoundaryLoop { coedges });
    id
}
