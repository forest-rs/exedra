// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Opt-in verification of declared contact rectangles against compiled parts.
//!
//! This adapter checks surface coverage; it does not discover contacts, compile
//! recipes, detect solid interpenetration, or establish structural capacity.
//! Analytic [`crate::validate()`] remains independent of mesh evaluation.

use alloc::vec;
use alloc::vec::Vec;

use exedra_assembly::CompiledPart;
use exedra_math::{cross, dot, sub};

use crate::validate::contact_is_valid_witness;
use crate::{Construction, ContactMeaning, ContactPatch, Element, Vec3};

type Point2 = [f64; 2];
type Polygon = Vec<Point2>;

const MAX_FRAGMENTS: usize = 4096;
const MAX_CLIPS: usize = 1_000_000;

/// Why a compiled contact measurement could not be made.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContactGeometryError {
    /// The patch has no declared rectangular footprint.
    MissingFootprint,
    /// The analytic contact is invalid, omitted, or clearance-only.
    InvalidContact,
    /// The tolerance is nonpositive, nonfinite, or erases the rectangle.
    InvalidTolerance,
    /// A participant has no compiled triangles.
    MissingGeometry,
    /// A compiled triangle has an invalid index or nonfinite position.
    InvalidMesh,
    /// Surface subtraction exceeded its bounded fragment or clipping budget.
    BudgetExceeded,
}

impl core::fmt::Display for ContactGeometryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::MissingFootprint => "contact has no declared footprint",
            Self::InvalidContact => "contact is not a valid finite surface claim",
            Self::InvalidTolerance => "tolerance must be positive and smaller than half each side",
            Self::MissingGeometry => "a contact participant has no compiled triangles",
            Self::InvalidMesh => "compiled contact geometry contains an invalid triangle",
            Self::BudgetExceeded => "contact surface coverage exceeded its clipping budget",
        })
    }
}

impl core::error::Error for ContactGeometryError {}

/// Surface area still missing from each side of a declared contact rectangle.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ContactGeometryMeasurement {
    /// Rectangle area checked after insetting its boundary by the distance
    /// tolerance, in square meters.
    pub checked_area: f64,
    /// Checked area without an outward-facing carried surface, in square meters.
    pub carried_uncovered_area: f64,
    /// Checked area without an outward-facing carrier surface, in square meters.
    pub carrier_uncovered_area: f64,
}

impl ContactGeometryMeasurement {
    /// Whether both surfaces cover the checked rectangle, allowing only
    /// floating-point area roundoff (`256 * f64::EPSILON * checked_area`).
    #[must_use]
    pub fn is_covered(&self) -> bool {
        let roundoff = area_roundoff(self.checked_area);
        self.checked_area.is_finite()
            && self.checked_area > 0.0
            && (0.0..=roundoff).contains(&self.carried_uncovered_area)
            && (0.0..=roundoff).contains(&self.carrier_uncovered_area)
    }
}

