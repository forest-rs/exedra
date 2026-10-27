// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{StepCounts, StepError, StepOptions};
use alloc::{vec, vec::Vec};
use exedra_analytic::{
    body::{Body, EdgeId, FaceId, ValidationPolicy},
    geometry::{Curve, FRAME_TOLERANCE, Surface},
};
use exedra_math::{Placement3, norm};

pub(super) enum SurfaceSupport {
    Plane,
    Cylinder { radius: f64 },
}

pub(super) fn surface_support(
    surface: &Surface,
    face: FaceId,
) -> Result<SurfaceSupport, StepError> {
    match surface {
        Surface::Plane(_) => Ok(SurfaceSupport::Plane),
        Surface::Cylinder(c) => Ok(SurfaceSupport::Cylinder { radius: c.radius() }),
        Surface::Cone(_) => Err(StepError::UnsupportedSurface {
            face,
            family: "cone",
        }),
        Surface::Torus(_) => Err(StepError::UnsupportedSurface {
            face,
            family: "torus",
        }),
        Surface::Ruled(_) => Err(StepError::UnsupportedSurface {
            face,
            family: "ruled",
        }),
    }
}

pub(super) struct Prepared {
    pub vertices: Vec<bool>,
    pub edges: Vec<bool>,
    pub loops: Vec<bool>,
    pub counts: StepCounts,
    pub gap: f64,
    pub allowance: f64,
}

