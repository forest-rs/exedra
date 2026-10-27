// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native loft tables with authoritative shared section edges and vertex rails.

use super::{EdgeRole, FaceRole, Loft, LoftError, LoftPolicy, loops};
use crate::{
    End,
    body::{
        Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, Sense, ValidationPolicy,
        VertexId,
    },
    geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
};
use alloc::{vec, vec::Vec};
use exedra_math::{Placement3, dot, norm, sub};
use exedra_profile::{IntoProfile, LoopRef, SegmentRef};

pub(super) fn construct<P: IntoProfile>(
    inputs: impl IntoIterator<Item = (Placement3, P)>,
    frame: Placement3,
    policy: &LoftPolicy,
) -> Result<Loft, LoftError<P::Error>> {
    if policy.max_sections < 2
        || policy.max_native_elements == 0
        || !policy.profile.min_separation.is_finite()
        || policy.profile.min_separation <= 0.0
        || policy.profile.max_segments < 2
    {
        return Err(LoftError::InvalidPolicy);
    }
    crate::geometry::rigid(frame).map_err(|error| LoftError::Frame {
        section: None,
        error,
    })?;
    let mut sections = Vec::new();
    let mut input_segments = 0_usize;
    for (i, (placement, input)) in inputs.into_iter().enumerate() {
        if i >= policy.max_sections as usize {
            return Err(LoftError::Budget {
                storage: "sections",
                required: i + 1,
                maximum: policy.max_sections,
            });
        }
        crate::geometry::rigid(placement).map_err(|error| LoftError::Frame {
            section: Some(i),
            error,
        })?;
        let profile = input
            .into_profile(&policy.profile)
            .map_err(|error| LoftError::Profile { section: i, error })?;
        input_segments = input_segments.saturating_add(
            loops(&profile)
                .map(|(_, r)| r.segments().len())
                .sum::<usize>(),
        );
        if input_segments > policy.max_native_elements as usize {
            return Err(LoftError::Budget {
                storage: "source segments",
                required: input_segments,
                maximum: policy.max_native_elements,
            });
        }
        sections.push((placement, profile));
    }
    if sections.len() < 2 {
        return Err(LoftError::Sections);
    }
    let first = &sections[0].1;
    let mut offsets = Vec::new();
    let mut sources = Vec::new();
    let mut next = Vec::new();
    for (lr, ring) in loops(first) {
        let base = sources.len();
        offsets.push(base);
        for i in 0..ring.segments().len() {
            sources.push(SegmentRef {
                loop_ref: lr,
                segment: i,
            });
            next.push(base + (i + 1) % ring.segments().len());
        }
    }
    let n = sources.len();
    let ns = sections.len();
    for (i, (_, p)) in sections.iter().enumerate().skip(1) {
        if p.holes().len() != first.holes().len() {
            return Err(LoftError::Correspondence {
                section: i,
                loop_ref: None,
            });
        }
        for ((lr, a), (_, b)) in loops(first).zip(loops(p)) {
            if a.segments().len() != b.segments().len() {
                return Err(LoftError::Correspondence {
                    section: i,
                    loop_ref: Some(lr),
                });
            }
        }
    }
    let vertices = ns.saturating_mul(n);
    let edges = vertices.saturating_add((ns - 1).saturating_mul(n));
    let walls = (ns - 1).saturating_mul(n);
    let required = vertices
        .saturating_add(edges)
        .saturating_add(walls)
        .saturating_add(2 * offsets.len())
        .saturating_add(walls)
        .saturating_add(2);
    if required > policy.max_native_elements as usize {
        return Err(LoftError::Budget {
            storage: "native tables",
            required,
            maximum: policy.max_native_elements,
        });
    }
    let mut parts = BodyParts {
        vertices: Vec::with_capacity(vertices),
        edges: Vec::with_capacity(edges),
        loops: Vec::with_capacity(walls + 2 * offsets.len()),
        faces: Vec::with_capacity(walls + 2),
    };
    for (section, (placement, profile)) in sections.iter().enumerate() {
        for (lr, ring) in loops(profile) {
            let base = offsets[super::loop_index(lr)];
            for (i, (from, seg)) in ring.iter_with_starts().enumerate() {
                let a = placement.transform_point([from[0], from[1], 0.0]);
                let b = placement.transform_point([seg.to[0], seg.to[1], 0.0]);
                parts.vertices.push(a);
                let (curve, range) = if let Some(arc) = ring.arc(i) {
                    (
                        crate::profile_geometry::circle(arc, *placement),
                        ParameterRange::new(0.0, arc.sweep.abs()),
                    )
                } else {
                    (Curve::line(a, sub(b, a)), Ok(ParameterRange::UNIT))
                };
                let failure = |error| LoftError::Wall {
                    band: section.min(ns - 2),
                    segment: sources[base + i],
                    error,
                };
                parts.edges.push(Edge {
                    curve: curve.map_err(failure)?,
                    range: range.map_err(failure)?,
                    start: VertexId(crate::usize_to_u32(section * n + base + i)),
                    end: VertexId(crate::usize_to_u32(section * n + next[base + i])),
                    role: EdgeRole::Section {
                        section,
                        segment: sources[base + i],
                    },
                });
            }
        }
    }
    for band in 0..ns - 1 {
        let normals = [
            sections[band].0.rows.map(|r| r[2]),
            sections[band + 1].0.rows.map(|r| r[2]),
        ];
        for (k, &vertex) in sources.iter().enumerate() {
            let a = parts.vertices[band * n + k];
            let b = parts.vertices[(band + 1) * n + k];
            let d = sub(b, a);
            if normals
                .into_iter()
                .any(|normal| dot(d, normal) <= 128.0 * f64::EPSILON * norm(d))
            {
                return Err(LoftError::Direction { band, vertex });
            }
            let curve = Curve::line(a, d).map_err(|error| LoftError::Wall {
                band,
                segment: vertex,
                error,
            })?;
            parts.edges.push(Edge {
                curve,
                range: ParameterRange::UNIT,
                start: VertexId(crate::usize_to_u32(band * n + k)),
                end: VertexId(crate::usize_to_u32((band + 1) * n + k)),
                role: EdgeRole::Rail { band, vertex },
            });
        }
    }
    for band in 0..ns - 1 {
        for (k, &segment) in sources.iter().enumerate() {
            let failure = |error| LoftError::Wall {
                band,
                segment,
                error,
            };
            let span =
                |edge: &Edge<EdgeRole>| CurveSpan::new(edge.curve.clone(), edge.range, false);
            let surface = Surface::ruled(
                span(&parts.edges[band * n + k]).map_err(failure)?,
                span(&parts.edges[(band + 1) * n + k]).map_err(failure)?,
            )
            .map_err(failure)?;
            let id = LoopId(crate::usize_to_u32(parts.loops.len()));
            parts.loops.push(BoundaryLoop {
                coedges: vec![
                    Coedge {
                        edge: EdgeId(crate::usize_to_u32(band * n + k)),
                        reversed: false,
                    },
                    Coedge {
                        edge: EdgeId(crate::usize_to_u32(vertices + band * n + next[k])),
                        reversed: false,
                    },
                    Coedge {
                        edge: EdgeId(crate::usize_to_u32((band + 1) * n + k)),
                        reversed: true,
                    },
                    Coedge {
                        edge: EdgeId(crate::usize_to_u32(vertices + band * n + k)),
                        reversed: true,
                    },
                ],
            });
            let tag = |section: usize| {
                let p = &sections[section].1;
                let r = match segment.loop_ref {
                    LoopRef::Outer => p.outer(),
                    LoopRef::Hole(i) => &p.holes()[i],
                };
                r.segments()[segment.segment].tag
            };
            parts.faces.push(Face {
                surface,
                domain: ParameterDomain {
                    u: ParameterRange::UNIT,
                    v: ParameterRange::UNIT,
                },
                sense: Sense::Forward,
                loops: vec![id],
                role: FaceRole::Wall {
                    band,
                    segment,
                    tags: [tag(band), tag(band + 1)],
                },
            });
        }
    }
    for (end, section) in [(End::Start, 0), (End::End, ns - 1)] {
        let (placement, profile) = &sections[section];
        let mut face_loops = Vec::with_capacity(offsets.len());
        for (lr, ring) in loops(profile) {
            let base = offsets[super::loop_index(lr)];
            let mut coedges: Vec<_> = (0..ring.segments().len())
                .map(|i| Coedge {
                    edge: EdgeId(crate::usize_to_u32(section * n + base + i)),
                    reversed: end == End::Start,
                })
                .collect();
            if end == End::Start {
                coedges.reverse();
            }
            face_loops.push(LoopId(crate::usize_to_u32(parts.loops.len())));
            parts.loops.push(BoundaryLoop { coedges });
        }
        let bounds = profile.bounds();
        let surface = Surface::plane(*placement).map_err(|error| LoftError::Frame {
            section: Some(section),
            error,
        })?;
        let domain = ParameterDomain {
            u: ParameterRange::new(bounds[0][0], bounds[1][0]).map_err(|error| {
                LoftError::Frame {
                    section: Some(section),
                    error,
                }
            })?,
            v: ParameterRange::new(bounds[0][1], bounds[1][1]).map_err(|error| {
                LoftError::Frame {
                    section: Some(section),
                    error,
                }
            })?,
        };
        parts.faces.push(Face {
            surface,
            domain,
            sense: if end == End::Start {
                Sense::Reverse
            } else {
                Sense::Forward
            },
            loops: face_loops,
            role: FaceRole::Cap { end },
        });
    }
    let body = Body::try_from_parts(
        parts,
        &ValidationPolicy {
            max_gap: policy.profile.min_separation * 0.25,
        },
    )
    .map_err(LoftError::Boundary)?;
    Ok(Loft {
        sections,
        frame,
        body,
        ring_offsets: offsets,
        section_vertices: n,
    })
}
