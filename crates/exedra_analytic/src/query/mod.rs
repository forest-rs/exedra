// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bounded point classification on retained planar/cylindrical boundaries.
//!
//! Prepare once with [`SolidQuery::prepare`], then query without allocation.
//! Native surfaces, bounded edges and trims are authoritative. No mesh, feature
//! history or caller metadata participates. The input must already be an
//! embedded, outward-oriented regular solid (including correctly oriented cavity
//! shells). Local boundary checks do not prove that premise. Inconsistent signed
//! ray winding is refused, but a successful query is not a global validity proof.
//! Arithmetic allowances are f64 evidence, not interval-arithmetic certificates.

#[cfg(test)]
mod tests;
mod trim;

use crate::body::{Body, CylindricalBand, EdgeId, FaceId, LoopId, ValidationPolicy};
use crate::geometry::Surface;
use crate::validation::{
    BoundaryShell, BoundaryValidationError, BoundaryValidationPolicy, BoundaryValidationReport,
    TrimValidation,
};
use alloc::vec::Vec;
use exedra_math::{Placement3, add, cross, dot, scale, sub};
use trim::{Curve2, Region};

/// Accuracy and resource bounds for preparation and repeated point queries.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SolidQueryPolicy {
    /// Model-unit distance to the actual trimmed boundary reported as boundary.
    /// The tolerance band itself has a numeric uncertainty fringe: an unresolved
    /// point at its threshold is refused rather than assigned a side.
    pub boundary_tolerance: f64,
    /// Bounded local topology, geometric agreement and trim inspection.
    pub validation: BoundaryValidationPolicy,
    /// Maximum edge, support, trim, ray-intersection and subdivision tests per point.
    pub max_query_work: usize,
    /// Maximum deterministic ray directions tried, in `1..=8`.
    pub max_ray_attempts: usize,
}
impl Default for SolidQueryPolicy {
    fn default() -> Self {
        Self {
            boundary_tolerance: 1e-7,
            validation: BoundaryValidationPolicy::default(),
            max_query_work: 100_000,
            max_ray_attempts: 8,
        }
    }
}
/// Membership relative to the caller's boundary tolerance.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointLocation {
    /// Material, outside the boundary tolerance band.
    Inside,
    /// Void or exterior, outside the boundary tolerance band.
    Outside,
    /// A witness on the actual trimmed boundary lies within the tolerance.
    Boundary,
}
/// Retained entity supplying a boundary witness; IDs belong to the borrowed body.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoundaryWitness {
    /// A point on a native bounded edge, including its endpoints.
    Edge(EdgeId),
    /// A point in a native trimmed supporting face.
    Face(FaceId),
}
/// Actual per-point work. Every test contributes one unit to [`Self::total`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryWork {
    /// Bounded edge distance tests.
    pub edge_tests: usize,
    /// Closest-support boundary tests.
    pub support_tests: usize,
    /// Continuous 2D trim predicates, including deterministic chart-ray retries.
    pub trim_tests: usize,
    /// Analytic ray/support intersection tests, including misses and retries.
    pub intersection_tests: usize,
    /// Interval nodes used to resolve elliptic edge distance thresholds.
    pub subdivisions: usize,
    /// 3D ray directions started; not an additional work unit.
    pub ray_attempts: usize,
}
impl QueryWork {
    /// Total admitted primitive tests; never exceeds `max_query_work`.
    #[must_use]
    pub const fn total(self) -> usize {
        self.edge_tests
            + self.support_tests
            + self.trim_tests
            + self.intersection_tests
            + self.subdivisions
    }
}
/// Membership, traceable boundary evidence and the actual work performed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PointQuery {
    /// Material/void/boundary classification.
    pub location: PointLocation,
    /// Present exactly for a boundary result; no nearest-entity claim is made.
    pub boundary: Option<BoundaryWitness>,
    /// Actual per-point work.
    pub work: QueryWork,
}
/// Preparation evidence and persistent allocation payload, excluding the body.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PreparationReport {
    /// Prepared native faces.
    pub faces: usize,
    /// Distinct used native edges; detached historical entries are ignored.
    pub edges: usize,
    /// Owned 2D line/conic trim records.
    pub trim_curves: usize,
    /// Sum of retained vector capacities times element sizes. Excludes allocator
    /// headers and temporary boundary-validation/preparation scratch.
    pub owned_capacity_bytes: usize,
    /// Local edge occurrences checked by boundary validation.
    pub validated_coedges: usize,
    /// Conservative arrangement pair-work bound admitted by local validation.
    /// This is a bound, not a measured number of executed pair predicates.
    pub validation_pair_test_bound: usize,
    /// Largest local 3D support/correspondence disagreement bound.
    pub max_geometry_gap: f64,
}
/// Why a geometric side could not be separated from query uncertainty.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Ambiguity {
    /// Actual boundary distance could not be separated from the tolerance threshold.
    BoundaryTolerance,
    /// All permitted rays met tangency, a trim edge or an unresolved root.
    RayDegeneracy,
}
/// An explicit refusal; no result silently substitutes a mesh or convex hull.
#[derive(Clone, Debug, PartialEq)]
pub enum QueryError {
    /// Invalid accuracy/work policy or boundary tolerance not greater than the
    /// requested geometry agreement gap.
    InvalidPolicy,
    /// A query coordinate is not finite.
    NonFinitePoint,
    /// Bounded local boundary checks failed. They are not a global solid proof.
    BoundaryValidation(BoundaryValidationError),
    /// This support family is outside the planar/cylindrical subset.
    UnsupportedSurface {
        /// Native face.
        face: FaceId,
    },
    /// Native trim topology/chart family is outside this query's subset.
    UnsupportedTrim {
        /// Native face.
        face: FaceId,
        /// Offending loop when available.
        loop_id: Option<LoopId>,
    },
    /// Finite arithmetic cannot separate geometry at this accuracy.
    NumericLimit {
        /// Native face when localized.
        face: Option<FaceId>,
        /// Conservative arithmetic allowance in model units.
        allowance: f64,
        /// Completed query work; zero during preparation.
        work: QueryWork,
    },
    /// The next primitive test would exceed the point's work limit.
    WorkLimit {
        /// Completed work.
        work: QueryWork,
        /// Maximum permitted primitive tests.
        maximum: usize,
    },
    /// A side cannot be assigned without crossing an uncertainty threshold.
    Ambiguous {
        /// Limiting condition.
        reason: Ambiguity,
        /// Completed query work.
        work: QueryWork,
    },
    /// Signed crossings do not describe one outward material region. This can
    /// expose reversed shells or overlapping material, but is no global diagnosis.
    InconsistentWinding {
        /// Observed signed crossing sum.
        winding: i32,
        /// Completed query work.
        work: QueryWork,
    },
}
impl core::fmt::Display for QueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "native solid point query: {self:?}")
    }
}
impl core::error::Error for QueryError {}

