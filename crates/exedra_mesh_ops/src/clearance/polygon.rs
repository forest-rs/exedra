// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Filled polygon queries against an already checked planar material domain.

mod intersection;

use super::{
    BoundaryWitness, ClearanceDecision, PlanarPatch, contains, intersects, point_segment,
    valid_point,
};
use alloc::vec::Vec;
use exedra_triangulate::predicates::{Orientation, orient2d};

/// Limits for validating and measuring a simple filled polygon in patch units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolygonClearancePolicy {
    /// Maximum footprint vertices, without a repeated closing endpoint.
    pub max_vertices: u32,
    /// Maximum segment-pair and point/segment visits, including validation,
    /// boundary distances, interval splitting and containment. Zero refuses work.
    pub max_pair_checks: u64,
    /// Minimum footprint edge length and nonadjacent-edge separation. This
    /// validates the footprint; it does not expand the support or its holes.
    pub min_separation: f64,
}
impl Default for PolygonClearancePolicy {
    fn default() -> Self {
        Self {
            max_vertices: 4096,
            max_pair_checks: 16_000_000,
            min_separation: 1e-9,
        }
    }
}

/// Inability to validate or measure a polygon under the requested contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolygonClearanceError {
    /// Invalid policy or nonfinite/out-of-range coordinates.
    InvalidInput,
    /// Fewer than three vertices, a short edge or adjacent backtracking.
    InvalidFootprint,
    /// Nonadjacent footprint edges intersect or violate minimum separation.
    FootprintContact {
        /// First authored edge index.
        first: u32,
        /// Second authored edge index.
        second: u32,
    },
    /// Footprint vertices or geometric visits exceed an explicit limit.
    BudgetExceeded,
    /// Arithmetic cannot reliably resolve an intersection or interval interior.
    /// Crossing witnesses require a positional enclosure no wider than
    /// `64 * f64::EPSILON * coordinate_scale` per axis.
    NumericLimit,
}
impl core::fmt::Display for PolygonClearanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid polygon-clearance input or policy"),
            Self::InvalidFootprint => f.write_str("footprint needs a simple nondegenerate polygon"),
            Self::FootprintContact { first, second } => write!(
                f,
                "footprint edges {first} and {second} cross or touch within separation"
            ),
            Self::BudgetExceeded => f.write_str("polygon-clearance work budget exceeded"),
            Self::NumericLimit => {
                f.write_str("polygon-clearance arithmetic cannot resolve the geometry")
            }
        }
    }
}
impl core::error::Error for PolygonClearanceError {}

/// A point on an authored footprint edge, in patch-local XY coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootprintWitness {
    /// Edge `i` joins vertex `i` to `(i + 1) % len`.
    pub segment_index: u32,
    /// Measured location in patch units.
    pub point: [f64; 2],
}

/// Concrete evidence that some filled footprint lies outside support material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PolygonViolation<S = super::BoundarySource> {
    /// Two boundary interiors cross transversely.
    BoundaryCrossing {
        /// Crossing footprint edge and location.
        footprint: FootprintWitness,
        /// Crossing material boundary and source.
        boundary: BoundaryWitness<S>,
    },
    /// An open interval of the footprint boundary is outside material.
    Outside {
        /// Witness in that interval, rather than just a corner or bounding box.
        footprint: FootprintWitness,
    },
    /// Footprint interior covers material excluded by a support hole.
    CoveredHole {
        /// Hole boundary adjacent to the covered void.
        boundary: BoundaryWitness<S>,
    },
}

/// Filled-polygon containment and nearest boundary separation.
///
/// Containment and distance are separate: enclosing a hole can give positive
/// boundary distance, and crossing a boundary gives zero distance despite a
/// violation. Distances are f64 measurements of the retained polygonal domain,
/// not analytic-surface bounds or a translation distance for repairing a fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolygonClearance<S = super::BoundarySource> {
    /// Minimum distance between footprint and material boundaries, nonnegative.
    pub boundary_distance: f64,
    /// Closest footprint location. Ties use support loop/edge then footprint order.
    pub nearest_footprint: FootprintWitness,
    /// Closest support location and original source evidence.
    pub nearest_boundary: BoundaryWitness<S>,
    /// `None` indicates filled-polygon containment, including boundary contact.
    pub violation: Option<PolygonViolation<S>>,
    /// Segment-pair and point/segment visits charged, including validation.
    pub pair_checks: u64,
}
impl<S> PolygonClearance<S> {
    /// Whether all filled footprint points belong to material or its boundary.
    #[must_use]
    pub fn is_contained(&self) -> bool {
        self.violation.is_none()
    }

