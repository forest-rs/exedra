// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Planar mounting-footprint clearance against mesh-face boundaries.
//!
//! This measures the polygonal boundary stored in the evaluated mesh, projected
//! into a checked workplane. It does not recover an analytic curve, account for
//! a different tessellation's error, measure wall thickness, or certify a solid.
//! A circular footprint is contained exactly when its center is inside material
//! and its distance to every boundary exceeds its radius (subject to arithmetic
//! and the caller's decision tolerance). Holes are material exclusions.
//! Polygon queries additionally inspect full boundary intervals and excluded
//! holes enclosed by the footprint, returning separate containment evidence.

mod polygon;
pub use polygon::{
    FootprintWitness, PolygonClearance, PolygonClearanceError, PolygonClearancePolicy,
    PolygonViolation,
};

use crate::workplane::{Workplane, WorkplaneError};
use alloc::vec::Vec;
use exedra_math::Placement3;
use exedra_mesh::{FaceId, HalfEdgeId, Mesh};
use exedra_triangulate::predicates::{MAX_COORDINATE, Orientation, orient2d};

/// Finite work and separation limits for extracting a planar patch boundary.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundaryPolicy {
    /// Maximum selected face corners, including repeated interior vertices.
    pub max_corners: u32,
    /// Maximum tested nonadjacent edge pairs and containment edge visits.
    pub max_pair_checks: u64,
    /// Reject shorter edges or nonadjacent boundaries this close, in body units.
    /// Positive and finite. Adjacent collinear edges may continue but not reverse.
    pub min_separation: f64,
}
impl Default for BoundaryPolicy {
    fn default() -> Self {
        Self {
            max_corners: 65536,
            max_pair_checks: 16_000_000,
            min_separation: 1e-9,
        }
    }
}

/// Construction work performed before clearance queries become available.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundaryStats {
    /// Selected face corners visited, including interior corners.
    pub corners_examined: u32,
    /// Retained boundary segments.
    pub boundary_edges: u32,
    /// Nonadjacent segment pairs and containment edge visits tested.
    pub pair_checks: u64,
}

/// Provenance of a boundary segment, scoped to the source body snapshot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoundarySource {
    /// Face on the selected side of this boundary.
    pub face: FaceId,
    /// Selected-side half-edge in the original evaluated mesh.
    pub edge: HalfEdgeId,
    /// Face across the boundary, or `None` for an open mesh edge.
    pub adjacent_face: Option<FaceId>,
}

/// One immutable oriented boundary: material lies on its left.
#[derive(Clone, Debug, PartialEq)]
pub struct PatchLoop<S = BoundarySource> {
    points: Vec<[f64; 2]>,
    sources: Vec<S>,
}
impl<S> PatchLoop<S> {
    /// Cyclic workplane-XY vertices without a repeated closing point.
    #[must_use]
    pub fn points(&self) -> &[[f64; 2]] {
        &self.points
    }
    /// Source for each segment `points[i] -> points[(i+1) % len]`.
    #[must_use]
    pub fn sources(&self) -> &[S] {
        &self.sources
    }
}

/// Checked polygonal material domain on one connected planar face patch.
///
/// The outer loop is counter-clockwise; hole loops are clockwise. Construction
/// rejects crossings, contacts, reversed adjacent edges and invalid nesting.
/// This is an immutable geometric snapshot. Source mesh IDs are evidence for
/// that snapshot only, not persistent selections across edits. `S` carries source
/// evidence; the default is geometric mesh IDs. Use [`Self::try_map_sources`]
/// to bind application provenance without duplicating the boundary geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarPatch<S = BoundarySource> {
    frame: Placement3,
    loops: Vec<PatchLoop<S>>,
    stats: BoundaryStats,
    max_plane_deviation: f64,
}

