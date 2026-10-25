// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An exact analytic extrusion reached by two routes, then realized.
//!
//! A plate with a round hole is described twice: by hand, as an
//! [`exedra::profile::Profile`] of lines and arcs, and the way a spec compiler
//! would reach it, through constructive builders and
//! [`exedra::constructive::profile::Profile2::to_line_arc`]. Both give the same
//! profile, so the same [`Extrusion`] and the same realized mesh. The mesh can
//! then be compiled as baked geometry while the caller keeps the realization
//! report and correspondence.

use exedra::analytic::extrusion::{Extrusion, ExtrusionError};
use exedra::constructive::builders;
use exedra::constructive::ir::Placement3;
use exedra::constructive::profile::LineArcError;
use exedra::profile::{Loop, LoopError, Profile, ProfileError, ProfilePolicy, Segment, SegmentTag};

/// Plate dimensions in millimeters, centered on the origin.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlateSpec {
    /// Length along X.
    pub width: f64,
    /// Length along Y.
    pub depth: f64,
    /// Thickness along Z, from the profile plane.
    pub thickness: f64,
    /// Hole radius.
    pub hole_radius: f64,
    /// Hole center along X.
    pub hole_x: f64,
}

impl Default for PlateSpec {
    fn default() -> Self {
        Self {
            width: 80.0,
            depth: 40.0,
            thickness: 6.0,
            hole_radius: 5.0,
            hole_x: 20.0,
        }
    }
}

/// Separation every plate profile keeps between its boundaries.
pub const POLICY: ProfilePolicy = ProfilePolicy::new(1e-6);

/// Why a plate could not be described.
#[derive(Clone, Debug, PartialEq)]
pub enum PlateError {
    /// A handwritten loop is malformed.
    Loop(LoopError),
    /// The loops do not form a valid profile, such as a hole that escapes.
    Profile(ProfileError),
    /// The constructive profile has no line and arc form, or is invalid.
    Constructive(String),
    /// The extrusion is invalid, such as a zero thickness.
    Extrusion(ExtrusionError),
}

impl core::fmt::Display for PlateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Loop(error) => write!(f, "plate loop: {error}"),
            Self::Profile(error) => write!(f, "plate profile: {error}"),
            Self::Constructive(error) => write!(f, "constructive plate: {error}"),
            Self::Extrusion(error) => write!(f, "plate extrusion: {error}"),
        }
    }
}

impl core::error::Error for PlateError {}

/// The plate drawn by hand: the outline counter-clockwise from its lower
/// left corner, and the hole clockwise as two half circles.
///
/// Segment order and tags follow the constructive builders, so both routes
/// produce identical profiles.
///
/// # Errors
///
/// Returns the loop or arrangement error for dimensions that do not fit.
pub fn handwritten_profile(spec: &PlateSpec) -> Result<Profile, PlateError> {
    let (x, y) = (0.5 * spec.width, 0.5 * spec.depth);
    let outer = Loop::new(vec![
        Segment::line([x, -y]).tagged(SegmentTag(0)),
        Segment::line([x, y]).tagged(SegmentTag(1)),
        Segment::line([-x, y]).tagged(SegmentTag(2)),
        Segment::line([-x, -y]).tagged(SegmentTag(3)),
    ])
    .map_err(PlateError::Loop)?;
    let (center, r) = (spec.hole_x, spec.hole_radius);
    let hole = Loop::new(vec![
        Segment::arc([center + r, 0.0], -1.0).tagged(SegmentTag(1)),
        Segment::arc([center - r, 0.0], -1.0).tagged(SegmentTag(0)),
    ])
    .map_err(PlateError::Loop)?;
    Profile::new(outer, vec![hole], &POLICY).map_err(PlateError::Profile)
}