    /// Classifies a nonnegative clearance requirement with a decision tolerance.
    /// Any containment violation is `Violated`, regardless of distance or
    /// tolerance. Contained boundary contact has zero clearance. The decision
    /// tolerance never expands the material domain or fills its holes.
    pub fn classify(
        &self,
        minimum: f64,
        tolerance: f64,
    ) -> Result<ClearanceDecision, PolygonClearanceError> {
        if !minimum.is_finite()
            || minimum < 0.0
            || !tolerance.is_finite()
            || tolerance < 0.0
            || !self.boundary_distance.is_finite()
            || self.boundary_distance < 0.0
        {
            return Err(PolygonClearanceError::InvalidInput);
        }
        if !self.is_contained() {
            return Ok(ClearanceDecision::Violated);
        }
        let margin = self.boundary_distance - minimum;
        Ok(if margin > tolerance {
            ClearanceDecision::Satisfied
        } else if margin < -tolerance {
            ClearanceDecision::Violated
        } else {
            ClearanceDecision::WithinTolerance
        })
    }
}

struct Work {
    used: u64,
    maximum: u64,
}
impl Work {
    fn spend(&mut self) -> Result<(), PolygonClearanceError> {
        if self.used == self.maximum {
            return Err(PolygonClearanceError::BudgetExceeded);
        }
        self.used += 1;
        Ok(())
    }
}
fn index(i: usize) -> u32 {
    u32::try_from(i).expect("checked polygon size fits u32")
}
fn on(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    (0..2).all(|i| p[i] >= a[i].min(b[i]) && p[i] <= a[i].max(b[i]))
        && orient2d(a, b, p) == Orientation::Collinear
}
fn transverse(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let sides = [
        orient2d(a, b, c),
        orient2d(a, b, d),
        orient2d(c, d, a),
        orient2d(c, d, b),
    ];
    sides.iter().all(|s| *s != Orientation::Collinear)
        && sides[0] != sides[1]
        && sides[2] != sides[3]
}
fn segment_pair(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> Result<(f64, [f64; 2], [f64; 2]), PolygonClearanceError> {
    if intersects(a, b, c, d) {
        for p in [a, b, c, d] {
            if on(p, a, b) && on(p, c, d) {
                return Ok((0.0, p, p));
            }
        }
        let p = intersection::witness(a, b, c, d)?;
        return Ok((0.0, p, p));
    }
    let (da, pa) = point_segment(a, c, d);
    let mut best = (da, a, pa);
    let (db, pb) = point_segment(b, c, d);
    let (dc, pc) = point_segment(c, a, b);
    let (dd, pd) = point_segment(d, a, b);
    for next in [(db, b, pb), (dc, pc, c), (dd, pd, d)] {
        if next.0 < best.0 {
            best = next;
        }
    }
    Ok(best)
}
fn validate(
    points: &[[f64; 2]],
    policy: &PolygonClearancePolicy,
    work: &mut Work,
) -> Result<bool, PolygonClearanceError> {
    use PolygonClearanceError as E;
    if points.len() > policy.max_vertices as usize {
        return Err(E::BudgetExceeded);
    }
    if !policy.min_separation.is_finite()
        || policy.min_separation <= 0.0
        || points.iter().any(|&p| !valid_point(p))
    {
        return Err(E::InvalidInput);
    }
    if points.len() < 3 {
        return Err(E::InvalidFootprint);
    }
    for i in 0..points.len() {
        let (a, b, c) = (
            points[i],
            points[(i + 1) % points.len()],
            points[(i + 2) % points.len()],
        );
        work.spend()?;
        if libm::hypot(b[0] - a[0], b[1] - a[1]) <= policy.min_separation
            || (orient2d(a, b, c) == Orientation::Collinear
                && (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]) <= 0.0)
        {
            return Err(E::InvalidFootprint);
        }
        for j in i + 1..points.len() {
            if j == i + 1 || (i == 0 && j + 1 == points.len()) {
                continue;
            }
            work.spend()?;
            if segment_pair(a, b, points[j], points[(j + 1) % points.len()])?.0
                <= policy.min_separation
            {
                return Err(E::FootprintContact {
                    first: index(i),
                    second: index(j),
                });
            }
        }
    }
    // A simple polygon's lexicographically extreme corner determines winding
    // using an exact-sign predicate, without a cancellation-prone area sum.
    // Numeric ordering must treat signed zeros alike; otherwise a collinear
    // station can incorrectly precede the actual extreme corner.
    let i = (0..points.len())
        .min_by(|&a, &b| {
            points[a][0]
                .partial_cmp(&points[b][0])
                .expect("validated finite coordinates")
                .then(
                    points[a][1]
                        .partial_cmp(&points[b][1])
                        .expect("validated finite coordinates"),
                )
        })
        .expect("nonempty polygon");
    Ok(orient2d(
        points[(i + points.len() - 1) % points.len()],
        points[i],
        points[(i + 1) % points.len()],
    ) == Orientation::Ccw)
}
fn split(
    a: [f64; 2],
    b: [f64; 2],
    vertices: impl Iterator<Item = [f64; 2]>,
    work: &mut Work,
) -> Result<Vec<[f64; 2]>, PolygonClearanceError> {
    let mut points = alloc::vec![a, b];
    for p in vertices {
        work.spend()?;
        if on(p, a, b) {
            points.push(p);
        }
    }
    let axis = usize::from((b[1] - a[1]).abs() > (b[0] - a[0]).abs());
    points.sort_by(|a, b| a[axis].total_cmp(&b[axis]));
    points.dedup();
    Ok(points)
}
#[derive(Clone, Copy)]
enum Location {
    Inside,
    Outside,
    Boundary { loop_index: usize, segment: usize },
}
fn midpoint(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] * 0.5 + b[0] * 0.5, a[1] * 0.5 + b[1] * 0.5]
}
fn locate_interval<'a>(
    a: [f64; 2],
    b: [f64; 2],
    rings: impl Iterator<Item = &'a [[f64; 2]]>,
    work: &mut Work,
) -> Result<Location, PolygonClearanceError> {
    let p = midpoint(a, b);
    let mut inside = false;
    let mut distance = f64::INFINITY;
    let mut scale = p[0]
        .abs()
        .max(p[1].abs())
        .max(a[0].abs())
        .max(a[1].abs())
        .max(b[0].abs())
        .max(b[1].abs());
    for (loop_index, ring) in rings.enumerate() {
        for (segment, &c) in ring.iter().enumerate() {
            work.spend()?;
            let d = ring[(segment + 1) % ring.len()];
            // Exact shared intervals need no rounded midpoint-side decision.
            if on(a, c, d) && on(b, c, d) {
                return Ok(Location::Boundary {
                    loop_index,
                    segment,
                });
            }
            distance = distance.min(point_segment(p, c, d).0);
            scale = scale.max(c[0].abs()).max(c[1].abs());
        }
        for _ in ring {
            work.spend()?;
        }
        inside ^= contains(ring, p);
    }
    if p == a || p == b || distance <= scale * (64.0 * f64::EPSILON) {
        return Err(PolygonClearanceError::NumericLimit);
    }
    Ok(if inside {
        Location::Inside
    } else {
        Location::Outside
    })
}
impl<S: Copy> PlanarPatch<S> {
    /// Measures a simple filled polygon, in patch-local XY, against this support.
    /// Either winding is accepted; omit a repeated closing endpoint. Footprint
    /// holes are not part of this contract. Concave footprints and supports,
    /// support holes, collinear edges and exact boundary contact are supported.
    ///
    /// Validates the footprint, compares every boundary segment pair, and checks
    /// boundary intervals and enclosed holes. Corners alone are insufficient.
    ///
    /// ```
    /// use exedra_mesh_ops::clearance::{PlanarPatch, PolygonClearancePolicy};
    /// # fn inspect(patch: &PlanarPatch) -> Result<(), Box<dyn std::error::Error>> {
    /// let plate = [[1.0, 1.0], [3.0, 1.0], [3.0, 2.0], [1.0, 2.0]];
    /// let result = patch.polygon_clearance(&plate, &PolygonClearancePolicy::default())?;
    /// let decision = result.classify(0.2, 0.001)?;
    /// # let _ = decision;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// No mesh is tessellated again. Work and temporary interval storage are
    /// bounded by the input sizes and policy; failures return no partial answer.
    pub fn polygon_clearance(
        &self,
        footprint: &[[f64; 2]],
        policy: &PolygonClearancePolicy,
    ) -> Result<PolygonClearance<S>, PolygonClearanceError> {
        let mut work = Work {
            used: 0,
            maximum: policy.max_pair_checks,
        };
        let ccw = validate(footprint, policy, &mut work)?;
        let mut best = None;
        let mut violation = None;
        for (l, ring) in self.loops.iter().enumerate() {
            for (j, &c) in ring.points.iter().enumerate() {
                let d = ring.points[(j + 1) % ring.points.len()];
                for (i, &a) in footprint.iter().enumerate() {
                    work.spend()?;
                    let b = footprint[(i + 1) % footprint.len()];
                    let (distance, p, q) = segment_pair(a, b, c, d)?;
                    let fw = FootprintWitness {
                        segment_index: index(i),
                        point: p,
                    };
                    let bw = BoundaryWitness {
                        loop_index: index(l),
                        segment_index: index(j),
                        point: q,
                        source: ring.sources[j],
                    };
                    if best
                        .as_ref()
                        .is_none_or(|(previous, _, _)| distance < *previous)
                    {
                        best = Some((distance, fw, bw));
                    }
                    if violation.is_none() && transverse(a, b, c, d) {
                        violation = Some(PolygonViolation::BoundaryCrossing {
                            footprint: fw,
                            boundary: bw,
                        });
                    }
                }
            }
        }
        if violation.is_none() {
            'edges: for (i, &a) in footprint.iter().enumerate() {
                let points = split(
                    a,
                    footprint[(i + 1) % footprint.len()],
                    self.loops.iter().flat_map(|l| l.points.iter().copied()),
                    &mut work,
                )?;
                for pair in points.windows(2) {
                    if matches!(
                        locate_interval(
                            pair[0],
                            pair[1],
                            self.loops.iter().map(|l| l.points.as_slice()),
                            &mut work
                        )?,
                        Location::Outside
                    ) {
                        violation = Some(PolygonViolation::Outside {
                            footprint: FootprintWitness {
                                segment_index: index(i),
                                point: midpoint(pair[0], pair[1]),
                            },
                        });
                        break 'edges;
                    }
                }
            }
        }
        if violation.is_none() {
            'holes: for (l, ring) in self.loops.iter().enumerate().skip(1) {
                for (j, &a) in ring.points.iter().enumerate() {
                    let b = ring.points[(j + 1) % ring.points.len()];
                    let points = split(a, b, footprint.iter().copied(), &mut work)?;
                    for pair in points.windows(2) {
                        let covered = match locate_interval(
                            pair[0],
                            pair[1],
                            core::iter::once(footprint),
                            &mut work,
                        )? {
                            Location::Inside => true,
                            Location::Outside => false,
                            Location::Boundary {
                                loop_index,
                                segment,
                            } => {
                                debug_assert_eq!(
                                    loop_index, 0,
                                    "the footprint has one boundary loop"
                                );
                                let (c, d) = (
                                    footprint[segment],
                                    footprint[(segment + 1) % footprint.len()],
                                );
                                let axis = usize::from((b[1] - a[1]).abs() > (b[0] - a[0]).abs());
                                // Hole material is on the left; reject footprint
                                // fill on its right even for coincident boundaries.
                                let same = (b[axis] > a[axis]) == (d[axis] > c[axis]);
                                same != ccw
                            }
                        };
                        if covered {
                            violation = Some(PolygonViolation::CoveredHole {
                                boundary: BoundaryWitness {
                                    loop_index: index(l),
                                    segment_index: index(j),
                                    point: midpoint(pair[0], pair[1]),
                                    source: ring.sources[j],
                                },
                            });
                            break 'holes;
                        }
                    }
                }
            }
        }
        let (boundary_distance, nearest_footprint, nearest_boundary) =
            best.ok_or(PolygonClearanceError::InvalidInput)?;
        Ok(PolygonClearance {
            boundary_distance,
            nearest_footprint,
            nearest_boundary,
            violation,
            pair_checks: work.used,
        })
    }
}

#[cfg(test)]
mod tests;
