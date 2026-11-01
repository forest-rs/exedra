// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Closest points on one native rectangular ruled patch, independent of trims.

use crate::geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Ruled};
use alloc::collections::BinaryHeap;
use core::cmp::Ordering;
use exedra_curves::enclosure::{
    EnclosureError, EnclosureScratch, PointEnclosure, PreparationLimits, PreparedNurbsEnclosure,
    ProjectionEnclosure, SourceRange, line_enclosure, line_projection,
};
use exedra_math::{dot, sub};

/// Distance accuracy and explicit preparation/query resource bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionPolicy {
    /// Absolute upper-minus-lower distance gap, in source model units.
    /// This promises no UV error bound and no uniqueness of the nearest point.
    pub distance_tolerance: f64,
    /// Maximum admitted primitive work per query, including source control
    /// loads/blends, enclosure tests and ruling candidate evaluations.
    pub max_work: u64,
    /// Maximum outstanding U intervals. Scratch reserves this capacity once.
    pub max_pending_intervals: usize,
    /// Maximum combined original source control records admitted at preparation.
    /// A line contributes two records. Checked before visiting/allocating them.
    pub max_source_controls: usize,
    /// Combined preparation scan/control/blend work bound. Checked before
    /// constructing original-source proof hulls; separate from per-query work.
    pub max_preparation_work: u64,
}
impl Default for ProjectionPolicy {
    fn default() -> Self {
        Self {
            distance_tolerance: 1e-6,
            max_work: 100_000,
            max_pending_intervals: 4096,
            max_source_controls: 65_536,
            max_preparation_work: 1_000_000,
        }
    }
}

/// Actual admitted primitive query work. Preparation has its own reported bound.
/// Heap operations and span binary searches are bounded by the admitted pending
/// capacity and source record count; they are not extra primitive work units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionWork {
    /// Candidate ruling evaluations started, including endpoint candidates.
    /// V is clamped analytically to the nearest point on that ruling.
    pub ruling_evaluations: u64,
    /// Explicit native-UV hint evaluations started. The supplied V is retained;
    /// source enclosures and control steps are also charged independently.
    pub hint_evaluations: u64,
    /// Global U interval bounds evaluated, including rejected candidates.
    pub interval_tests: u64,
    /// Source position or projected-source enclosure requests started, including
    /// the span binary search (bounded by the prepared source-record count).
    pub source_enclosures: u64,
    /// Proof-control loads, row copies, hull visits and rational de Casteljau
    /// blends admitted. Each has fixed work for three coordinates.
    pub control_steps: u64,
}
impl ProjectionWork {
    /// Total primitive work, never exceeding the query's maximum.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.ruling_evaluations
            + self.hint_evaluations
            + self.interval_tests
            + self.source_enclosures
            + self.control_steps
    }
}

/// Why the global distance-bracket search stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionStatus {
    /// The global distance bracket is no wider than the requested tolerance.
    ToleranceMet,
    /// The next primitive operation would exceed the query work budget.
    WorkLimit,
    /// Splitting the next interval would exceed preallocated pending storage.
    PendingLimit,
    /// Outward arithmetic or representable parameter resolution cannot continue.
    NumericLimit,
}

/// A native-UV witness and its explicit positional and distance uncertainty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionWitness {
    /// Native normalized parameters inside the supplied rectangular domain.
    pub uv: [f64; 2],
    /// A representative of the exact native position at `uv`, not a mesh point.
    /// It may differ from another evaluator's representative by roundoff.
    pub point: [f64; 3],
    /// Upper bound on the representative's distance from the exact position at UV.
    pub point_error_upper: f64,
    /// Upper bound on distance from the target to the exact position at UV and
    /// to the returned representative.
    pub distance_upper: f64,
}

/// Global evidence for the complete rectangular patch, including all boundaries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionResult {
    /// Best completed candidate. Its UV is not claimed unique or stationary.
    pub witness: ProjectionWitness,
    /// No exact point of the supplied patch is nearer than this model-unit bound.
    pub distance_lower: f64,
    /// Termination distinguishes completed accuracy from bounded partial work.
    pub status: ProjectionStatus,
    /// Actual work, including incomplete bound/candidate attempts.
    pub work: ProjectionWork,
}
impl ProjectionResult {
    /// Whether the requested global distance gap was established.
    #[must_use]
    pub fn within_tolerance(&self) -> bool {
        self.status == ProjectionStatus::ToleranceMet
    }
}

/// Preparation size and retained payload, excluding the borrowed geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionPreparation {
    /// Original defining source records admitted, counting two per line.
    pub source_controls: usize,
    /// Largest source order; two for a line.
    pub max_source_order: usize,
    /// Retained source-enclosure allocation payload, excluding allocator headers.
    pub owned_capacity_bytes: usize,
    /// Admitted conservative work bound for preparation scans/loads/blends.
    pub preparation_work_bound: u64,
}

