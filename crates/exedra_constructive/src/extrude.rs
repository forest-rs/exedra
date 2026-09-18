// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Closed profile extrusions terminated by an authored plane.
//!
//! This evaluated-body API owns forward termination and composes ordinary
//! extrusion with checked plane cutting. Recipe IR and arbitrary target-surface
//! termination are separate concerns. Geometry follows the discretized profile;
//! the target cap uses the section triangulator and its realization checks.

use crate::discretize::discretize_profile;
use crate::ir::{CapMode, Placement3, Plane3};
use crate::profile::{Profile2, SegKind};
use crate::section::{CutCap, PlaneSection, SectionError, SectionPolicy, split_body};
use crate::tessellate::{
    EvalPolicy, Feature, REGION_CAP_END, TessellateError, TessellatedBody, det3, tessellate_extrude,
};
use exedra_math::{dot, norm};

/// The closed extrusion and its terminating section, including cut measurements.
#[derive(Debug)]
pub struct PlaneExtrusion {
    /// Closed body. Walls and the start cap retain extrusion provenance and
    /// regions; the terminating cap uses `REGION_CAP_END` and `Feature::CapEnd`.
    /// Material slots may be bound to these regions by the assembly consumer.
    pub body: TessellatedBody,
    /// Terminating profile in the target plane's deterministic frame.
    pub section: PlaneSection,
}

/// A forward plane-terminated extrusion could not be constructed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ExtrudeToPlaneError {
    /// Placement, plane, or section policy is nonfinite, singular, or invalid.
    InvalidInput,
    /// The extrusion direction is parallel to the target plane.
    Parallel,
    /// The complete profile cannot be established strictly before the plane.
    /// This includes crossing, touching, and backward termination.
    NotForward,
    /// A finite enclosing extrusion cannot be represented.
    NumericLimit,
    /// Profile sampling or the enclosing extrusion failed.
    Tessellation(TessellateError),
    /// Checked termination failed; no partial body is returned.
    Section(SectionError),
}
impl core::fmt::Display for ExtrudeToPlaneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid plane extrusion input"),
            Self::Parallel => f.write_str("extrusion direction is parallel to target plane"),
            Self::NotForward => {
                f.write_str("target plane is not strictly ahead of the whole profile")
            }
            Self::NumericLimit => f.write_str("plane extrusion exceeds numeric realization limits"),
            Self::Tessellation(error) => write!(f, "plane extrusion: {error}"),
            Self::Section(error) => write!(f, "plane extrusion termination: {error}"),
        }
    }
}
impl core::error::Error for ExtrudeToPlaneError {}