/// A failed boundary extraction or circular-footprint query.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClearanceError {
    /// Invalid limits, nonfinite coordinates, negative radius, or invalid requirement.
    InvalidInput,
    /// The workplane does not match its source mesh revision.
    Workplane(WorkplaneError),
    /// The patch has no valid, simple material boundary with consistent nesting.
    InvalidBoundary,
    /// Two nonadjacent segments cross, touch, or violate minimum separation.
    BoundaryContact {
        /// First offending source segment.
        first: HalfEdgeId,
        /// Second offending source segment.
        second: HalfEdgeId,
    },
    /// The selected topology exceeds an explicit work budget.
    BudgetExceeded,
    /// Projected geometry or arithmetic cannot be represented reliably.
    NumericLimit,
}
impl core::fmt::Display for ClearanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("invalid planar-clearance inputs or policy"),
            Self::Workplane(error) => write!(f, "clearance workplane: {error}"),
            Self::InvalidBoundary => {
                f.write_str("patch boundary is empty, degenerate, or incorrectly nested")
            }
            Self::BoundaryContact { first, second } => write!(
                f,
                "patch boundary edges {first:?} and {second:?} cross or touch within tolerance"
            ),
            Self::BudgetExceeded => f.write_str("planar-clearance boundary work budget exceeded"),
            Self::NumericLimit => f.write_str("planar-clearance geometry exceeds numeric limits"),
        }
    }
}
impl core::error::Error for ClearanceError {}

/// Relationship to an authored minimum clearance using an explicit decision band.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClearanceDecision {
    /// Clearance exceeds the minimum by more than the decision tolerance.
    Satisfied,
    /// Clearance falls below the minimum by more than the decision tolerance.
    Violated,
    /// Clearance is within the decision tolerance of the minimum; not certified.
    WithinTolerance,
}

/// Nearest boundary location, in the patch's workplane coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BoundaryWitness<S = BoundarySource> {
    /// Loop index, with zero denoting the outer boundary.
    pub loop_index: u32,
    /// Segment index within that loop.
    pub segment_index: u32,
    /// Closest point on the segment in workplane XY.
    pub point: [f64; 2],
    /// Source evidence for the original boundary segment.
    pub source: S,
}

/// Circular footprint's signed containment margin against an evaluated patch.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CircleClearance<S = BoundarySource> {
    /// Authored center in patch-local XY coordinates.
    pub center: [f64; 2],
    /// Authored radius, in body units. Zero measures point clearance.
    pub radius: f64,
    /// Whether the center lies in the material interior (not on its boundary).
    pub center_inside: bool,
    /// Positive distance for an interior center, negative outside, zero on boundary.
    pub signed_center_distance: f64,
    /// `signed_center_distance - radius`. A negative margin is not a general
    /// minimum translation distance for repairing a footprint outside the patch.
    pub clearance: f64,
    /// Nearest boundary segment and point; ties use loop/segment traversal order.
    pub nearest: BoundaryWitness<S>,
    /// Boundary segments measured. Containment also traverses these segments;
    /// no triangulation is repeated.
    pub edges_tested: u32,
}
impl<S> CircleClearance<S> {
    /// Compares the measured margin with a nonnegative minimum and tolerance.
    /// The tolerance is a caller's decision band, not an analytic error bound.
    ///
    /// # Errors
    /// Rejects negative or nonfinite requirement/tolerance values.
    pub fn classify(
        &self,
        minimum: f64,
        tolerance: f64,
    ) -> Result<ClearanceDecision, ClearanceError> {
        if !self.clearance.is_finite()
            || !minimum.is_finite()
            || minimum < 0.0
            || !tolerance.is_finite()
            || tolerance < 0.0
        {
            return Err(ClearanceError::InvalidInput);
        }
        let margin = self.clearance - minimum;
        Ok(if margin > tolerance {
            ClearanceDecision::Satisfied
        } else if margin < -tolerance {
            ClearanceDecision::Violated
        } else {
            ClearanceDecision::WithinTolerance
        })
    }
}