/// A query could not produce its first finite witness, or preparation was refused.
/// After a witness exists, bounded/numeric stops retain it in [`ProjectionResult`].
#[derive(Clone, Debug, PartialEq)]
pub enum ProjectionError {
    /// Invalid tolerance or a domain outside the native unit square.
    InvalidPolicy,
    /// The target contains a nonfinite coordinate.
    NonFinitePoint,
    /// A supplied native UV hint is nonfinite or outside the prepared rectangle.
    InvalidHint {
        /// Rejected native parameters; the hint never changes the search domain.
        uv: [f64; 2],
    },
    /// This source family has no original-source enclosure kernel yet.
    UnsupportedSource {
        /// Zero or one, identifying the ruled source.
        source: usize,
    },
    /// Combined source controls exceed the preparation bound.
    PreparationLimit {
        /// Required original source records.
        controls: usize,
        /// Maximum permitted records.
        maximum: usize,
    },
    /// Original-source proof preparation exceeds its explicit work bound.
    PreparationWorkLimit {
        /// Combined maximum permitted work.
        maximum: u64,
    },
    /// Scratch capacity is insufficient or allocation was refused.
    Capacity,
    /// Finite outward arithmetic could not enclose the source/first witness.
    NumericLimit,
    /// The first candidate could not finish under the work limit.
    WorkLimit {
        /// Work already completed, which may be zero.
        work: ProjectionWork,
    },
}
impl core::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "native ruled projection: {self:?}")
    }
}
impl core::error::Error for ProjectionError {}