pub(super) fn prepare<F, E>(
    body: &Body<F, E>,
    placement: Placement3,
    options: &StepOptions<'_>,
) -> Result<Prepared, StepError> {
    if !options.tolerance.is_finite()
        || options.tolerance <= 0.0
        || options.max_entities == 0
        || options.max_bytes == 0
    {
        return Err(StepError::InvalidOptions);
    }
    if !timestamp(options.timestamp) {
        return Err(StepError::Timestamp);
    }
    if options.name.len() > options.max_bytes {
        return Err(StepError::Budget {
            resource: "bytes",
            required: options.name.len(),
            maximum: options.max_bytes,
        });
    }
    let column = |i| placement.rows.map(|r| r[i]);
    Placement3::try_from_orthonormal_axes(
        column(0),
        column(1),
        column(2),
        column(3),
        FRAME_TOLERANCE,
    )
    .map_err(StepError::Placement)?;
    let work = body
        .vertices()
        .len()
        .saturating_add(body.edges().len())
        .saturating_add(body.loops().len())
        .saturating_add(body.faces().len())
        .saturating_add(
            body.loops()
                .iter()
                .map(|l| l.coedges.len())
                .fold(0_usize, usize::saturating_add),
        );
    if work > options.max_entities as usize {
        return Err(StepError::Budget {
            resource: "input tables",
            required: work,
            maximum: options.max_entities as usize,
        });
    }
    let validation = body
        .validate(&ValidationPolicy {
            max_gap: options.tolerance,
        })
        .map_err(StepError::Boundary)?;
    if body.faces().is_empty() {
        return Err(StepError::EmptyBody);
    }
    let mut vertices = vec![false; body.vertices().len()];
    let mut edges = vec![false; body.edges().len()];
    let mut loops = vec![false; body.loops().len()];
    let mut uses = vec![[0_u32; 2]; body.edges().len()];
    let mut first_face = vec![None; body.edges().len()];
    let mut parent: Vec<_> = (0..body.faces().len()).collect();
    let mut coedges = 0_u32;
    let mut scale = 0.0_f64;
    let magnitude = |p: [f64; 3]| p.into_iter().map(f64::abs).fold(0.0, f64::max);
    for (i, face) in body.faces().iter().enumerate() {
        let id = FaceId(u32::try_from(i).expect("input budget"));
        let radius = match surface_support(&face.surface, id)? {
            SurfaceSupport::Plane => 0.0,
            SurfaceSupport::Cylinder { radius } => {
                if face.domain.u.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON) {
                    return Err(StepError::CylinderDomain { face: id });
                }
                radius
            }
        };
        scale = scale.max(magnitude(face.surface.frame().rows.map(|r| r[3])) + radius);
        for &loop_id in &face.loops {
            loops[loop_id.index()] = true;
            for c in &body.loops()[loop_id.index()].coedges {
                coedges += 1;
                edges[c.edge.index()] = true;
                uses[c.edge.index()][usize::from(c.reversed)] += 1;
                if uses[c.edge.index()][usize::from(c.reversed)] > 1 {
                    return Err(StepError::EdgeIncidence {
                        edge: c.edge,
                        uses: uses[c.edge.index()],
                    });
                }
                if let Some(other) = first_face[c.edge.index()] {
                    let a = find(&mut parent, i);
                    let b = find(&mut parent, other);
                    parent[a] = b;
                } else {
                    first_face[c.edge.index()] = Some(i);
                }
            }
        }
    }
    for (i, (&active, edge)) in edges.iter().zip(body.edges()).enumerate() {
        if !active {
            continue;
        }
        let id = EdgeId(u32::try_from(i).expect("input budget"));
        if uses[i] != [1, 1] {
            return Err(StepError::EdgeIncidence {
                edge: id,
                uses: uses[i],
            });
        }
        vertices[edge.start.index()] = true;
        vertices[edge.end.index()] = true;
        match &edge.curve {
            Curve::Line(l) => {
                if !norm(l.direction()).is_finite() || norm(l.direction()) == 0.0 {
                    return Err(StepError::Numeric);
                }
                scale = scale.max(magnitude(l.origin()));
            }
            curve => {
                let (frame, radius) = match curve {
                    Curve::Circle(c) => (c.frame(), c.radius()),
                    Curve::Ellipse(e) => (e.frame(), e.semi_major()),
                    Curve::Line(_) => unreachable!(),
                };
                let span = edge.range.span();
                let full = (span - core::f64::consts::TAU).abs()
                    <= 64.0 * f64::EPSILON * core::f64::consts::TAU;
                if span > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON)
                    || (edge.start == edge.end) != full
                {
                    return Err(StepError::PeriodicCurveRange {
                        edge: id,
                        range: edge.range.bounds(),
                    });
                }
                scale = scale.max(magnitude(frame.rows.map(|r| r[3])) + radius);
            }
        }
    }
    let mut components = 0;
    for i in 0..parent.len() {
        if find(&mut parent, i) == i {
            components += 1;
        }
    }
    if components != 1 {
        return Err(StepError::ShellCount { found: components });
    }
    for (&active, &point) in vertices.iter().zip(body.vertices()) {
        if active {
            scale = scale.max(magnitude(point));
            if placement
                .transform_point(point)
                .into_iter()
                .any(|v| !v.is_finite())
            {
                return Err(StepError::Numeric);
            }
        }
    }
    let world = scale * 4.0 + magnitude(placement.rows.map(|r| r[3]));
    // A STEP axis placement normalizes ratios and orthogonalizes its X/Z axes.
    // Each source/occurrence frame has checked axis errors <= FRAME_TOLERANCE.
    // This headroom covers that correction plus placed coordinate arithmetic;
    // no curve fitting or mesh approximation is introduced.
    let allowance = 32.0 * FRAME_TOLERANCE * scale + 128.0 * f64::EPSILON * world;
    let required = validation.max_gap + allowance;
    if !required.is_finite() {
        return Err(StepError::Numeric);
    }
    if required > options.tolerance {
        return Err(StepError::Accuracy {
            required,
            tolerance: options.tolerance,
        });
    }
    let count =
        |flags: &[bool]| u32::try_from(flags.iter().filter(|&&v| v).count()).expect("input budget");
    let counts = StepCounts {
        entities: 0,
        vertices: count(&vertices),
        edges: count(&edges),
        coedges,
        loops: count(&loops),
        faces: u32::try_from(body.faces().len()).expect("input budget"),
        shells: 1,
        solids: 1,
    };
    Ok(Prepared {
        vertices,
        edges,
        loops,
        counts,
        gap: validation.max_gap,
        allowance,
    })
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
fn timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 19 || b.len() > 30 {
        return false;
    }
    for (i, &byte) in b.iter().take(19).enumerate() {
        let expected = match i {
            4 | 7 => b'-',
            10 => b'T',
            13 | 16 => b':',
            _ => 0,
        };
        if (expected == 0 && !byte.is_ascii_digit()) || (expected != 0 && byte != expected) {
            return false;
        }
    }
    let n = |a: usize, z: usize| {
        b[a..z]
            .iter()
            .fold(0_u32, |v, &c| v * 10 + u32::from(c - b'0'))
    };
    let (year, month, day) = (n(0, 4), n(5, 7), n(8, 10));
    if !(1..=12).contains(&month) || day == 0 || n(11, 13) > 23 || n(14, 16) > 59 || n(17, 19) > 59
    {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > days[(month - 1) as usize] {
        return false;
    }
    let tail = &b[19..];
    tail.is_empty()
        || tail == b"Z"
        || (tail.first() == Some(&b'.') && {
            let digits = tail.strip_suffix(b"Z").unwrap_or(tail);
            digits.len() > 1 && digits[1..].iter().all(u8::is_ascii_digit)
        })
}