/// The plate reached through constructive builders, as a compiler would.
///
/// # Errors
///
/// Returns the constructive or arrangement error for dimensions that do not fit.
pub fn compiled_profile(spec: &PlateSpec) -> Result<Profile, PlateError> {
    let constructive = |error: exedra::constructive::profile::ProfileError| {
        PlateError::Constructive(error.to_string())
    };
    let hole = builders::circle(spec.hole_radius)
        .and_then(|circle| circle.translated((spec.hole_x, 0.0)))
        .map_err(constructive)?;
    let plate = builders::rect_centered(spec.width, spec.depth)
        .and_then(|outline| outline.with_cutouts([hole]))
        .map_err(constructive)?;
    plate.to_line_arc(&POLICY).map_err(|error| match error {
        LineArcError::Profile(error) => PlateError::Profile(error),
        other => PlateError::Constructive(other.to_string()),
    })
}

/// Extrudes a plate profile by the spec's thickness.
///
/// # Errors
///
/// Returns [`PlateError::Extrusion`] for a nonpositive thickness.
pub fn extrude(profile: Profile, spec: &PlateSpec) -> Result<Extrusion, PlateError> {
    Extrusion::new(profile, Placement3::IDENTITY, spec.thickness).map_err(PlateError::Extrusion)
}

#[cfg(test)]
mod tests {
    use super::*;
    use exedra::analytic::extrusion::{FaceRole, TessellationPolicy};
    use exedra::profile::LoopRef;

    #[test]
    fn both_routes_give_the_same_solid_and_mesh() {
        let spec = PlateSpec::default();
        let by_hand = extrude(handwritten_profile(&spec).unwrap(), &spec).unwrap();
        let compiled = extrude(compiled_profile(&spec).unwrap(), &spec).unwrap();
        assert_eq!(by_hand, compiled);
        let policy = TessellationPolicy::default();
        let (a, b) = (
            by_hand.tessellate(&policy).unwrap(),
            compiled.tessellate(&policy).unwrap(),
        );
        let bits = |mesh: &exedra::Mesh| -> Vec<[u32; 3]> {
            mesh.vertices()
                .map(|v| mesh.vertex_position(v).unwrap().map(f32::to_bits))
                .collect()
        };
        assert_eq!(bits(&a.mesh), bits(&b.mesh));
        assert_eq!(a.report, b.report);
    }

    #[test]
    fn hole_walls_are_selected_by_role_and_measured_exactly() {
        let spec = PlateSpec::default();
        let solid = extrude(handwritten_profile(&spec).unwrap(), &spec).unwrap();
        let hole_area: f64 = solid
            .faces()
            .iter()
            .enumerate()
            .filter(|(_, face)| {
                matches!(face.role, FaceRole::Wall { segment, .. } if segment.loop_ref == LoopRef::Hole(0))
            })
            .map(|(index, _)| {
                solid
                    .face_area(exedra::analytic::extrusion::FaceId(u32::try_from(index).unwrap()))
                    .unwrap()
            })
            .sum();
        let expected = core::f64::consts::TAU * spec.hole_radius * spec.thickness;
        assert!((hole_area - expected).abs() < 1e-9);
    }

    #[test]
    fn failures_name_their_cause_on_both_routes() {
        let escaping = PlateSpec {
            hole_x: 37.0,
            ..PlateSpec::default()
        };
        // The shared check names the touching segments; the constructive
        // route refuses the escaping cutout earlier, in its own terms.
        assert!(matches!(
            handwritten_profile(&escaping),
            Err(PlateError::Profile(ProfileError::Contact { .. }))
        ));
        assert_eq!(
            compiled_profile(&escaping),
            Err(PlateError::Constructive(
                "hole 0 bounding box escapes the outer loop".into()
            ))
        );
        let flat = PlateSpec {
            thickness: 0.0,
            ..PlateSpec::default()
        };
        assert_eq!(
            extrude(handwritten_profile(&flat).unwrap(), &flat),
            Err(PlateError::Extrusion(ExtrusionError::InvalidLength))
        );
    }
}