/// Extrudes a profile along the placement's local +Z axis to `plane`.
///
/// The plane is in output/body coordinates. Placement may include reflection,
/// scale, and shear, but must be finite and nonsingular. The whole profile must
/// terminate strictly forward, with clearance greater than the section distance
/// tolerance. Curved profiles additionally reserve their discretization chord
/// bound projected onto the plane normal: uncertain near-contact cases are
/// refused, even when a finer sampling could establish clearance.
///
/// The enclosing extrusion extent is computed internally. Both caps are closed;
/// holes remain holes. Start-cap and wall regions follow ordinary extrusion,
/// while the terminating cap has region [`REGION_CAP_END`], section-frame UVs,
/// and sharp rims. This is an additive evaluated-body API; existing extrusion
/// callers and recipe formats are unchanged. It does not certify distant
/// self-intersections or promise analytic curved surfaces.
///
/// # Errors
/// Returns typed errors for invalid inputs, parallel rays, crossing/touching or
/// backward termination, exhausted sampling/cut budgets, or geometry that cannot
/// survive f32 mesh storage. Near-parallel finite directions are attempted and
/// subject to the same numeric and distance checks.
///
/// ```
/// use exedra_constructive::{builders::rect_from_corner, extrude::extrude_to_plane,
///     ir::{Placement3, Plane3}, section::SectionPolicy, tessellate::EvalPolicy};
/// let result = extrude_to_plane(&rect_from_corner(2.0, 1.0)?, &Placement3::IDENTITY,
///     Plane3 { normal: [-0.25, 0.0, 1.0], distance: 2.0 },
///     &EvalPolicy::default(), &SectionPolicy::default())?;
/// assert!(result.body.mesh.boundary_loops()?.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn extrude_to_plane(
    profile: &Profile2,
    placement: &Placement3,
    plane: Plane3,
    evaluation: &EvalPolicy,
    section_policy: &SectionPolicy,
) -> Result<PlaneExtrusion, ExtrudeToPlaneError> {
    use ExtrudeToPlaneError as Error;
    let determinant = det3(placement);
    if placement.rows.iter().flatten().any(|v| !v.is_finite())
        || !determinant.is_finite()
        || determinant == 0.0
        || !section_policy.distance_tolerance.is_finite()
        || section_policy.distance_tolerance <= 0.0
        || section_policy.max_triangles == 0
        || section_policy.max_section_vertices == 0
        || section_policy.max_pair_checks == 0
    {
        return Err(Error::InvalidInput);
    }
    let (normal, distance) = plane.normalized().ok_or(Error::InvalidInput)?;
    let column = |i| placement.rows.map(|row| row[i]);
    let denominator = dot(normal, column(2));
    if !denominator.is_finite() {
        return Err(Error::NumericLimit);
    }
    if denominator == 0.0 {
        return Err(Error::Parallel);
    }
    let sampled = discretize_profile(profile, &evaluation.discretize)
        .map_err(|error| Error::Tessellation(error.into()))?;
    let curved = core::iter::once(profile.outer())
        .chain(profile.holes())
        .flat_map(|ring| ring.segs())
        .any(|segment| {
            let geometry = match &segment.kind {
                SegKind::PolicyTo { realized, .. } => realized.as_ref(),
                kind => kind,
            };
            !matches!(geometry, SegKind::Line)
        });
    let projected_error = if curved {
        evaluation.discretize.chord_tolerance
            * norm([dot(normal, column(0)), dot(normal, column(1)), 0.0])
    } else {
        0.0
    };
    let clearance = section_policy.distance_tolerance + projected_error;
    if !clearance.is_finite() {
        return Err(Error::NumericLimit);
    }
    let mut max_height = 0.0_f64;
    for ring in core::iter::once(&sampled.outer).chain(&sampled.holes) {
        for p in &ring.points {
            let start = placement
                .rows
                .map(|row| row[0] * p[0] + row[1] * p[1] + row[3]);
            let remaining = (distance - dot(normal, start)) * denominator.signum();
            if !remaining.is_finite() {
                return Err(Error::NumericLimit);
            }
            if remaining <= clearance {
                return Err(Error::NotForward);
            }
            max_height = max_height.max((remaining + clearance) / denominator.abs());
        }
    }
    // Twice the bounded maximum leaves both original rings strictly away from
    // the cut. This is an internal construction extent, never a caller guess.
    let height = 2.0 * max_height;
    if !height.is_finite() || height <= 0.0 {
        return Err(Error::NumericLimit);
    }
    let envelope = tessellate_extrude(profile, placement, height, CapMode::Both, evaluation)
        .map_err(Error::Tessellation)?;
    // Quantization must not move any original cap through the target. Checking
    // every cap corner also covers optional cap-refinement vertices.
    for face in envelope.mesh.faces() {
        let sign = match envelope.source_map.face_feature(face) {
            Some(Feature::CapStart) => 1.0,
            Some(Feature::CapEnd) => -1.0,
            _ => continue,
        };
        for corner in envelope.mesh.face_loop(face) {
            let position = envelope
                .mesh
                .vertex_position(envelope.mesh.to_vertex(corner).ok_or(Error::NumericLimit)?)
                .ok_or(Error::NumericLimit)?
                .map(f64::from);
            let remaining = (distance - dot(normal, position)) * denominator.signum() * sign;
            if !remaining.is_finite() || remaining <= section_policy.distance_tolerance {
                return Err(Error::NumericLimit);
            }
        }
    }
    let split = split_body(
        &envelope,
        plane,
        section_policy,
        CutCap {
            region: REGION_CAP_END,
            material: None,
        },
    )
    .map_err(Error::Section)?;
    let mut body = if denominator > 0.0 {
        split.negative
    } else {
        split.positive
    }
    .ok_or(Error::NumericLimit)?;
    if split.section.regions.is_empty() {
        return Err(Error::NumericLimit);
    }
    let faces = body
        .source_map
        .face_features()
        .iter()
        .map(|&feature| {
            if feature == Feature::PlaneCutCap {
                Feature::CapEnd
            } else {
                feature
            }
        })
        .collect();
    body.source_map = crate::source_map::SourceMap::new(
        &body.mesh,
        faces,
        body.mesh
            .vertices()
            .map(|vertex| {
                body.source_map
                    .vertex_feature(vertex)
                    .expect("current extrusion provenance")
            })
            .collect(),
    );
    Ok(PlaneExtrusion {
        body,
        section: split.section,
    })
}

#[cfg(test)]
mod tests;