#[derive(Debug)]
enum Patch {
    Planar(Vec<Curve2>),
    Polygon(Vec<Curve2>),
    Band(CylindricalBand),
}
#[derive(Debug)]
struct PreparedFace {
    id: FaceId,
    patch: Patch,
    inverse: LocalFrame,
}

// Immutable inverse coefficients retain the same subtraction/dot/division
// order as geometry::frame_coordinates, but do not rebuild cofactors per probe.
#[derive(Debug)]
struct LocalFrame {
    origin: [f64; 3],
    cofactors: [[f64; 3]; 3],
    determinant: f64,
}
impl LocalFrame {
    fn new(frame: Placement3) -> Option<Self> {
        let axis = |i| frame.rows.map(|r| r[i]);
        let cofactors = [
            cross(axis(1), axis(2)),
            cross(axis(2), axis(0)),
            cross(axis(0), axis(1)),
        ];
        let determinant = dot(axis(0), cofactors[0]);
        (determinant.is_finite()
            && determinant != 0.0
            && cofactors.iter().flatten().all(|v| v.is_finite()))
        .then_some(Self {
            origin: axis(3),
            cofactors,
            determinant,
        })
    }
    fn point(&self, point: [f64; 3]) -> Option<[f64; 3]> {
        self.vector(sub(point, self.origin))
    }
    fn vector(&self, vector: [f64; 3]) -> Option<[f64; 3]> {
        let result = self.cofactors.map(|v| dot(vector, v) / self.determinant);
        result.iter().all(|v| v.is_finite()).then_some(result)
    }
}
#[derive(Debug)]
enum EdgeQuery {
    Line {
        origin: [f64; 3],
        axis: [f64; 3],
        length: f64,
    },
    Conic {
        inverse: LocalFrame,
        axes: [f64; 2],
        singular: f64,
        speed: f64,
    },
}
#[derive(Debug)]
struct PreparedEdge {
    id: EdgeId,
    geometry: EdgeQuery,
}
impl PreparedEdge {
    fn new<F, E>(body: &Body<F, E>, id: EdgeId) -> Option<Self> {
        let edge = &body.edges()[id.index()];
        let (curve, _) = edge.regular()?;
        let geometry = if let Some((frame, axes)) = curve.harmonic() {
            let cols = [0, 1, 2].map(|i| frame.rows.map(|r| r[i]));
            let singular = libm::sqrt(
                (0..3)
                    .map(|i| {
                        dot(cols[i], cols[i])
                            - (0..3)
                                .filter(|j| *j != i)
                                .map(|j| dot(cols[i], cols[j]).abs())
                                .sum::<f64>()
                    })
                    .fold(f64::INFINITY, f64::min)
                    .max(0.0),
            );
            let speed = norm(scale(cols[0], axes[0])) + norm(scale(cols[1], axes[1]));
            if !singular.is_finite() || !speed.is_finite() {
                return None;
            }
            EdgeQuery::Conic {
                inverse: LocalFrame::new(frame)?,
                axes,
                singular,
                speed,
            }
        } else {
            let origin = body.vertices()[edge.start.index()];
            let d = sub(body.vertices()[edge.end.index()], origin);
            let length = norm(d);
            if length == 0.0 || !length.is_finite() {
                return None;
            }
            EdgeQuery::Line {
                origin,
                axis: scale(d, 1.0 / length),
                length,
            }
        };
        Some(Self { id, geometry })
    }
}
/// Immutable query preparation borrowing native geometry, independently of roles.
///
/// Preparation owns bounded trim/index buffers. [`Self::query_point`] allocates
/// nothing and reports actual primitive work. The body's borrow prevents edits
/// invalidating this preparation; rebuilding after an edit is explicit.
#[derive(Debug)]
pub struct SolidQuery<'a, F = (), E = ()> {
    body: &'a Body<F, E>,
    policy: SolidQueryPolicy,
    faces: Vec<PreparedFace>,
    edges: Vec<PreparedEdge>,
    magnitude: f64,
    report: PreparationReport,
    validation: BoundaryValidationReport,
}
struct Meter {
    work: QueryWork,
    maximum: usize,
}
impl Meter {
    fn admit(&self) -> Result<(), QueryError> {
        if self.work.total() == self.maximum {
            return Err(QueryError::WorkLimit {
                work: self.work,
                maximum: self.maximum,
            });
        }
        Ok(())
    }
    fn edge(&mut self) -> Result<(), QueryError> {
        self.admit()?;
        self.work.edge_tests += 1;
        Ok(())
    }
    fn support(&mut self) -> Result<(), QueryError> {
        self.admit()?;
        self.work.support_tests += 1;
        Ok(())
    }
    fn trim(&mut self) -> Result<(), QueryError> {
        self.admit()?;
        self.work.trim_tests += 1;
        Ok(())
    }
    fn intersection(&mut self) -> Result<(), QueryError> {
        self.admit()?;
        self.work.intersection_tests += 1;
        Ok(())
    }
    fn subdivision(&mut self) -> Result<(), QueryError> {
        self.admit()?;
        self.work.subdivisions += 1;
        Ok(())
    }
    fn numeric(&self, face: Option<FaceId>, allowance: f64) -> QueryError {
        QueryError::NumericLimit {
            face,
            allowance,
            work: self.work,
        }
    }
    fn ambiguous(&self, reason: Ambiguity) -> QueryError {
        QueryError::Ambiguous {
            reason,
            work: self.work,
        }
    }
}
fn norm(p: [f64; 3]) -> f64 {
    libm::hypot(libm::hypot(p[0], p[1]), p[2])
}
impl<'a, F, E> SolidQuery<'a, F, E> {
    /// Checks local boundary invariants and prepares continuous face predicates.
    ///
    /// Planes accept bounded lines/circles/ellipses. Cylinders accept explicit
    /// polygon chart fragments or checked full-turn harmonic graph bands. A
    /// cylinder without a supported chart is refused, even if its rectangular
    /// parameter domain resembles the intended face. Caller roles are ignored.
    ///
    /// # Errors
    /// Refuses invalid policy, local topology/support violations, unsupported
    /// geometry/charts or preparation work exceeding `policy.validation`.
    pub fn prepare(body: &'a Body<F, E>, policy: &SolidQueryPolicy) -> Result<Self, QueryError> {
        if !policy.boundary_tolerance.is_finite()
            || policy.boundary_tolerance <= policy.validation.max_gap
            || policy.max_query_work == 0
            || !(1..=8).contains(&policy.max_ray_attempts)
        {
            return Err(QueryError::InvalidPolicy);
        }
        let validation = body
            .validate_boundary(&policy.validation)
            .map_err(QueryError::BoundaryValidation)?;
        let mut faces = Vec::with_capacity(body.faces().len());
        let mut used = alloc::vec![false; body.edges().len()];
        let mut magnitude = 0.0_f64;
        for (i, face) in body.faces().iter().enumerate() {
            let id = FaceId(crate::usize_to_u32(i));
            for loop_id in &face.loops {
                for co in &body.loops()[loop_id.index()].coedges {
                    used[co.edge.index()] = true;
                    let edge = &body.edges()[co.edge.index()];
                    for vertex in [edge.start, edge.end] {
                        magnitude = body.vertices()[vertex.index()]
                            .iter()
                            .map(|v| v.abs())
                            .fold(magnitude, f64::max);
                    }
                }
            }
            let patch = match &face.surface {
                Surface::Plane(_) => {
                    let mut curves = Vec::with_capacity(
                        face.loops
                            .iter()
                            .map(|l| body.loops()[l.index()].coedges.len())
                            .sum(),
                    );
                    for &loop_id in &face.loops {
                        for &co in &body.loops()[loop_id.index()].coedges {
                            curves.push(Curve2::from_edge(body, id, co).ok_or(
                                QueryError::UnsupportedTrim {
                                    face: id,
                                    loop_id: Some(loop_id),
                                },
                            )?);
                        }
                    }
                    Patch::Planar(curves)
                }
                Surface::Cylinder(cylinder) => {
                    magnitude = magnitude.max(cylinder.radius());
                    if let Ok(band) = body.cylindrical_band(
                        id,
                        &ValidationPolicy {
                            max_gap: policy.validation.max_gap,
                        },
                    ) {
                        Patch::Band(band)
                    } else {
                        let mut curves = Vec::new();
                        for &loop_id in &face.loops {
                            for (index, co) in
                                body.loops()[loop_id.index()].coedges.iter().enumerate()
                            {
                                let Some(crate::body::TrimCurve::Line { start, end }) =
                                    body.trim_curve(loop_id, index)
                                else {
                                    return Err(QueryError::UnsupportedTrim {
                                        face: id,
                                        loop_id: Some(loop_id),
                                    });
                                };
                                let metric = |uv: [f64; 2]| [uv[0] * cylinder.radius(), uv[1]];
                                curves.push(Curve2::Line(if co.reversed {
                                    [metric(*end), metric(*start)]
                                } else {
                                    [metric(*start), metric(*end)]
                                }));
                            }
                        }
                        Patch::Polygon(curves)
                    }
                }
                _ => return Err(QueryError::UnsupportedSurface { face: id }),
            };
            // The face frame's translation and full support enclosure also cover
            // large conics whose extrema are not authoritative edge endpoints.
            let bounds = face.surface.conservative_bounds(face.domain).map_err(|_| {
                QueryError::NumericLimit {
                    face: Some(id),
                    allowance: f64::INFINITY,
                    work: QueryWork::default(),
                }
            })?;
            magnitude = bounds
                .iter()
                .flatten()
                .map(|v| v.abs())
                .fold(magnitude, f64::max);
            let inverse =
                LocalFrame::new(face.surface.frame()).ok_or(QueryError::NumericLimit {
                    face: Some(id),
                    allowance: f64::INFINITY,
                    work: QueryWork::default(),
                })?;
            faces.push(PreparedFace { id, patch, inverse });
        }
        let mut edges = Vec::with_capacity(used.iter().filter(|yes| **yes).count());
        for (index, used) in used.into_iter().enumerate() {
            if used {
                let id = EdgeId(crate::usize_to_u32(index));
                edges.push(PreparedEdge::new(body, id).ok_or(QueryError::NumericLimit {
                    face: None,
                    allowance: f64::INFINITY,
                    work: QueryWork::default(),
                })?);
            }
        }
        fn curves(face: &PreparedFace) -> Option<&Vec<Curve2>> {
            match &face.patch {
                Patch::Planar(c) | Patch::Polygon(c) => Some(c),
                Patch::Band(_) => None,
            }
        }
        let report = PreparationReport {
            faces: faces.len(),
            edges: edges.len(),
            trim_curves: faces.iter().filter_map(curves).map(Vec::len).sum(),
            owned_capacity_bytes: validation.shells.capacity() * size_of::<BoundaryShell>()
                + validation
                    .shells
                    .iter()
                    .map(|s| s.faces.capacity() * size_of::<FaceId>())
                    .sum::<usize>()
                + validation.face_trims.capacity() * size_of::<TrimValidation>()
                + faces.capacity() * size_of::<PreparedFace>()
                + edges.capacity() * size_of::<PreparedEdge>()
                + faces
                    .iter()
                    .filter_map(curves)
                    .map(|c| c.capacity() * size_of::<Curve2>())
                    .sum::<usize>(),
            validated_coedges: validation.coedges,
            validation_pair_test_bound: validation.pair_test_bound,
            max_geometry_gap: validation
                .geometry
                .max_gap
                .max(validation.geometry.max_trim_gap),
        };
        Ok(Self {
            body,
            policy: *policy,
            faces,
            edges,
            magnitude,
            report,
            validation,
        })
    }
    /// Persistent storage and bounded local preparation evidence.
    #[must_use]
    pub const fn report(&self) -> &PreparationReport {
        &self.report
    }
    /// Local boundary evidence retained from preparation, including which face
    /// arrangements were certified or left unsupported by that inspection.
    /// Query support for continuous trim predicates does not turn an unsupported
    /// arrangement check into a validity certificate.
    #[must_use]
    pub const fn boundary_validation(&self) -> &BoundaryValidationReport {
        &self.validation
    }
    /// Classifies a body-local point using native geometry and continuous trims.
    ///
    /// Boundary is established by a witness on a trimmed face or bounded edge,
    /// including when the point projects just outside a face at a rim/vertex.
    /// Otherwise deterministic analytic rays count oriented material crossings.
    /// Tangencies and shared-edge crossings retry; uncertain roots are never
    /// silently discarded. Success does not certify the input globally.
    ///
    /// # Errors
    /// Refuses nonfinite points, numeric uncertainty, exhausted work/rays or
    /// inconsistent signed winding. No allocation occurs, including on refusal.
    pub fn query_point(&self, point: [f64; 3]) -> Result<PointQuery, QueryError> {
        if !point.iter().all(|v| v.is_finite()) {
            return Err(QueryError::NonFinitePoint);
        }
        let mut meter = Meter {
            work: QueryWork::default(),
            maximum: self.policy.max_query_work,
        };
        let magnitude = point.iter().map(|v| v.abs()).fold(self.magnitude, f64::max);
        let allowance = 4096.0 * f64::EPSILON * magnitude + self.report.max_geometry_gap;
        let tolerance = self.policy.boundary_tolerance;
        if !allowance.is_finite()
            || allowance >= tolerance * 0.125
            || magnitude > libm::sqrt(f64::MAX) / 64.0
        {
            return Err(meter.numeric(None, allowance));
        }
        let mut ambiguous = false;
        for edge in &self.edges {
            meter.edge()?;
            match self.edge_near(edge, point, tolerance, allowance, &mut meter)? {
                Near::Yes => {
                    return Ok(PointQuery {
                        location: PointLocation::Boundary,
                        boundary: Some(BoundaryWitness::Edge(edge.id)),
                        work: meter.work,
                    });
                }
                Near::Unknown => ambiguous = true,
                Near::No => (),
            }
        }
        for prepared in &self.faces {
            meter.support()?;
            let face = &self.body.faces()[prepared.id.index()];
            let local = prepared
                .inverse
                .point(point)
                .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
            let uv = match &face.surface {
                Surface::Plane(_) => [local[0], local[1]],
                Surface::Cylinder(_) => [
                    if local[0] == 0.0 && local[1] == 0.0 {
                        face.domain.u.at(0.5).expect("finite domain")
                    } else {
                        libm::atan2(local[1], local[0])
                    },
                    local[2],
                ],
                _ => unreachable!("prepared support"),
            };
            let witness = face
                .surface
                .evaluate(uv)
                .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?
                .point;
            let distance = norm(sub(point, witness));
            if distance > tolerance + allowance {
                continue;
            }
            match self.region(prepared, uv, allowance, &mut meter)? {
                Region::Inside if distance + allowance <= tolerance => {
                    return Ok(PointQuery {
                        location: PointLocation::Boundary,
                        boundary: Some(BoundaryWitness::Face(prepared.id)),
                        work: meter.work,
                    });
                }
                Region::Inside | Region::Edge => ambiguous = true,
                Region::Outside => (),
            }
        }
        if ambiguous {
            return Err(meter.ambiguous(Ambiguity::BoundaryTolerance));
        }
        let directions = [
            [1.0, 0.371, 0.529],
            [0.219, 1.0, -0.437],
            [-0.613, 0.293, 1.0],
            [1.0, -0.731, 0.183],
            [-0.347, 1.0, 0.819],
            [0.557, -0.239, 1.0],
            [1.0, 0.643, -0.887],
            [0.913, 1.0, 0.317],
        ];
        'ray: for direction in directions.into_iter().take(self.policy.max_ray_attempts) {
            meter.work.ray_attempts += 1;
            let direction = scale(direction, 1.0 / norm(direction));
            let mut winding = 0_i32;
            for prepared in &self.faces {
                meter.intersection()?;
                let face = &self.body.faces()[prepared.id.index()];
                let origin = prepared
                    .inverse
                    .point(point)
                    .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                let vector = prepared
                    .inverse
                    .vector(direction)
                    .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                let roots = match &face.surface {
                    Surface::Plane(_) => {
                        if vector[2].abs() <= 512.0 * f64::EPSILON {
                            if origin[2].abs() <= allowance {
                                continue 'ray;
                            }
                            [None; 2]
                        } else {
                            [Some(-origin[2] / vector[2]), None]
                        }
                    }
                    Surface::Cylinder(cylinder) => {
                        let a = vector[0] * vector[0] + vector[1] * vector[1];
                        let b = origin[0] * vector[0] + origin[1] * vector[1];
                        let radius = cylinder.radius();
                        let c = (libm::hypot(origin[0], origin[1]) - radius)
                            * (libm::hypot(origin[0], origin[1]) + radius);
                        if a <= 512.0 * f64::EPSILON {
                            if c.abs() <= allowance * (norm(origin) + radius) {
                                continue 'ray;
                            }
                            [None; 2]
                        } else {
                            let discriminant = b * b - a * c;
                            let error = 2048.0 * f64::EPSILON * (b * b + (a * c).abs())
                                + allowance * (b.abs() + radius) * 8.0;
                            if !discriminant.is_finite() || !error.is_finite() {
                                return Err(meter.numeric(Some(prepared.id), allowance));
                            }
                            if discriminant < -error {
                                [None; 2]
                            } else if discriminant <= error {
                                continue 'ray;
                            } else {
                                let q = -b - libm::sqrt(discriminant).copysign(b);
                                if q == 0.0 {
                                    continue 'ray;
                                }
                                [Some(q / a), Some(c / q)]
                            }
                        }
                    }
                    _ => unreachable!("prepared support"),
                };
                for t in roots.into_iter().flatten() {
                    if !t.is_finite() {
                        return Err(meter.numeric(Some(prepared.id), allowance));
                    }
                    if t < -allowance {
                        continue;
                    }
                    let hit = add(point, scale(direction, t));
                    let local = prepared
                        .inverse
                        .point(hit)
                        .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                    let uv = if matches!(face.surface, Surface::Plane(_)) {
                        [local[0], local[1]]
                    } else {
                        [libm::atan2(local[1], local[0]), local[2]]
                    };
                    let hit_allowance = allowance + 4096.0 * f64::EPSILON * t.abs();
                    match self.region(prepared, uv, hit_allowance, &mut meter)? {
                        Region::Outside => continue,
                        Region::Edge => continue 'ray,
                        Region::Inside => (),
                    }
                    // A near-zero root on an infinite support is irrelevant
                    // outside the actual bounded patch (for example above a bore).
                    if t <= allowance {
                        continue 'ray;
                    }
                    let evaluation = face
                        .surface
                        .evaluate(uv)
                        .ok_or_else(|| meter.numeric(Some(prepared.id), hit_allowance))?;
                    if norm(sub(evaluation.point, hit)) > hit_allowance * 4.0 {
                        continue 'ray;
                    }
                    let normal_dot = dot(evaluation.normal, direction) * face.sense.sign();
                    if normal_dot.abs() <= 4096.0 * f64::EPSILON {
                        continue 'ray;
                    }
                    winding = winding
                        .checked_add(if normal_dot > 0.0 { 1 } else { -1 })
                        .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                }
            }
            let location = match winding {
                0 => PointLocation::Outside,
                1 => PointLocation::Inside,
                _ => {
                    return Err(QueryError::InconsistentWinding {
                        winding,
                        work: meter.work,
                    });
                }
            };
            return Ok(PointQuery {
                location,
                boundary: None,
                work: meter.work,
            });
        }
        Err(meter.ambiguous(Ambiguity::RayDegeneracy))
    }
    fn region(
        &self,
        prepared: &PreparedFace,
        mut uv: [f64; 2],
        allowance: f64,
        meter: &mut Meter,
    ) -> Result<Region, QueryError> {
        let face = &self.body.faces()[prepared.id.index()];
        if let Surface::Cylinder(cylinder) = &face.surface {
            let [lo, hi] = face.domain.u.bounds();
            uv[0] += core::f64::consts::TAU * libm::ceil((lo - uv[0]) / core::f64::consts::TAU);
            if uv[0] > hi + allowance / cylinder.radius() {
                // The principal angle may be just below a partial patch's lower rim.
                if uv[0] - core::f64::consts::TAU < lo - allowance / cylinder.radius() {
                    return Ok(Region::Outside);
                }
                uv[0] -= core::f64::consts::TAU;
            }
            if let Patch::Band(band) = &prepared.patch {
                meter.trim()?;
                let bounds = band.boundaries();
                let lower = bounds[0]
                    .at(uv[0])
                    .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                let upper = bounds[1]
                    .at(uv[0])
                    .ok_or_else(|| meter.numeric(Some(prepared.id), allowance))?;
                if uv[1] < lower - allowance || uv[1] > upper + allowance {
                    return Ok(Region::Outside);
                }
                if (uv[1] - lower).abs() <= allowance || (uv[1] - upper).abs() <= allowance {
                    return Ok(Region::Edge);
                }
                return Ok(Region::Inside);
            }
            uv[0] *= cylinder.radius();
        }
        match &prepared.patch {
            Patch::Planar(curves) | Patch::Polygon(curves) => {
                trim::classify(curves, uv, allowance, meter)
            }
            Patch::Band(_) => unreachable!("cylindrical band"),
        }
    }
}

