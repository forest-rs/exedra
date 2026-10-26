// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared section topology and analytic line/arc transport.

use super::{EdgeRole, FaceRole, Sweep, SweepError};
use crate::End;
use crate::body::{
    Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, ValidationPolicy,
    VertexId,
};
use crate::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
use alloc::{vec, vec::Vec};
use exedra_math::path::{CurvePathSource, PathClosure, PathJoin, PathSegment3};
use exedra_math::{Placement3, add, cross, dot, scale, sub};
use exedra_profile::{Curve as ProfileCurve, LoopRef, Profile, SegmentRef};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Section {
    pub point: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub tangent: [f64; 3],
}
impl Section {
    fn point(&self, p: [f64; 2], datum: [f64; 2]) -> [f64; 3] {
        add(
            self.point,
            add(
                scale(self.x, p[0] - datum[0]),
                scale(self.y, p[1] - datum[1]),
            ),
        )
    }
    fn frame(&self, datum: [f64; 2]) -> Placement3 {
        Placement3::from_axes(
            self.x,
            self.y,
            self.tangent,
            sub(
                self.point,
                add(scale(self.x, datum[0]), scale(self.y, datum[1])),
            ),
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Motion {
    Line,
    Arc {
        center: [f64; 3],
        axis: [f64; 3],
        radial: [f64; 3],
        radius: f64,
        angle: f64,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Span {
    pub start: Section,
    pub end: Section,
    pub motion: Motion,
    pub length: f64,
}
struct Vertex {
    point: [f64; 2],
    source: SegmentRef,
    tag: Option<exedra_profile::SegTag>,
    next: usize,
}
fn norm(p: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(p[0], p[1]), p[2])
}
fn unit(p: [f64; 3]) -> Option<[f64; 3]> {
    let m = p.iter().map(|v| v.abs()).fold(0.0, f64::max);
    if m == 0.0 || !m.is_finite() || p.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let p = p.map(|v| v / m);
    Some(scale(p, 1.0 / norm(p)))
}
fn rotate(p: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = (libm::sin(angle), libm::cos(angle));
    add(
        add(scale(p, c), scale(cross(axis, p), s)),
        scale(axis, dot(axis, p) * (1.0 - c)),
    )
}
fn tangent_matches(a: [f64; 3], b: [f64; 3]) -> bool {
    dot(a, b) > 0.0 && norm(cross(a, b)) <= 1e-10
}
fn arc(
    from: [f64; 3],
    origin: [f64; 3],
    axis: [f64; 3],
    sweep: f64,
    segment: usize,
) -> Result<(Motion, Curve, [f64; 3]), SweepError> {
    let invalid = || SweepError::PathSegment { segment };
    if origin.iter().any(|v| !v.is_finite())
        || !sweep.is_finite()
        || sweep == 0.0
        || sweep.abs() >= core::f64::consts::TAU
    {
        return Err(invalid());
    }
    let axis = unit(axis).ok_or_else(invalid)?;
    let delta = sub(from, origin);
    let radial = sub(delta, scale(axis, dot(delta, axis)));
    let radius = norm(radial);
    if !radius.is_finite() || radius == 0.0 {
        return Err(invalid());
    }
    let radial = scale(radial, 1.0 / radius);
    let center = sub(from, scale(radial, radius));
    let axis = scale(axis, sweep.signum());
    let tangent = cross(axis, radial);
    let circle = Curve::circle(Placement3::from_axes(radial, tangent, axis, center), radius)
        .map_err(SweepError::Geometry)?;
    Ok((
        Motion::Arc {
            center,
            axis,
            radial,
            radius,
            angle: sweep.abs(),
        },
        circle,
        tangent,
    ))
}

pub(super) fn construct(
    profile: Profile,
    path: &impl CurvePathSource,
    frame: Placement3,
) -> Result<Sweep, SweepError> {
    crate::geometry::rigid(frame).map_err(SweepError::Geometry)?;
    let path = path.curve_path().ok_or(SweepError::UnsupportedPath)?;
    if path.segments.is_empty()
        || path.closure != PathClosure::Open
        || path.start.iter().any(|v| !v.is_finite())
    {
        return Err(SweepError::UnsupportedPath);
    }
    if let PathJoin::Miter { limit } = path.joins
        && (!limit.is_finite() || limit < 1.0)
    {
        return Err(SweepError::Join);
    }
    if path.section_origin.iter().any(|v| !v.is_finite()) {
        return Err(SweepError::SectionOrigin);
    }
    let mut vertices = Vec::new();
    let mut rings = Vec::new();
    for (loop_ref, ring) in core::iter::once((LoopRef::Outer, profile.outer())).chain(
        profile
            .holes()
            .iter()
            .enumerate()
            .map(|(i, r)| (LoopRef::Hole(i), r)),
    ) {
        let begin = vertices.len();
        let n = ring.segments().len();
        for (i, (from, segment)) in ring.iter_with_starts().enumerate() {
            let source = SegmentRef {
                loop_ref,
                segment: i,
            };
            if !matches!(segment.curve, ProfileCurve::Line) {
                return Err(SweepError::CurvedProfile { segment: source });
            }
            vertices.push(Vertex {
                point: from,
                source,
                tag: segment.tag,
                next: begin + (i + 1) % n,
            });
        }
        rings.push(begin..vertices.len());
    }
    let n = vertices.len();
    let count = path.segments.len();
    let station_count = count.checked_add(1).ok_or(SweepError::NumericLimit)?;
    let section_count = n
        .checked_mul(station_count)
        .ok_or(SweepError::NumericLimit)?;
    let rail_count = n.checked_mul(count).ok_or(SweepError::NumericLimit)?;
    let edge_count = section_count
        .checked_add(rail_count)
        .ok_or(SweepError::NumericLimit)?;
    let face_count = rail_count.checked_add(2).ok_or(SweepError::NumericLimit)?;
    let loop_count = rail_count
        .checked_add(rings.len().checked_mul(2).ok_or(SweepError::NumericLimit)?)
        .ok_or(SweepError::NumericLimit)?;
    if [section_count, edge_count, face_count, loop_count]
        .into_iter()
        .any(|size| u32::try_from(size).is_err())
    {
        return Err(SweepError::NumericLimit);
    }
    let mut spans = Vec::with_capacity(count);
    let mut current = path.start;
    let mut section: Option<Section> = None;
    for (segment, curve) in path.segments.iter().enumerate() {
        let (motion, first, end, last, length) = match curve {
            PathSegment3::Line { to } => {
                if to.iter().any(|v| !v.is_finite()) {
                    return Err(SweepError::PathSegment { segment });
                }
                let delta = sub(*to, current);
                let direction = unit(delta).ok_or(SweepError::PathSegment { segment })?;
                (Motion::Line, direction, *to, direction, norm(delta))
            }
            PathSegment3::Arc {
                axis_origin,
                axis,
                sweep,
            } => {
                let (motion, circle, first) = arc(current, *axis_origin, *axis, *sweep, segment)?;
                let Motion::Arc { radius, angle, .. } = motion else {
                    unreachable!()
                };
                let end = circle.evaluate(angle).ok_or(SweepError::NumericLimit)?;
                let last = unit(end.first).ok_or(SweepError::NumericLimit)?;
                (motion, first, end.point, last, radius * angle)
            }
            _ => return Err(SweepError::PathSegment { segment }),
        };
        if !length.is_finite() {
            return Err(SweepError::NumericLimit);
        }
        let start = if let Some(previous) = section.take() {
            if !tangent_matches(previous.tangent, first) {
                return Err(SweepError::Corner { segment });
            }
            previous
        } else {
            let authored = unit(path.section_x).ok_or(SweepError::SectionDirection)?;
            let x = unit(sub(authored, scale(first, dot(authored, first))))
                .ok_or(SweepError::SectionDirection)?;
            if norm(cross(authored, first)) < 1e-12 {
                return Err(SweepError::SectionDirection);
            }
            Section {
                point: current,
                x,
                y: cross(first, x),
                tangent: first,
            }
        };
        let end_section = match motion {
            Motion::Line => Section {
                point: end,
                ..start.clone()
            },
            Motion::Arc { axis, angle, .. } => Section {
                point: end,
                x: rotate(start.x, axis, angle),
                y: rotate(start.y, axis, angle),
                tangent: last,
            },
        };
        crate::geometry::rigid(end_section.frame(path.section_origin))
            .map_err(SweepError::Geometry)?;
        section = Some(end_section.clone());
        current = end;
        spans.push(Span {
            start,
            end: end_section,
            motion,
            length,
        });
    }
    if current == path.start {
        return Err(SweepError::UnsupportedPath);
    }
    let mut parts = BodyParts {
        vertices: Vec::with_capacity(section_count),
        edges: Vec::with_capacity(edge_count),
        loops: Vec::with_capacity(loop_count),
        faces: Vec::with_capacity(face_count),
    };
    for section in core::iter::once(&spans[0].start).chain(spans.iter().map(|s| &s.end)) {
        for vertex in &vertices {
            parts
                .vertices
                .push(section.point(vertex.point, path.section_origin));
        }
    }
    let vid = |station: usize, j: usize| VertexId(crate::usize_to_u32(station * n + j));
    let mut sections = Vec::with_capacity((count + 1) * n);
    for station in 0..=count {
        for (j, v) in vertices.iter().enumerate() {
            let a = vid(station, j);
            let b = vid(station, v.next);
            let origin = parts.vertices[a.index()];
            let direction = sub(parts.vertices[b.index()], origin);
            sections.push(push_edge(
                &mut parts,
                Edge {
                    curve: Curve::line(origin, direction).map_err(SweepError::Geometry)?,
                    range: ParameterRange::UNIT,
                    start: a,
                    end: b,
                    role: EdgeRole::Section {
                        station,
                        segment: v.source,
                    },
                },
            ));
        }
    }
    let mut area = 2.0 * profile.area();
    let mut volume = 0.0;
    let (section_area, moment) = moments(&vertices, path.section_origin);
    let mut rails = Vec::with_capacity(n);
    let mut coordinates = Vec::with_capacity(n);
    for (i, span) in spans.iter().enumerate() {
        rails.clear();
        coordinates.clear();
        for (j, v) in vertices.iter().enumerate() {
            let start = vid(i, j);
            let end = vid(i + 1, j);
            let p = parts.vertices[start.index()];
            let (curve, range, coordinate) = match span.motion {
                Motion::Line => (
                    Curve::line(p, sub(parts.vertices[end.index()], p))
                        .map_err(SweepError::Geometry)?,
                    ParameterRange::UNIT,
                    [0.0; 2],
                ),
                Motion::Arc {
                    center,
                    axis,
                    radial,
                    angle,
                    ..
                } => {
                    let offset = sub(p, center);
                    let radius = dot(offset, radial);
                    let h = dot(offset, axis);
                    if !radius.is_finite() || radius <= 0.0 {
                        return Err(SweepError::AxisContact {
                            path_segment: i,
                            vertex: v.source,
                            radius,
                        });
                    }
                    let f = Placement3::from_axes(
                        radial,
                        cross(axis, radial),
                        axis,
                        add(center, scale(axis, h)),
                    );
                    (
                        Curve::circle(f, radius).map_err(SweepError::Geometry)?,
                        ParameterRange::new(0.0, angle).map_err(SweepError::Geometry)?,
                        [radius, h],
                    )
                }
            };
            coordinates.push(coordinate);
            rails.push(push_edge(
                &mut parts,
                Edge {
                    curve,
                    range,
                    start,
                    end,
                    role: EdgeRole::Rail {
                        path_segment: i,
                        vertex: v.source,
                    },
                },
            ));
        }
        volume += match span.motion {
            Motion::Line => section_area * span.length,
            Motion::Arc {
                radius,
                radial,
                angle,
                ..
            } => {
                angle
                    * (radius * section_area
                        + dot(span.start.x, radial) * moment[0]
                        + dot(span.start.y, radial) * moment[1])
            }
        };
        for (j, v) in vertices.iter().enumerate() {
            let a = parts.vertices[vid(i, j).index()];
            let b = parts.vertices[vid(i, v.next).index()];
            let direction = sub(b, a);
            let length = norm(direction);
            let (surface, domain, sense) = match span.motion {
                Motion::Line => {
                    let x = scale(direction, 1.0 / length);
                    let t = span.start.tangent;
                    (
                        Surface::plane(Placement3::from_axes(x, t, cross(x, t), a))
                            .map_err(SweepError::Geometry)?,
                        ParameterDomain {
                            u: ParameterRange::new(0.0, length).map_err(SweepError::Geometry)?,
                            v: ParameterRange::new(0.0, span.length)
                                .map_err(SweepError::Geometry)?,
                        },
                        Sense::Forward,
                    )
                }
                Motion::Arc {
                    center,
                    axis,
                    radial,
                    angle,
                    ..
                } => {
                    let (surface, domain, sense) =
                        crate::revolve::wall(coordinates[v.next], coordinates[j], angle)
                            .map_err(SweepError::Geometry)?;
                    let place = Placement3::from_axes(radial, axis, cross(radial, axis), center);
                    (place_surface(surface, place)?, domain, sense)
                }
            };
            area += match span.motion {
                Motion::Line => length * span.length,
                Motion::Arc { angle, .. } => {
                    angle * 0.5 * (coordinates[j][0] + coordinates[v.next][0]) * length
                }
            };
            let boundary = push_loop(
                &mut parts,
                vec![
                    Coedge {
                        edge: sections[i * n + j],
                        reversed: false,
                    },
                    Coedge {
                        edge: rails[v.next],
                        reversed: false,
                    },
                    Coedge {
                        edge: sections[(i + 1) * n + j],
                        reversed: true,
                    },
                    Coedge {
                        edge: rails[j],
                        reversed: true,
                    },
                ],
            );
            parts.faces.push(Face {
                surface,
                domain,
                sense,
                loops: vec![boundary],
                role: FaceRole::Wall {
                    path_segment: i,
                    segment: v.source,
                    tag: v.tag,
                },
            });
        }
    }
    let bounds = profile.bounds();
    let domain = ParameterDomain {
        u: ParameterRange::new(bounds[0][0], bounds[1][0]).map_err(SweepError::Geometry)?,
        v: ParameterRange::new(bounds[0][1], bounds[1][1]).map_err(SweepError::Geometry)?,
    };
    for (station, end, section, sense) in [
        (0, End::Start, &spans[0].start, Sense::Reverse),
        (count, End::End, &spans[count - 1].end, Sense::Forward),
    ] {
        let mut loops = Vec::with_capacity(rings.len());
        for ring in &rings {
            let mut edges: Vec<_> = ring
                .clone()
                .map(|j| Coedge {
                    edge: sections[station * n + j],
                    reversed: station == 0,
                })
                .collect();
            if station == 0 {
                edges.reverse();
            }
            loops.push(push_loop(&mut parts, edges));
        }
        parts.faces.push(Face {
            surface: Surface::plane(section.frame(path.section_origin))
                .map_err(SweepError::Geometry)?,
            domain,
            sense,
            loops,
            role: FaceRole::Cap { end },
        });
    }
    if !area.is_finite() || !volume.is_finite() || volume <= 0.0 {
        return Err(SweepError::NumericLimit);
    }
    let size = parts
        .vertices
        .iter()
        .flatten()
        .fold(1.0_f64, |a, b| a.max(b.abs()));
    let body = Body::try_from_parts(
        parts,
        &ValidationPolicy {
            max_gap: 512.0 * f64::EPSILON * size,
        },
    )
    .map_err(SweepError::Boundary)?;
    let mut path_distances = Vec::with_capacity(spans.len() + 1);
    path_distances.push(0.0);
    for span in &spans {
        let next = *path_distances.last().unwrap() + span.length;
        if !next.is_finite() {
            return Err(SweepError::NumericLimit);
        }
        path_distances.push(next);
    }
    // The construction frames are temporary. Retain the source path and one
    // compact distance prefix, not duplicate frames at every span boundary.
    drop(spans);
    Ok(Sweep {
        profile,
        frame,
        body,
        path_start: path.start,
        path_segments: path.segments.to_vec(),
        section_x: path.section_x,
        joins: path.joins,
        path_distances,
        section_origin: path.section_origin,
        area,
        volume,
    })
}
fn place_surface(surface: Surface, frame: Placement3) -> Result<Surface, SweepError> {
    let placement = frame * surface.frame();
    match surface {
        Surface::Plane(_) => Surface::plane(placement),
        Surface::Cylinder(c) => Surface::cylinder(placement, c.radius()),
        Surface::Cone(c) => Surface::cone(placement, c.radius(), c.slope()),
    }
    .map_err(SweepError::Geometry)
}
fn moments(vertices: &[Vertex], datum: [f64; 2]) -> (f64, [f64; 2]) {
    let mut a = 0.0;
    let mut m = [0.0; 2];
    for v in vertices {
        let p = [v.point[0] - datum[0], v.point[1] - datum[1]];
        let q = [
            vertices[v.next].point[0] - datum[0],
            vertices[v.next].point[1] - datum[1],
        ];
        let c = p[0] * q[1] - q[0] * p[1];
        a += c;
        for k in 0..2 {
            m[k] += (p[k] + q[k]) * c;
        }
    }
    (a * 0.5, m.map(|v| v / 6.0))
}
fn push_edge(parts: &mut BodyParts<FaceRole, EdgeRole>, edge: Edge<EdgeRole>) -> EdgeId {
    let id = EdgeId(crate::usize_to_u32(parts.edges.len()));
    parts.edges.push(edge);
    id
}
fn push_loop(parts: &mut BodyParts<FaceRole, EdgeRole>, coedges: Vec<Coedge>) -> LoopId {
    let id = LoopId(crate::usize_to_u32(parts.loops.len()));
    parts.loops.push(BoundaryLoop { coedges });
    id
}