#[derive(Debug)]
enum Source<'a> {
    Line(&'a CurveSpan),
    Nurbs(PreparedNurbsEnclosure<'a>),
}
impl<'a> Source<'a> {
    fn new(span: &'a CurveSpan, maximum: usize, work: u64) -> Result<Self, EnclosureError> {
        match span.curve() {
            Curve::Line(_) => {
                if work >= 2 {
                    Ok(Self::Line(span))
                } else {
                    Err(EnclosureError::WorkLimit)
                }
            }
            Curve::Nurbs(n) => Ok(Self::Nurbs(PreparedNurbsEnclosure::prepare(
                n.curve().view(),
                SourceRange {
                    parameters: span.range().bounds(),
                    reversed: span.reversed(),
                },
                PreparationLimits {
                    max_controls: maximum,
                    max_work: work,
                },
            )?)),
            _ => Err(EnclosureError::InvalidInput),
        }
    }
    fn preparation_work_bound(&self) -> u64 {
        match self {
            Self::Line(_) => 2,
            Self::Nurbs(n) => n.preparation_work_bound(),
        }
    }
    fn enclose(
        &self,
        range: [f64; 2],
        scratch: &mut EnclosureScratch,
        meter: &mut Meter,
    ) -> Result<PointEnclosure, EnclosureError> {
        if !meter.admit(Kind::Source) {
            return Err(EnclosureError::WorkLimit);
        }
        match self {
            Self::Nurbs(n) => n.enclose(range, scratch, || meter.admit(Kind::Control)),
            Self::Line(span) => {
                let Curve::Line(l) = span.curve() else {
                    unreachable!()
                };
                line_enclosure(
                    l.origin(),
                    l.direction(),
                    SourceRange {
                        parameters: span.range().bounds(),
                        reversed: span.reversed(),
                    },
                    range,
                )
            }
        }
    }
    fn project(
        &self,
        range: [f64; 2],
        target: [f64; 3],
        direction: [f64; 3],
        scratch: &mut EnclosureScratch,
        meter: &mut Meter,
    ) -> Result<ProjectionEnclosure, EnclosureError> {
        if !meter.admit(Kind::Source) {
            return Err(EnclosureError::WorkLimit);
        }
        match self {
            Self::Nurbs(n) => n.project(range, target, direction, scratch, || {
                meter.admit(Kind::Control)
            }),
            Self::Line(span) => {
                let Curve::Line(l) = span.curve() else {
                    unreachable!()
                };
                line_projection(
                    l.origin(),
                    l.direction(),
                    SourceRange {
                        parameters: span.range().bounds(),
                        reversed: span.reversed(),
                    },
                    range,
                    target,
                    direction,
                )
            }
        }
    }
}

/// Prepared closest-point search on an existing native ruled rectangle.
///
/// The two native source curves remain authoritative. For fixed U, the closest
/// V on its ruling is clamped analytically. A best-first one-parameter search
/// bounds the whole remaining patch by outward original-source coordinate and
/// directional-support enclosures. A local candidate never proves globality.
///
/// This service projects onto the supplied rectangle, not a trimmed face or a
/// solid. Line and NURBS sources are supported; conics are explicitly refused.
/// Results give distance brackets, not unique inverse parameters or UV accuracy.
///
/// ```
/// use exedra_analytic::{
///     geometry::{Curve, CurveSpan, ParameterDomain, ParameterRange, Surface},
///     query::{ProjectionPolicy, RuledProjection},
/// };
///
/// let row = |z| {
///     CurveSpan::new(
///         Curve::line([0.0, 0.0, z], [4.0, 0.0, 0.0]).unwrap(),
///         ParameterRange::UNIT,
///         false,
///     ).unwrap()
/// };
/// let surface = Surface::ruled(row(0.0), row(1.0)).unwrap();
/// let Surface::Ruled(ruled) = &surface else { unreachable!() };
/// let domain = ParameterDomain {
///     u: ParameterRange::UNIT,
///     v: ParameterRange::UNIT,
/// };
/// let query = RuledProjection::prepare(
///     ruled, domain, &ProjectionPolicy::default(),
/// ).unwrap();
/// let mut scratch = query.scratch().unwrap();
/// let found = query.closest([1.2, 2.0, 0.7], &mut scratch).unwrap();
/// assert!(found.within_tolerance());
/// assert!(found.distance_lower <= 2.0 && found.witness.distance_upper >= 2.0);
/// ```
#[derive(Debug)]
pub struct RuledProjection<'a> {
    sources: [Source<'a>; 2],
    domain: ParameterDomain,
    policy: ProjectionPolicy,
    report: ProjectionPreparation,
}
impl<'a> RuledProjection<'a> {
    /// Prepare immutable original-source enclosures without cloning the curves.
    ///
    /// # Errors
    /// Refuses unsupported sources, domains outside the unit square, excess
    /// source records, allocation failure and unrepresentable outward geometry.
    pub fn prepare(
        ruled: &'a Ruled,
        domain: ParameterDomain,
        policy: &ProjectionPolicy,
    ) -> Result<Self, ProjectionError> {
        if !policy.distance_tolerance.is_finite()
            || policy.distance_tolerance < 0.0
            || domain
                .u
                .bounds()
                .into_iter()
                .chain(domain.v.bounds())
                .any(|v| !ParameterRange::UNIT.contains(v))
        {
            return Err(ProjectionError::InvalidPolicy);
        }
        let mut report = ProjectionPreparation::default();
        for (i, span) in ruled.curves().iter().enumerate() {
            let (controls, order) = match span.curve() {
                Curve::Line(_) => (2, 2),
                Curve::Nurbs(n) => (
                    n.curve().points().len(),
                    usize::try_from(n.curve().order())
                        .expect("validated order fits source controls"),
                ),
                _ => return Err(ProjectionError::UnsupportedSource { source: i }),
            };
            report.source_controls = report.source_controls.saturating_add(controls);
            report.max_source_order = report.max_source_order.max(order);
        }
        if report.source_controls > policy.max_source_controls {
            return Err(ProjectionError::PreparationLimit {
                controls: report.source_controls,
                maximum: policy.max_source_controls,
            });
        }
        let prepare_error = |e| {
            if e == EnclosureError::WorkLimit {
                ProjectionError::PreparationWorkLimit {
                    maximum: policy.max_preparation_work,
                }
            } else {
                preparation_error(e)
            }
        };
        let first = Source::new(
            &ruled.curves()[0],
            policy.max_source_controls,
            policy.max_preparation_work,
        )
        .map_err(prepare_error)?;
        let second = Source::new(
            &ruled.curves()[1],
            policy.max_source_controls,
            policy.max_preparation_work - first.preparation_work_bound(),
        )
        .map_err(prepare_error)?;
        let sources = [first, second];
        report.preparation_work_bound = sources.iter().map(Source::preparation_work_bound).sum();
        report.owned_capacity_bytes = sources
            .iter()
            .map(|s| match s {
                Source::Line(_) => 0,
                Source::Nurbs(n) => n.capacity_bytes(),
            })
            .sum();
        Ok(Self {
            sources,
            domain,
            policy: *policy,
            report,
        })
    }
    /// Preparation counts and retained source-enclosure payload.
    #[must_use]
    pub const fn preparation(&self) -> ProjectionPreparation {
        self.report
    }
    /// Allocate scratch once for repeated queries. Hot queries never grow it.
    pub fn scratch(&self) -> Result<ProjectionScratch, ProjectionError> {
        let mut pending = BinaryHeap::new();
        pending
            .try_reserve_exact(self.policy.max_pending_intervals)
            .map_err(|_| ProjectionError::Capacity)?;
        Ok(ProjectionScratch {
            pending,
            source: EnclosureScratch::new(self.report.max_source_order)
                .map_err(preparation_error)?,
            max_source_order: self.report.max_source_order,
        })
    }
    /// Find a globally bounded closest point, retaining the best completed
    /// witness when work, storage or numerical resolution interrupts the search.
    ///
    /// # Errors
    /// Refuses nonfinite targets, insufficient scratch, and failure to complete
    /// even the first witness. Zero work performs no source evaluation.
    pub fn closest(
        &self,
        target: [f64; 3],
        scratch: &mut ProjectionScratch,
    ) -> Result<ProjectionResult, ProjectionError> {
        self.closest_impl(target, None, scratch)
    }
    /// Supply a feasible native-UV candidate without restricting global search.
    ///
    /// The hint is checked against the prepared rectangle and evaluated under
    /// the complete source/control work budget. Its outward distance upper
    /// bound may establish the requested gap immediately against the universal
    /// lower bound zero. Otherwise the same complete-domain search runs, keeping
    /// the hint only when it improves the best candidate. No local convergence,
    /// stationarity or uniqueness is assumed, and scratch retains no hint state.
    ///
    /// # Errors
    /// In addition to [`Self::closest`] failures, rejects nonfinite/out-of-domain
    /// hints with [`ProjectionError::InvalidHint`]. Zero work evaluates no hint.
    pub fn closest_with_hint(
        &self,
        target: [f64; 3],
        hint_uv: [f64; 2],
        scratch: &mut ProjectionScratch,
    ) -> Result<ProjectionResult, ProjectionError> {
        self.closest_impl(target, Some(hint_uv), scratch)
    }
    fn closest_impl(
        &self,
        target: [f64; 3],
        hint: Option<[f64; 2]>,
        scratch: &mut ProjectionScratch,
    ) -> Result<ProjectionResult, ProjectionError> {
        if target.iter().any(|v| !v.is_finite()) {
            return Err(ProjectionError::NonFinitePoint);
        }
        if let Some(uv) = hint
            && (!self.domain.u.contains(uv[0]) || !self.domain.v.contains(uv[1]))
        {
            return Err(ProjectionError::InvalidHint { uv });
        }
        if scratch.max_source_order < self.report.max_source_order
            || scratch.pending.capacity() < self.policy.max_pending_intervals
        {
            return Err(ProjectionError::Capacity);
        }
        scratch.pending.clear();
        let mut meter = Meter {
            work: ProjectionWork::default(),
            maximum: self.policy.max_work,
        };
        let [lo, hi] = self.domain.u.bounds();
        let middle = lo + (hi - lo) * 0.5;
        let first = hint.unwrap_or([middle, 0.]);
        let mut best = self
            .candidate(
                first[0],
                hint.map(|uv| uv[1]),
                target,
                &mut scratch.source,
                &mut meter,
            )
            .map_err(|e| query_error(e, meter.work))?;
        if hint.is_some() {
            if gap_met(best.distance_upper, 0., self.policy.distance_tolerance) {
                return Ok(result(best, 0., ProjectionStatus::ToleranceMet, meter.work));
            }
            match self.candidate(middle, None, target, &mut scratch.source, &mut meter) {
                Ok(candidate) => offer(&mut best, candidate),
                Err(e) => return Ok(result(best, 0., status(e), meter.work)),
            }
        }
        for u in [lo, hi] {
            match self.candidate(u, None, target, &mut scratch.source, &mut meter) {
                Ok(candidate) => offer(&mut best, candidate),
                Err(e) => return Ok(result(best, 0.0, status(e), meter.work)),
            }
        }
        let lower = match self.lower([lo, hi], target, &mut best, &mut scratch.source, &mut meter) {
            Ok(lower) => lower,
            Err(e) => return Ok(result(best, 0.0, status(e), meter.work)),
        };
        if gap_met(best.distance_upper, lower, self.policy.distance_tolerance) {
            return Ok(result(
                best,
                lower,
                ProjectionStatus::ToleranceMet,
                meter.work,
            ));
        }
        if self.policy.max_pending_intervals == 0 {
            return Ok(result(
                best,
                lower,
                ProjectionStatus::PendingLimit,
                meter.work,
            ));
        }
        scratch.pending.push(Node {
            range: [lo, hi],
            lower,
        });
        let mut retired = f64::INFINITY;
        loop {
            let lower = scratch
                .pending
                .peek()
                .map_or(f64::INFINITY, |n| n.lower)
                .min(retired)
                .min(best.distance_upper);
            if gap_met(best.distance_upper, lower, self.policy.distance_tolerance) {
                return Ok(result(
                    best,
                    lower,
                    ProjectionStatus::ToleranceMet,
                    meter.work,
                ));
            }
            let Some(node) = scratch.pending.pop() else {
                return Ok(result(
                    best,
                    lower,
                    ProjectionStatus::NumericLimit,
                    meter.work,
                ));
            };
            if node.lower >= best.distance_upper {
                retired = retired.min(node.lower);
                continue;
            }
            let [a, b] = node.range;
            let mid = a + (b - a) * 0.5;
            if mid <= a || mid >= b {
                return Ok(result(
                    best,
                    lower.min(node.lower),
                    ProjectionStatus::NumericLimit,
                    meter.work,
                ));
            }
            if scratch.pending.len() + 2 > self.policy.max_pending_intervals {
                return Ok(result(
                    best,
                    lower.min(node.lower),
                    ProjectionStatus::PendingLimit,
                    meter.work,
                ));
            }
            let mut children = [node; 2];
            for (child, range) in children.iter_mut().zip([[a, mid], [mid, b]]) {
                child.range = range;
                child.lower =
                    match self.lower(range, target, &mut best, &mut scratch.source, &mut meter) {
                        Ok(lower) => lower.max(node.lower),
                        // The parent still covers BOTH children if either attempt
                        // stops. Never discard an unbounded part of the domain.
                        Err(e) => {
                            return Ok(result(best, lower.min(node.lower), status(e), meter.work));
                        }
                    };
            }
            for child in children {
                scratch.pending.push(child);
            }
        }
    }
    fn candidate(
        &self,
        u: f64,
        hint_v: Option<f64>,
        target: [f64; 3],
        scratch: &mut EnclosureScratch,
        meter: &mut Meter,
    ) -> Result<ProjectionWitness, EnclosureError> {
        if !meter.admit(if hint_v.is_some() {
            Kind::Hint
        } else {
            Kind::Ruling
        }) {
            return Err(EnclosureError::WorkLimit);
        }
        let a = self.sources[0].enclose([u, u], scratch, meter)?;
        let b = self.sources[1].enclose([u, u], scratch, meter)?;
        let v = if let Some(v) = hint_v {
            v
        } else {
            let p = a.midpoint();
            let d = sub(b.midpoint(), p);
            let magnitude = d.iter().copied().map(f64::abs).fold(0.0, f64::max);
            if magnitude == 0.0 || !magnitude.is_finite() {
                return Err(EnclosureError::NumericLimit);
            }
            let axis = d.map(|d| d / magnitude);
            (dot(sub(target, p), axis) / magnitude / dot(axis, axis))
                .clamp(self.domain.v.bounds()[0], self.domain.v.bounds()[1])
        };
        if !v.is_finite() {
            return Err(EnclosureError::NumericLimit);
        }
        let enclosure = a.affine_blend(b, [v, v])?;
        let point = enclosure.midpoint();
        Ok(ProjectionWitness {
            uv: [u, v],
            point,
            point_error_upper: enclosure.distance_bounds(point)?[1],
            distance_upper: enclosure.distance_bounds(target)?[1],
        })
    }
    fn lower(
        &self,
        range: [f64; 2],
        target: [f64; 3],
        best: &mut ProjectionWitness,
        scratch: &mut EnclosureScratch,
        meter: &mut Meter,
    ) -> Result<f64, EnclosureError> {
        if !meter.admit(Kind::Interval) {
            return Err(EnclosureError::WorkLimit);
        }
        let candidate = self.candidate(
            range[0] + (range[1] - range[0]) * 0.5,
            None,
            target,
            scratch,
            meter,
        )?;
        offer(best, candidate);
        let a = self.sources[0].enclose(range, scratch, meter)?;
        let b = self.sources[1].enclose(range, scratch, meter)?;
        let mut lower = a
            .affine_blend(b, self.domain.v.bounds())?
            .distance_bounds(target)?[0];
        let direction = sub(candidate.point, target);
        let magnitude = direction.iter().copied().map(f64::abs).fold(0.0, f64::max);
        if magnitude > 0.0 && magnitude.is_finite() {
            let direction = direction.map(|d| d / magnitude);
            let a = self.sources[0].project(range, target, direction, scratch, meter)?;
            let b = self.sources[1].project(range, target, direction, scratch, meter)?;
            lower = lower.max(
                a.affine_blend(b, self.domain.v.bounds())?
                    .distance_lower()?,
            );
        }
        Ok(lower)
    }
}

/// Reusable pending-interval heap and source evaluation rows.
#[derive(Debug)]
pub struct ProjectionScratch {
    pending: BinaryHeap<Node>,
    source: EnclosureScratch,
    max_source_order: usize,
}
impl ProjectionScratch {
    /// Retained allocation payload, excluding allocator headers and prepared data.
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        self.pending.capacity() * size_of::<Node>() + self.source.capacity_bytes()
    }
}
#[derive(Clone, Copy, Debug)]
struct Node {
    range: [f64; 2],
    lower: f64,
}
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.lower == other.lower && self.range == other.range
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .lower
            .total_cmp(&self.lower)
            .then_with(|| other.range[0].total_cmp(&self.range[0]))
            .then_with(|| other.range[1].total_cmp(&self.range[1]))
    }
}
enum Kind {
    Ruling,
    Hint,
    Interval,
    Source,
    Control,
}
struct Meter {
    work: ProjectionWork,
    maximum: u64,
}
impl Meter {
    fn admit(&mut self, kind: Kind) -> bool {
        if self.work.total() >= self.maximum {
            return false;
        }
        let slot = match kind {
            Kind::Ruling => &mut self.work.ruling_evaluations,
            Kind::Hint => &mut self.work.hint_evaluations,
            Kind::Interval => &mut self.work.interval_tests,
            Kind::Source => &mut self.work.source_enclosures,
            Kind::Control => &mut self.work.control_steps,
        };
        *slot += 1;
        true
    }
}
fn offer(best: &mut ProjectionWitness, candidate: ProjectionWitness) {
    if candidate.distance_upper < best.distance_upper
        || (candidate.distance_upper == best.distance_upper && candidate.uv < best.uv)
    {
        *best = candidate;
    }
}
fn gap_met(upper: f64, lower: f64, tolerance: f64) -> bool {
    upper == lower || (upper - lower).next_up() <= tolerance
}
fn result(
    witness: ProjectionWitness,
    lower: f64,
    status: ProjectionStatus,
    work: ProjectionWork,
) -> ProjectionResult {
    ProjectionResult {
        witness,
        distance_lower: lower.min(witness.distance_upper).max(0.0),
        status,
        work,
    }
}
fn status(e: EnclosureError) -> ProjectionStatus {
    if e == EnclosureError::WorkLimit {
        ProjectionStatus::WorkLimit
    } else {
        ProjectionStatus::NumericLimit
    }
}
fn preparation_error(e: EnclosureError) -> ProjectionError {
    match e {
        EnclosureError::Capacity => ProjectionError::Capacity,
        _ => ProjectionError::NumericLimit,
    }
}
fn query_error(e: EnclosureError, work: ProjectionWork) -> ProjectionError {
    if e == EnclosureError::WorkLimit {
        ProjectionError::WorkLimit { work }
    } else {
        preparation_error(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{NurbsCurve, Surface};
    use alloc::vec;
    use exedra_math::Placement3;

    fn unit() -> ParameterDomain {
        ParameterDomain {
            u: ParameterRange::UNIT,
            v: ParameterRange::UNIT,
        }
    }
    fn plane(scale: f64) -> Surface {
        let span = |z| {
            CurveSpan::new(
                Curve::line([0., 0., z], [4. * scale, 0., 0.]).unwrap(),
                ParameterRange::UNIT,
                false,
            )
            .unwrap()
        };
        Surface::ruled(span(0.), span(2. * scale)).unwrap()
    }
    fn query(
        surface: &Surface,
        domain: ParameterDomain,
        policy: ProjectionPolicy,
        target: [f64; 3],
    ) -> ProjectionResult {
        let Surface::Ruled(r) = surface else {
            unreachable!()
        };
        let prepared = RuledProjection::prepare(r, domain, &policy).unwrap();
        let mut scratch = prepared.scratch().unwrap();
        let capacity = scratch.capacity_bytes();
        let found = prepared.closest(target, &mut scratch).unwrap();
        assert_eq!(capacity, scratch.capacity_bytes());
        assert!(found.work.total() <= policy.max_work);
        assert!(domain.u.contains(found.witness.uv[0]) && domain.v.contains(found.witness.uv[1]));
        let native = surface.evaluate(found.witness.uv).unwrap().point;
        let error = sub(native, found.witness.point);
        let distance = libm::hypot(libm::hypot(error[0], error[1]), error[2]);
        assert!(distance <= found.witness.point_error_upper + 1e-14 * scale_of(native));
        found
    }
    fn scale_of(p: [f64; 3]) -> f64 {
        p.into_iter().map(f64::abs).fold(1., f64::max)
    }
    fn check_bracket(found: ProjectionResult, exact: f64, tolerance: f64) {
        assert!(found.within_tolerance(), "{found:?}");
        assert!(
            found.distance_lower <= exact && exact <= found.witness.distance_upper,
            "exact={exact:?} {found:?}"
        );
        assert!(found.witness.distance_upper - found.distance_lower <= tolerance);
    }
    #[test]
    fn analytic_plane_global_distances_include_edges_corners_and_subdomains() {
        let surface = plane(1.);
        for (target, exact) in [
            ([1.2, 3., 0.7], 3.),
            ([-1., 3., 3.], libm::sqrt(11.)),
            ([1.2, 0., 0.7], 0.),
        ] {
            check_bracket(
                query(&surface, unit(), ProjectionPolicy::default(), target),
                exact,
                1e-6,
            );
        }
        let domain = ParameterDomain {
            u: ParameterRange::new(0.25, 0.75).unwrap(),
            v: ParameterRange::new(0.2, 0.8).unwrap(),
        };
        check_bracket(
            query(&surface, domain, ProjectionPolicy::default(), [0., 0., 2.]),
            libm::sqrt(1.16),
            1e-6,
        );
    }
    fn crown(scale: f64, reversed: bool) -> Surface {
        let curve = Curve::nurbs(
            NurbsCurve::new(
                4,
                vec![2., 2., 2., 2., 5., 5., 5., 5.],
                vec![[0., 0., 0.], [1., 2., 0.], [3., 2., 0.], [4., 0., 0.]],
                Some(vec![1., 0.7, 1.4, 1.]),
            )
            .unwrap(),
        )
        .unwrap();
        let Curve::Nurbs(n) = &curve else {
            unreachable!()
        };
        let upper = Curve::nurbs(
            NurbsCurve::new(
                4,
                n.curve().knots().to_vec(),
                n.curve()
                    .points()
                    .iter()
                    .map(|p| [2. + scale * (p[0] - 2.), scale * p[1], 1.])
                    .collect(),
                Some(n.curve().weights().unwrap().to_vec()),
            )
            .unwrap(),
        )
        .unwrap();
        let range = ParameterRange::new(2., 5.).unwrap();
        Surface::ruled(
            CurveSpan::new(curve, range, reversed).unwrap(),
            CurveSpan::new(upper, range, reversed).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn independent_bounded_crown_oracle_and_reversed_source_ranges() {
        // Independently computed bounded OCCT minima for the original crown
        // curves, not a triangulated or sampled reference.
        for (scale, exact) in [(1., 0.507_200_618_784_817), (0.75, 0.648_493_727_321_868)] {
            for reversed in [false, true] {
                let surface = crown(scale, reversed);
                let found = query(&surface, unit(), ProjectionPolicy::default(), [2., 2., 0.5]);
                assert!(found.within_tolerance(), "{found:?}");
                assert!(
                    found.distance_lower <= exact + 1e-13
                        && found.witness.distance_upper >= exact - 1e-13,
                    "{found:?}"
                );
                assert!(found.witness.distance_upper - found.distance_lower <= 1e-6);
                let outside = if scale == 1. { 1.5 } else { libm::sqrt(3.5) };
                check_bracket(
                    query(
                        &surface,
                        unit(),
                        ProjectionPolicy::default(),
                        [-1., -0.5, 2.],
                    ),
                    outside,
                    1e-6,
                );
            }
        }
    }
    #[test]
    fn exhausted_work_retains_a_witness_and_the_parent_global_lower() {
        let surface = crown(0.75, false);
        let Surface::Ruled(r) = &surface else {
            unreachable!()
        };
        for maximum in [0, 1, 32, 50, 100, 200, 500, 1000] {
            let policy = ProjectionPolicy {
                max_work: maximum,
                ..Default::default()
            };
            let prepared = RuledProjection::prepare(r, unit(), &policy).unwrap();
            let result = prepared.closest([2., 2., 0.5], &mut prepared.scratch().unwrap());
            match result {
                Err(ProjectionError::WorkLimit { work }) => assert!(work.total() <= maximum),
                Ok(found) => {
                    assert!(found.work.total() <= maximum);
                    assert!(found.distance_lower <= 0.648_493_727_321_868 + 1e-13);
                    assert!(found.witness.distance_upper >= 0.648_493_727_321_868 - 1e-13);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        let found = query(
            &surface,
            unit(),
            ProjectionPolicy {
                max_pending_intervals: 1,
                ..Default::default()
            },
            [2., 2., 0.5],
        );
        assert_eq!(found.status, ProjectionStatus::PendingLimit);
        assert!(found.distance_lower <= 0.648_493_727_321_868);
    }
    #[test]
    fn unsupported_sources_invalid_targets_and_preparation_limits_are_explicit() {
        let c = Curve::circle(Placement3::IDENTITY, 1.).unwrap();
        let range = ParameterRange::new(0., 1.).unwrap();
        let surface = Surface::ruled(
            CurveSpan::new(c.clone(), range, false).unwrap(),
            CurveSpan::new(
                c.transformed(Placement3::translate(0., 0., 1.)).unwrap(),
                range,
                false,
            )
            .unwrap(),
        )
        .unwrap();
        let Surface::Ruled(r) = &surface else {
            unreachable!()
        };
        assert!(matches!(
            RuledProjection::prepare(r, unit(), &ProjectionPolicy::default()),
            Err(ProjectionError::UnsupportedSource { source: 0 })
        ));
        let surface = crown(1., false);
        let Surface::Ruled(r) = &surface else {
            unreachable!()
        };
        assert!(matches!(
            RuledProjection::prepare(
                r,
                unit(),
                &ProjectionPolicy {
                    max_source_controls: 7,
                    ..Default::default()
                }
            ),
            Err(ProjectionError::PreparationLimit { controls: 8, .. })
        ));
        let prepared = RuledProjection::prepare(r, unit(), &ProjectionPolicy::default()).unwrap();
        assert!(matches!(
            RuledProjection::prepare(
                r,
                unit(),
                &ProjectionPolicy {
                    max_preparation_work: 0,
                    ..Default::default()
                }
            ),
            Err(ProjectionError::PreparationWorkLimit { maximum: 0 })
        ));
        assert!(matches!(
            prepared.closest([f64::NAN, 0., 0.], &mut prepared.scratch().unwrap()),
            Err(ProjectionError::NonFinitePoint)
        ));
    }

    #[test]
    fn unequal_weight_multispan_sources_project_to_the_exact_rectangular_image() {
        let source = |z, weights| {
            let curve = Curve::nurbs(
                NurbsCurve::new(
                    3,
                    vec![2., 2., 2., 3., 4., 5., 5., 5.],
                    vec![
                        [0., 0., z],
                        [0.5, 0., z],
                        [2., 0., z],
                        [3.5, 0., z],
                        [4., 0., z],
                    ],
                    Some(weights),
                )
                .unwrap(),
            )
            .unwrap();
            CurveSpan::new(curve, ParameterRange::new(2., 5.).unwrap(), true).unwrap()
        };
        let surface = Surface::ruled(
            source(0., vec![1., 0.5, 2., 0.75, 1.]),
            source(1., vec![1., 2., 0.5, 1.5, 1.]),
        )
        .unwrap();
        for (point, distance) in [([1.3, 2., 0.7], 2.), ([-1., 2., 2.], libm::sqrt(6.))] {
            check_bracket(
                query(&surface, unit(), ProjectionPolicy::default(), point),
                distance,
                1e-6,
            );
        }
    }

    #[test]
    fn symmetric_wavy_patch_retains_both_global_minimum_candidates() {
        let curve = Curve::nurbs(
            NurbsCurve::new(
                6,
                [vec![2.; 6], vec![5.; 6]].concat(),
                [0., 3., -3., -3., 3., 0.]
                    .into_iter()
                    .zip([0., 0.8, 1.6, 2.4, 3.2, 4.])
                    .map(|(y, x)| [x, y, 0.])
                    .collect(),
                Some(vec![1., 0.8, 1.3, 1.3, 0.8, 1.]),
            )
            .unwrap(),
        )
        .unwrap();
        let upper = curve
            .transformed(Placement3::translate(0., 0., 1.))
            .unwrap();
        let range = ParameterRange::new(2., 5.).unwrap();
        let surface = Surface::ruled(
            CurveSpan::new(curve, range, true).unwrap(),
            CurveSpan::new(upper, range, true).unwrap(),
        )
        .unwrap();
        let found = query(&surface, unit(), ProjectionPolicy::default(), [2., 2., 0.5]);
        assert!(found.within_tolerance(), "{found:?}");
        assert!(found.witness.uv[0] < 0.25 || found.witness.uv[0] > 0.75);
        let mirror = surface
            .evaluate([1. - found.witness.uv[0], found.witness.uv[1]])
            .unwrap()
            .point;
        let distance = libm::hypot(mirror[0] - 2., mirror[1] - 2.);
        assert!(
            distance >= found.distance_lower - 1e-13
                && distance <= found.witness.distance_upper + 1e-13
        );
        assert!((mirror[0] + found.witness.point[0] - 4.).abs() < 1e-12);
    }

    #[test]
    fn transformed_native_sources_expose_their_numeric_accuracy_floor() {
        let surface = plane(1.);
        let Surface::Ruled(r) = surface else {
            unreachable!()
        };
        let placement = Placement3::from_axes(
            [0., 1., 0.],
            [-1., 0., 0.],
            [0., 0., 1.],
            [1e10, -1e10, 1e10],
        );
        let surface = Surface::ruled(
            r.curves()[0].transformed(placement).unwrap(),
            r.curves()[1].transformed(placement).unwrap(),
        )
        .unwrap();
        let target = placement.transform_point([1.2, 3., 0.7]);
        check_bracket(
            query(
                &surface,
                unit(),
                ProjectionPolicy {
                    distance_tolerance: 1e-3,
                    ..Default::default()
                },
                target,
            ),
            3.,
            1e-3,
        );
        let found = query(
            &surface,
            unit(),
            ProjectionPolicy {
                distance_tolerance: 1e-10,
                max_work: 1000,
                ..Default::default()
            },
            target,
        );
        assert!(!found.within_tolerance());
        assert!(found.distance_lower <= 3. && found.witness.distance_upper >= 3.);
        assert!(found.witness.point_error_upper > 1e-10);
    }

    #[test]
    fn a_native_uv_hint_proves_a_small_global_gap_without_subdivision() {
        for scale in [1., 0.75] {
            let surface = crown(scale, false);
            let Surface::Ruled(r) = &surface else {
                unreachable!()
            };
            let query = RuledProjection::prepare(r, unit(), &ProjectionPolicy::default()).unwrap();
            let mut scratch = query.scratch().unwrap();
            let capacity = scratch.capacity_bytes();
            for uv in [[0.37, 0.43], [0.71, 0.64]] {
                let point = surface.evaluate(uv).unwrap().point;
                let found = query.closest_with_hint(point, uv, &mut scratch).unwrap();
                assert!(found.within_tolerance());
                assert_eq!(found.witness.uv, uv);
                assert_eq!(found.distance_lower, 0.);
                assert!(found.witness.distance_upper <= 1e-6);
                assert_eq!(found.work.hint_evaluations, 1);
                assert_eq!(found.work.ruling_evaluations, 0);
                assert_eq!(found.work.interval_tests, 0);
                assert_eq!(found.work.source_enclosures, 2);
                assert!(found.work.control_steps > 0);
                assert_eq!(scratch.capacity_bytes(), capacity);
            }
        }
    }

    #[test]
    fn off_surface_and_wrong_valid_hints_still_search_the_complete_domain() {
        let surface = crown(0.75, false);
        let Surface::Ruled(r) = &surface else {
            unreachable!()
        };
        let query = RuledProjection::prepare(r, unit(), &ProjectionPolicy::default()).unwrap();
        let mut scratch = query.scratch().unwrap();
        for hint in [[0., 0.], [0.5, 0.5], [1., 1.]] {
            let found = query
                .closest_with_hint([2., 2., 0.5], hint, &mut scratch)
                .unwrap();
            assert!(found.within_tolerance());
            assert!(found.work.interval_tests > 0);
            assert_eq!(found.work.hint_evaluations, 1);
            assert!(
                found.distance_lower <= 0.648_493_727_321_868 + 1e-13
                    && found.witness.distance_upper >= 0.648_493_727_321_868 - 1e-13
            );
        }
        let uv = [0.37, 0.43];
        let e = surface.evaluate(uv).unwrap();
        let target = core::array::from_fn(|k| e.point[k] + 0.25 * e.normal[k]);
        let found = query.closest_with_hint(target, uv, &mut scratch).unwrap();
        assert!(found.within_tolerance());
        assert!(found.work.interval_tests > 0);
        assert!(
            found.distance_lower <= 0.25 + 1e-13 && found.witness.distance_upper >= 0.25 - 1e-13
        );
    }

    #[test]
    fn hints_are_domain_checked_metered_and_retained_when_later_work_stops() {
        let surface = crown(0.75, false);
        let Surface::Ruled(r) = &surface else {
            unreachable!()
        };
        let domain = ParameterDomain {
            u: ParameterRange::new(0.25, 0.75).unwrap(),
            v: ParameterRange::new(0.2, 0.8).unwrap(),
        };
        let prepared = RuledProjection::prepare(r, domain, &ProjectionPolicy::default()).unwrap();
        let mut scratch = prepared.scratch().unwrap();
        for uv in [
            [f64::NAN, 0.5],
            [0.5, f64::INFINITY],
            [0.1, 0.5],
            [0.5, 0.9],
        ] {
            assert!(matches!(
                prepared.closest_with_hint([2., 2., 0.5], uv, &mut scratch),
                Err(ProjectionError::InvalidHint { .. })
            ));
        }
        let uv = [0.25, 0.8];
        let target = surface.evaluate(uv).unwrap().point;
        let full = prepared
            .closest_with_hint(target, uv, &mut scratch)
            .unwrap();
        assert!(full.within_tolerance());
        let cost = full.work.total();
        for budget in [0, cost - 1, cost] {
            let prepared = RuledProjection::prepare(
                r,
                domain,
                &ProjectionPolicy {
                    max_work: budget,
                    ..Default::default()
                },
            )
            .unwrap();
            let result = prepared.closest_with_hint(target, uv, &mut prepared.scratch().unwrap());
            if budget < cost {
                assert!(
                    matches!(result,Err(ProjectionError::WorkLimit{work}) if work.total()==budget)
                );
            } else {
                assert!(result.unwrap().within_tolerance());
            }
        }
        let prepared = RuledProjection::prepare(
            r,
            domain,
            &ProjectionPolicy {
                max_work: cost,
                ..Default::default()
            },
        )
        .unwrap();
        let found = prepared
            .closest_with_hint([2., 2., 0.5], uv, &mut prepared.scratch().unwrap())
            .unwrap();
        assert_eq!(found.status, ProjectionStatus::WorkLimit);
        assert_eq!(found.work.total(), cost);
        assert_eq!(found.witness.uv, uv);
        assert_eq!(found.distance_lower, 0.);
        let plain = prepared.closest([2., 2., 0.5], &mut scratch);
        assert!(matches!(plain,Ok(ProjectionResult{work,..}) if work.hint_evaluations==0));
    }
}