#[derive(Copy, Clone)]
enum Near {
    Yes,
    No,
    Unknown,
}
impl<F, E> SolidQuery<'_, F, E> {
    fn edge_near(
        &self,
        prepared: &PreparedEdge,
        point: [f64; 3],
        tolerance: f64,
        allowance: f64,
        meter: &mut Meter,
    ) -> Result<Near, QueryError> {
        let edge = &self.body.edges()[prepared.id.index()];
        let (curve, range) = edge.regular().expect("prepared regular edge");
        if let EdgeQuery::Line {
            origin,
            axis,
            length,
        } = prepared.geometry
        {
            let t = dot(sub(point, origin), axis).clamp(0.0, length);
            let distance = norm(sub(point, add(origin, scale(axis, t))));
            return Ok(if distance + allowance <= tolerance {
                Near::Yes
            } else if distance > tolerance + allowance {
                Near::No
            } else {
                Near::Unknown
            });
        }
        let EdgeQuery::Conic {
            inverse,
            axes,
            singular,
            speed,
        } = &prepared.geometry
        else {
            unreachable!("prepared conic")
        };
        let local = inverse
            .point(point)
            .ok_or_else(|| meter.numeric(None, allowance))?;
        let radius = libm::hypot(local[0] / axes[0], local[1] / axes[1]);
        let lower = singular * libm::hypot(axes[0].min(axes[1]) * (radius - 1.0).abs(), local[2]);
        if !lower.is_finite() {
            return Err(meter.numeric(None, allowance));
        }
        if lower > tolerance + allowance {
            return Ok(Near::No);
        }
        let [lo, hi] = range.bounds();
        let theta = libm::atan2(local[1] / axes[1], local[0] / axes[0]);
        let theta =
            theta + core::f64::consts::TAU * libm::ceil((lo - theta) / core::f64::consts::TAU);
        for t in [lo, hi, theta.clamp(lo, hi)] {
            let witness = curve
                .evaluate(t)
                .ok_or_else(|| meter.numeric(None, allowance))?
                .point;
            if norm(sub(point, witness)) + allowance <= tolerance {
                return Ok(Near::Yes);
            }
        }
        // A continuous derivative enclosure gives a distance lower bound on each
        // interval. This resolves ellipse thresholds without a sampled mesh.
        let mut stack = [(0.0, 0.0, 0_u8); 64];
        stack[0] = (lo, hi, 0);
        let mut count = 1;
        while count != 0 {
            count -= 1;
            let (a, b, depth) = stack[count];
            meter.subdivision()?;
            let mid = a + (b - a) * 0.5;
            let distance = norm(sub(
                point,
                curve
                    .evaluate(mid)
                    .ok_or_else(|| meter.numeric(None, allowance))?
                    .point,
            ));
            if distance + allowance <= tolerance {
                return Ok(Near::Yes);
            }
            if distance - speed * (b - a) * 0.5 > tolerance + allowance {
                continue;
            }
            if depth == 48 || mid == a || mid == b {
                return Ok(Near::Unknown);
            }
            stack[count] = (mid, b, depth + 1);
            stack[count + 1] = (a, mid, depth + 1);
            count += 2;
        }
        Ok(Near::No)
    }
}