impl<S> PlanarPatch<S> {
    /// Replaces source evidence without copying points or changing checked geometry.
    ///
    /// This lets a caller attach authored provenance to geometric boundary IDs.
    /// The closure runs in loop/segment order. Failure consumes this snapshot.
    pub fn try_map_sources<T, E>(
        self,
        mut map: impl FnMut(S) -> Result<T, E>,
    ) -> Result<PlanarPatch<T>, E> {
        let loops = self
            .loops
            .into_iter()
            .map(|ring| {
                let sources = ring
                    .sources
                    .into_iter()
                    .map(&mut map)
                    .collect::<Result<Vec<_>, E>>()?;
                Ok(PatchLoop {
                    points: ring.points,
                    sources,
                })
            })
            .collect::<Result<Vec<_>, E>>()?;
        Ok(PlanarPatch {
            frame: self.frame,
            loops,
            stats: self.stats,
            max_plane_deviation: self.max_plane_deviation,
        })
    }

    /// Measures material area, total boundary length, centroid, and XY bounds.
    ///
    /// Uses this patch's projected polygonal boundaries and local frame. Holes
    /// subtract area and add perimeter. Work is linear in boundary edges, with
    /// no allocation, topology traversal, or tessellation. The extraction policy
    /// already bounds the number of edges. Source evidence remains on this patch.
    ///
    /// # Errors
    /// Returns a numeric-limit error if the measurement is not representable.
    pub fn measure(
        &self,
    ) -> Result<crate::measure::PlanarMeasurements, crate::measure::MeasurementError> {
        crate::measure::measure(self.loops.iter().enumerate().map(|(i, l)| {
            crate::measure::PlanarBoundary {
                points: l.points(),
                is_hole: i != 0,
            }
        }))
    }

    /// Workplane-to-body placement for interpreting measurements and witnesses.
    #[must_use]
    pub fn frame(&self) -> Placement3 {
        self.frame
    }
    /// Outer boundary first, followed by hole boundaries in deterministic order.
    #[must_use]
    pub fn loops(&self) -> &[PatchLoop<S>] {
        &self.loops
    }
    /// Boundary construction work, separate from subsequent query work.
    #[must_use]
    pub fn stats(&self) -> BoundaryStats {
        self.stats
    }
    /// Measured corner deviation from the workplane before projection.
    #[must_use]
    pub fn max_plane_deviation(&self) -> f64 {
        self.max_plane_deviation
    }
}

impl PlanarPatch {
    /// Extracts a boundary from a checked workplane and its original mesh.
    /// Callers must pair the workplane with that logical mesh: revision counters
    /// alone cannot distinguish unrelated meshes.
    ///
    /// # Errors
    /// Rejects a stale workplane, invalid boundaries, numeric limits and budgets.
    pub fn from_workplane(
        mesh: &Mesh,
        workplane: &Workplane,
        policy: &BoundaryPolicy,
    ) -> Result<Self, ClearanceError> {
        use ClearanceError as Error;
        if policy.max_corners == 0
            || policy.max_pair_checks == 0
            || !policy.min_separation.is_finite()
            || policy.min_separation <= 0.0
        {
            return Err(Error::InvalidInput);
        }
        workplane.check(mesh).map_err(Error::Workplane)?;
        let mut stats = BoundaryStats::default();
        for &face in workplane.faces() {
            for _ in mesh.face_loop(face) {
                if stats.corners_examined == policy.max_corners {
                    return Err(Error::BudgetExceeded);
                }
                stats.corners_examined += 1;
            }
        }
        let boundaries = mesh
            .selected_face_boundary_loops(workplane.faces())
            .map_err(|_| Error::InvalidBoundary)?;
        let mut loops = Vec::new();
        for boundary in boundaries {
            let mut points = Vec::new();
            let mut sources = Vec::new();
            for edge in boundary {
                let point = mesh
                    .from_vertex(edge)
                    .and_then(|v| mesh.vertex_position(v))
                    .ok_or(Error::InvalidBoundary)?;
                let local = workplane.to_local(point.map(f64::from));
                let xy = [local[0], local[1]];
                if !valid_point(xy) {
                    return Err(Error::NumericLimit);
                }
                let face = mesh.face(edge).ok_or(Error::InvalidBoundary)?;
                let adjacent_face = mesh
                    .twin(edge)
                    .and_then(|e| mesh.face(e))
                    .filter(|&face| face != FaceId::OUTSIDE);
                points.push(xy);
                sources.push(BoundarySource {
                    face,
                    edge,
                    adjacent_face,
                });
                stats.boundary_edges += 1; // Bounded by checked selected corners.
            }
            loops.push(PatchLoop { points, sources });
        }
        validate_loops(&mut loops, policy, &mut stats)?;
        Ok(Self {
            frame: workplane.frame(),
            loops,
            stats,
            max_plane_deviation: workplane.max_plane_deviation(),
        })
    }
}

