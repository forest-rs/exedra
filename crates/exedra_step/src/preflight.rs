// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{StepCounts, StepError, StepOptions};
use alloc::{vec, vec::Vec};
use exedra_analytic::{
    body::{Body, EdgeId, FaceId, ValidationPolicy},
    geometry::{Curve, FRAME_TOLERANCE, Surface},
};
use exedra_math::{Placement3, norm};

// Headroom for atan rounding, STEP decimal conversion and a reader's trig
// evaluation. It bounds angle error in radians, not a tessellation deviation.
const ANGLE_ERROR: f64 = 16.0 * f64::EPSILON;
// The independent OCCT reader silently clamps smaller positive angles to this
// floor. Refuse that interoperability loss rather than changing the geometry.
const MIN_CONE_ANGLE: f64 = 1e-12;

pub(super) enum SurfaceSupport {
    Plane,
    Cylinder {
        radius: f64,
    },
    Cone {
        radius: f64,
    },
    Sphere {
        radius: f64,
    },
    Torus {
        major_radius: f64,
        minor_radius: f64,
    },
}

pub(super) fn surface_support(
    surface: &Surface,
    face: FaceId,
) -> Result<SurfaceSupport, StepError> {
    match surface {
        Surface::Plane(_) => Ok(SurfaceSupport::Plane),
        Surface::Cylinder(c) => Ok(SurfaceSupport::Cylinder { radius: c.radius() }),
        Surface::Cone(c) if c.slope() == 0.0 => Ok(SurfaceSupport::Cylinder { radius: c.radius() }),
        Surface::Cone(c) => {
            let angle = c.semi_angle();
            if angle.abs() < MIN_CONE_ANGLE
                || angle.abs() >= core::f64::consts::FRAC_PI_2
                || c.slope().abs() * ANGLE_ERROR >= 0.25
            {
                return Err(StepError::ConeAngle {
                    face,
                    slope: c.slope(),
                    semi_angle: angle,
                    angle_range: [MIN_CONE_ANGLE, core::f64::consts::FRAC_PI_2],
                });
            }
            Ok(SurfaceSupport::Cone { radius: c.radius() })
        }
        Surface::Torus(t) => Ok(SurfaceSupport::Torus {
            major_radius: t.major_radius(),
            minor_radius: t.minor_radius(),
        }),
        Surface::Sphere(s) => Ok(SurfaceSupport::Sphere { radius: s.radius() }),
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
    pub parameter_allowance: f64,
    pub max_deviation: f64,
    pub material: Option<exedra_analytic::rectilinear::MaterialValidationReport>,
    pub reversed_faces: Vec<bool>,
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
    let mut parameter_allowance = 0.0_f64;
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
            SurfaceSupport::Cone { radius } => {
                if face.domain.u.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON) {
                    return Err(StepError::PeriodicSurfaceDomain {
                        face: id,
                        parameter: "u",
                        range: face.domain.u.bounds(),
                    });
                }
                let Surface::Cone(cone) = &face.surface else {
                    unreachable!()
                };
                let heights = face.domain.v.bounds();
                let mut height = heights.into_iter().map(f64::abs).fold(0.0, f64::max);
                let frame = cone.frame();
                for &loop_id in &face.loops {
                    for coedge in &body.loops()[loop_id.index()].coedges {
                        let edge = &body.edges()[coedge.edge.index()];
                        let bounds = if let Some((curve, range)) = edge.regular() {
                            curve
                                .conservative_bounds(range)
                                .map_err(|_| StepError::Numeric)?
                        } else {
                            [body.vertices()[edge.start.index()]; 2]
                        };
                        let mut curve_height = 0.0;
                        for (axis, row) in frame.rows.into_iter().enumerate() {
                            let extent = (bounds[0][axis] - row[3])
                                .abs()
                                .next_up()
                                .max((bounds[1][axis] - row[3]).abs().next_up());
                            // Checked frames can be nearly rigid. The Gram
                            // inverse differs from transpose by <16*FRAME_TOLERANCE
                            // per coefficient; outward arithmetic encloses the
                            // true axial coordinate of the entire bounded curve.
                            let coefficient = (row[2].abs() + 16.0 * FRAME_TOLERANCE).next_up();
                            curve_height =
                                (curve_height + (extent * coefficient).next_up()).next_up();
                        }
                        if !curve_height.is_finite() {
                            return Err(StepError::Numeric);
                        }
                        height = height.max(curve_height);
                    }
                }
                let slope = cone.slope().abs();
                // tan(a+d)-tan(a) <= d*(1+|s|)^2/(1-|s|*d),
                // using |tan(d)| <= 2*d here. Fourfold headroom covers
                // arithmetic rounding and bounds radial error at equal height.
                let error = height
                    * (1.0 + slope)
                    * (ANGLE_ERROR * (1.0 + slope) / (1.0 - 2.0 * slope * ANGLE_ERROR))
                    * 4.0;
                parameter_allowance = parameter_allowance.max(error);
                let outer = heights
                    .into_iter()
                    .filter_map(|h| cone.radius_at(h))
                    .fold(radius, f64::max);
                outer + height
            }
            SurfaceSupport::Sphere { radius } => {
                if face.domain.u.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON) {
                    return Err(StepError::PeriodicSurfaceDomain {
                        face: id,
                        parameter: "u",
                        range: face.domain.u.bounds(),
                    });
                }
                radius
            }
            SurfaceSupport::Torus {
                major_radius,
                minor_radius,
            } => {
                for (parameter, range) in [("u", face.domain.u), ("v", face.domain.v)] {
                    if range.span() > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON) {
                        return Err(StepError::PeriodicSurfaceDomain {
                            face: id,
                            parameter,
                            range: range.bounds(),
                        });
                    }
                }
                major_radius + minor_radius
            }
        };
        scale = scale.max(magnitude(face.surface.frame().rows.map(|r| r[3])) + radius);
        for &loop_id in &face.loops {
            loops[loop_id.index()] = true;
            for c in &body.loops()[loop_id.index()].coedges {
                edges[c.edge.index()] = true;
                if body.edges()[c.edge.index()].is_collapsed() {
                    continue;
                }
                coedges += 1;
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
        vertices[edge.start.index()] = true;
        vertices[edge.end.index()] = true;
        let Some((curve, range)) = edge.regular() else {
            continue;
        };
        if uses[i] != [1, 1] {
            return Err(StepError::EdgeIncidence {
                edge: id,
                uses: uses[i],
            });
        }
        match curve {
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
                let span = range.span();
                let full = (span - core::f64::consts::TAU).abs()
                    <= 64.0 * f64::EPSILON * core::f64::consts::TAU;
                if span > core::f64::consts::TAU * (1.0 + 64.0 * f64::EPSILON)
                    || (edge.start == edge.end) != full
                {
                    return Err(StepError::PeriodicCurveRange {
                        edge: id,
                        range: range.bounds(),
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
    let material = if components == 1 {
        None
    } else {
        Some(
            exedra_analytic::rectilinear::validate_material(body, &options.shell_validation)
                .map_err(StepError::ShellValidation)?,
        )
    };
    let mut reversed_faces = Vec::new();
    if let Some(material) = &material {
        reversed_faces.resize(body.faces().len(), false);
        for &shell in &material.void_shells {
            for &face in &material.boundary.shells[shell].faces {
                reversed_faces[face.index()] = true;
            }
        }
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
    let material_bound = material.as_ref().map_or(0.0, |evidence| {
        evidence.boundary.geometry.max_gap + evidence.arithmetic_allowance
    });
    let required = (validation.max_gap + allowance + parameter_allowance).max(material_bound);
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
        edges: u32::try_from(
            edges
                .iter()
                .zip(body.edges())
                .filter(|(active, edge)| **active && !edge.is_collapsed())
                .count(),
        )
        .expect("input budget"),
        coedges,
        loops: count(&loops),
        faces: u32::try_from(body.faces().len()).expect("input budget"),
        shells: u32::try_from(components).expect("input budget"),
        solids: 1,
    };
    Ok(Prepared {
        vertices,
        edges,
        loops,
        counts,
        gap: validation.max_gap,
        allowance,
        parameter_allowance,
        max_deviation: required,
        material,
        reversed_faces,
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrepresentable_or_unconditioned_cone_angles_are_precise_refusals() {
        for slope in [
            f64::MAX,
            -f64::MAX,
            1e14,
            -1e14,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::from_bits(1),
            1e-13,
            -1e-13,
            1e-14,
            -1e-14,
        ] {
            let surface = Surface::cone(Placement3::IDENTITY, 1.0, slope).unwrap();
            assert!(
                matches!(surface_support(&surface,FaceId(3)),Err(StepError::ConeAngle{face:FaceId(3),slope:s,..}) if s==slope)
            );
        }
        for slope in [1e-12, -1e-12, 1.0, -1.0] {
            assert!(matches!(
                surface_support(
                    &Surface::cone(Placement3::IDENTITY, 1.0, slope).unwrap(),
                    FaceId(3)
                ),
                Ok(SurfaceSupport::Cone { .. })
            ));
        }
    }
}