/// Checks the whole declared footprint against both participants' compiled surfaces.
///
/// `carried` and `carrier` must be the current compiled parts for the named
/// elements, including their composed edits. The caller owns that association;
/// this function neither compiles nor caches geometry.
///
/// The rectangle is inset by `tolerance_meters` on all four edges to allow
/// tessellated boundaries to differ from analytic dimensions. Only triangles
/// whose vertices lie within that distance of the contact plane, facing out
/// toward the other participant, contribute coverage. Interior holes remain
/// uncovered, and overlapping or duplicate triangles cannot increase coverage.
/// Coordinates stay in each participant's local frame, including for reflected
/// placements. This is a planar bearing check, not a general collision test.
///
/// # Errors
///
/// Returns [`ContactGeometryError`] for invalid claims, tolerance, or geometry,
/// or when subtraction exceeds 4,096 fragments or 1,000,000 half-plane clips
/// per participant. An uncovered valid rectangle is a successful measurement
/// with [`ContactGeometryMeasurement::is_covered`] equal to `false`.
pub fn measure_contact_geometry(
    construction: &Construction,
    contact: &ContactPatch,
    carried: &CompiledPart,
    carrier: &CompiledPart,
    tolerance_meters: f64,
) -> Result<ContactGeometryMeasurement, ContactGeometryError> {
    let size = contact
        .footprint_meters()
        .ok_or(ContactGeometryError::MissingFootprint)?;
    if !contact_is_valid_witness(construction, contact)
        || contact.meaning == ContactMeaning::ClearanceOnly
    {
        return Err(ContactGeometryError::InvalidContact);
    }
    if !tolerance_meters.is_finite()
        || tolerance_meters <= 0.0
        || size.iter().any(|side| *side <= 2.0 * tolerance_meters)
    {
        return Err(ContactGeometryError::InvalidTolerance);
    }
    if carried.triangle_count() == 0 || carrier.triangle_count() == 0 {
        return Err(ContactGeometryError::MissingGeometry);
    }
    let half = size.map(|side| side * 0.5 - tolerance_meters);
    let checked_area = 4.0 * half[0] * half[1];
    if !checked_area.is_finite() || checked_area <= 0.0 {
        return Err(ContactGeometryError::InvalidContact);
    }
    let mut missing = [0.0; 2];
    for (index, (part, anchor, facing)) in [
        (carried, &contact.carried, -1.0),
        (carrier, &contact.carrier, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let element = construction
            .element(&anchor.element)
            .ok_or(ContactGeometryError::InvalidContact)?;
        missing[index] = uncovered_area(
            part,
            element,
            anchor.local,
            contact,
            facing,
            half,
            tolerance_meters,
        )?;
    }
    Ok(ContactGeometryMeasurement {
        checked_area,
        carried_uncovered_area: missing[0],
        carrier_uncovered_area: missing[1],
    })
}

fn uncovered_area(
    part: &CompiledPart,
    element: &Element,
    anchor: Vec3,
    contact: &ContactPatch,
    facing: f64,
    half: Point2,
    tolerance: f64,
) -> Result<f64, ContactGeometryError> {
    let normal = element.extent.local_direction(contact.normal);
    let tangents = contact
        .tangents
        .map(|tangent| element.extent.local_direction(tangent));
    let mut remaining = vec![vec![
        [-half[0], -half[1]],
        [half[0], -half[1]],
        [half[0], half[1]],
        [-half[0], half[1]],
    ]];
    let roundoff = area_roundoff(4.0 * half[0] * half[1]);
    let mut clips = 0;
    for body in &part.bodies {
        if !body.tri.indices.len().is_multiple_of(3) {
            return Err(ContactGeometryError::InvalidMesh);
        }
        for indices in body.tri.indices.chunks_exact(3) {
            let mut points = [[0.0; 3]; 3];
            for (point, index) in points.iter_mut().zip(indices) {
                *point = body
                    .tri
                    .positions
                    .get(*index as usize)
                    .ok_or(ContactGeometryError::InvalidMesh)?
                    .map(f64::from);
                if point.iter().any(|coordinate| !coordinate.is_finite()) {
                    return Err(ContactGeometryError::InvalidMesh);
                }
                *point = sub(*point, anchor);
            }
            if points
                .iter()
                .any(|point| dot(*point, normal).abs() > tolerance)
                || facing
                    * dot(
                        cross(sub(points[1], points[0]), sub(points[2], points[0])),
                        normal,
                    )
                    <= 0.0
            {
                continue;
            }
            let mut triangle = points.map(|point| tangents.map(|tangent| dot(point, tangent)));
            if side(triangle[0], triangle[1], triangle[2]) < 0.0 {
                triangle.swap(1, 2);
            }
            let mut next = Vec::new();
            for polygon in remaining {
                subtract_triangle(polygon, triangle, &mut next, &mut clips, roundoff)?;
            }
            remaining = next;
            if remaining.is_empty() {
                return Ok(0.0);
            }
        }
    }
    Ok(remaining.iter().map(|polygon| area(polygon)).sum())
}

fn subtract_triangle(
    mut inside: Polygon,
    triangle: [Point2; 3],
    remaining: &mut Vec<Polygon>,
    clips: &mut usize,
    roundoff: f64,
) -> Result<(), ContactGeometryError> {
    // Each triangle half-plane divides the still-inside polygon. Its outside
    // piece is disjoint from the pieces emitted by previous half-planes.
    for edge in 0..3 {
        *clips += 1;
        if *clips > MAX_CLIPS || remaining.len() >= MAX_FRAGMENTS {
            return Err(ContactGeometryError::BudgetExceeded);
        }
        let (positive, negative) = split(&inside, triangle[edge], triangle[(edge + 1) % 3]);
        if area(&negative) > roundoff {
            remaining.push(negative);
        }
        inside = positive;
        if area(&inside) <= roundoff {
            break;
        }
    }
    Ok(())
}

fn split(polygon: &[Point2], a: Point2, b: Point2) -> (Polygon, Polygon) {
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let Some(mut previous) = polygon.last().copied() else {
        return (positive, negative);
    };
    let mut previous_side = side(a, b, previous);
    for &point in polygon {
        let current_side = side(a, b, point);
        if (previous_side < 0.0 && current_side > 0.0)
            || (previous_side > 0.0 && current_side < 0.0)
        {
            let t = previous_side / (previous_side - current_side);
            let intersection = [
                previous[0] + t * (point[0] - previous[0]),
                previous[1] + t * (point[1] - previous[1]),
            ];
            positive.push(intersection);
            negative.push(intersection);
        }
        if current_side >= 0.0 {
            positive.push(point);
        }
        if current_side <= 0.0 {
            negative.push(point);
        }
        previous = point;
        previous_side = current_side;
    }
    (positive, negative)
}

fn side(a: Point2, b: Point2, point: Point2) -> f64 {
    (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])
}

fn area(polygon: &[Point2]) -> f64 {
    let Some(&origin) = polygon.first() else {
        return 0.0;
    };
    polygon[1..]
        .windows(2)
        .map(|edge| side(origin, edge[0], edge[1]))
        .sum::<f64>()
        .abs()
        * 0.5
}

fn area_roundoff(area: f64) -> f64 {
    256.0 * f64::EPSILON * area
}

#[cfg(test)]
mod tests;