impl<S: Copy> PlanarPatch<S> {
    /// Measures a circular footprint against every material boundary, including
    /// holes. Coordinates and radius use the patch's orthonormal local frame.
    /// Query cost is linear in the retained boundary edges and requires no
    /// tessellation, topology traversal or allocation.
    ///
    /// # Errors
    /// Rejects nonfinite/out-of-range coordinates or negative radius.
    pub fn circle_clearance(
        &self,
        center: [f64; 2],
        radius: f64,
    ) -> Result<CircleClearance<S>, ClearanceError> {
        if !valid_point(center) || !radius.is_finite() || !(0.0..=MAX_COORDINATE).contains(&radius)
        {
            return Err(ClearanceError::InvalidInput);
        }
        let mut best: Option<(f64, BoundaryWitness<S>)> = None;
        let mut inside = false;
        for (loop_index, ring) in self.loops.iter().enumerate() {
            inside ^= contains(&ring.points, center);
            for (segment_index, &a) in ring.points.iter().enumerate() {
                let b = ring.points[(segment_index + 1) % ring.points.len()];
                let (distance, point) = point_segment(center, a, b);
                if best.is_none_or(|(previous, _)| distance < previous) {
                    best = Some((
                        distance,
                        BoundaryWitness {
                            loop_index: u32::try_from(loop_index)
                                .expect("checked boundary count fits u32"),
                            segment_index: u32::try_from(segment_index)
                                .expect("checked boundary count fits u32"),
                            point,
                            source: ring.sources[segment_index],
                        },
                    ));
                }
            }
        }
        let (distance, nearest) = best.ok_or(ClearanceError::InvalidBoundary)?;
        let signed_center_distance = if inside { distance } else { -distance };
        Ok(CircleClearance {
            center,
            radius,
            center_inside: inside && distance > 0.0,
            signed_center_distance,
            clearance: signed_center_distance - radius,
            nearest,
            edges_tested: self.stats.boundary_edges,
        })
    }
}

fn valid_point(point: [f64; 2]) -> bool {
    point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}
fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> (f64, [f64; 2]) {
    if (0..2).all(|i| p[i] >= a[i].min(b[i]) && p[i] <= a[i].max(b[i]))
        && orient2d(a, b, p) == Orientation::Collinear
    {
        return (0.0, p);
    }
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let length = libm::hypot(ab[0], ab[1]);
    let unit = [ab[0] / length, ab[1] / length];
    let t = (ap[0] * unit[0] + ap[1] * unit[1]).clamp(0.0, length);
    let point = if t == 0.0 {
        a
    } else if t == length {
        b
    } else {
        [a[0] + t * unit[0], a[1] + t * unit[1]]
    };
    (libm::hypot(p[0] - point[0], p[1] - point[1]), point)
}
fn contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut winding = 0_i64;
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        if a[1] <= p[1] && b[1] > p[1] && orient2d(a, b, p) == Orientation::Ccw {
            winding += 1;
        }
        if a[1] > p[1] && b[1] <= p[1] && orient2d(a, b, p) == Orientation::Cw {
            winding -= 1;
        }
    }
    winding != 0
}
fn area(ring: &[[f64; 2]]) -> f64 {
    let origin = ring[0];
    (1..ring.len() - 1)
        .map(|i| {
            let a = [ring[i][0] - origin[0], ring[i][1] - origin[1]];
            let b = [ring[i + 1][0] - origin[0], ring[i + 1][1] - origin[1]];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        * 0.5
}
fn intersects(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    use Orientation::Collinear;
    let (side_c, side_d, side_a, side_b) = (
        orient2d(a, b, c),
        orient2d(a, b, d),
        orient2d(c, d, a),
        orient2d(c, d, b),
    );
    let on = |p: [f64; 2], x: [f64; 2], y: [f64; 2]| {
        (0..2).all(|i| p[i] >= x[i].min(y[i]) && p[i] <= x[i].max(y[i]))
    };
    (side_c != Collinear
        && side_d != Collinear
        && side_c != side_d
        && side_a != Collinear
        && side_b != Collinear
        && side_a != side_b)
        || (side_c == Collinear && on(c, a, b))
        || (side_d == Collinear && on(d, a, b))
        || (side_a == Collinear && on(a, c, d))
        || (side_b == Collinear && on(b, c, d))
}
fn spend(
    stats: &mut BoundaryStats,
    policy: &BoundaryPolicy,
    count: u64,
) -> Result<(), ClearanceError> {
    stats.pair_checks = stats.pair_checks.saturating_add(count);
    if stats.pair_checks > policy.max_pair_checks {
        Err(ClearanceError::BudgetExceeded)
    } else {
        Ok(())
    }
}
fn validate_loops(
    loops: &mut [PatchLoop],
    policy: &BoundaryPolicy,
    stats: &mut BoundaryStats,
) -> Result<(), ClearanceError> {
    use ClearanceError as Error;
    let mut outer = None;
    for (index, ring) in loops.iter().enumerate() {
        if ring.points.len() < 3 {
            return Err(Error::InvalidBoundary);
        }
        let signed = area(&ring.points);
        if !signed.is_finite() || signed == 0.0 {
            return Err(Error::InvalidBoundary);
        }
        if signed > 0.0 && outer.replace(index).is_some() {
            return Err(Error::InvalidBoundary);
        }
        for (i, &a) in ring.points.iter().enumerate() {
            let b = ring.points[(i + 1) % ring.points.len()];
            let c = ring.points[(i + 2) % ring.points.len()];
            if libm::hypot(b[0] - a[0], b[1] - a[1]) <= policy.min_separation
                || (orient2d(a, b, c) == Orientation::Collinear
                    && (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]) <= 0.0)
            {
                return Err(Error::InvalidBoundary);
            }
        }
    }
    loops.swap(0, outer.ok_or(Error::InvalidBoundary)?);
    for i in 0..loops.len() {
        let first = &loops[i];
        for (j, second) in loops.iter().enumerate().take(i + 1) {
            for a in 0..first.points.len() {
                let an = (a + 1) % first.points.len();
                for b in 0..second.points.len() {
                    let bn = (b + 1) % second.points.len();
                    if i == j && (b <= a || an == b || bn == a) {
                        continue;
                    }
                    spend(stats, policy, 1)?;
                    let (p, q, r, s) = (
                        first.points[a],
                        first.points[an],
                        second.points[b],
                        second.points[bn],
                    );
                    if intersects(p, q, r, s)
                        || [
                            point_segment(p, r, s).0,
                            point_segment(q, r, s).0,
                            point_segment(r, p, q).0,
                            point_segment(s, p, q).0,
                        ]
                        .into_iter()
                        .any(|d| d <= policy.min_separation)
                    {
                        return Err(Error::BoundaryContact {
                            first: first.sources[a].edge,
                            second: second.sources[b].edge,
                        });
                    }
                }
            }
        }
        if i > 0 {
            spend(stats, policy, loops[0].points.len() as u64)?;
            if !contains(&loops[0].points, first.points[0]) {
                return Err(Error::InvalidBoundary);
            }
            for (j, other) in loops.iter().enumerate().skip(1) {
                if i == j {
                    continue;
                }
                spend(stats, policy, other.points.len() as u64)?;
                if contains(&other.points, first.points[0]) {
                    return Err(Error::InvalidBoundary);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
