// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic profile discretization with per-segment provenance.
//!
//! Discretization turns a validated [`Profile2`] into point loops whose
//! every edge knows which source segment produced it. Determinism is the
//! contract: identical profile bits and policy produce identical output
//! bits on every platform and build mode.
//!
//! - Segment endpoints are emitted *exactly* as stored — never recomputed —
//!   so loop closure survives discretization bit-for-bit.
//! - Arc interior points are computed with [`libm`] trig only (never `std`,
//!   never kurbo's feature-dependent dispatch), from subdivision counts
//!   derived once per arc by [`circular_edge_count`].
//! - Cubic interior points are sampled at uniform parameters with a count
//!   from the second-difference flatness bound — arithmetic plus square
//!   roots, owned here (kurbo does not sit on the deterministic path at
//!   all; [`crate::EVAL_SCHEMA_VERSION`] guards rule changes).

use alloc::vec::Vec;

use kurbo::Point;

use crate::len_u32;
use crate::profile::{Loop2, Profile2, SegKind};

/// Arithmetic and coordinate emission each consume rounding headroom. Below
/// this many ulps at the source-coordinate scale, reject instead of claiming a
/// chord bound that f64 output cannot reliably represent.
const REALIZATION_ULPS: f64 = 16.0;

/// Controls how finely curves are discretized.
///
/// The chord tolerance is an absolute mathematical chord-to-curve bound in
/// model units. A successful discretization uses enough edges to satisfy it;
/// an insufficient work budget is a typed error. This bound does not include
/// later coordinate quantization such as tessellation's f64-to-f32 boundary.
/// Callers choose it deliberately; the default is suitable for millimeter-unit
/// parametric spec geometry.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DiscretizePolicy {
    /// Maximum chord-to-curve deviation (sagitta), in model units.
    pub chord_tolerance: f64,
    /// Hard cap on edges produced per curve segment. Must be at least
    /// [`Self::min_arc_edges`].
    pub max_segment_edges: u32,
    /// Minimum edges per arc segment, regardless of tolerance. Must not
    /// exceed [`Self::max_segment_edges`].
    pub min_arc_edges: u32,
}

impl Default for DiscretizePolicy {
    fn default() -> Self {
        Self {
            chord_tolerance: 0.01,
            max_segment_edges: 4096,
            min_arc_edges: 4,
        }
    }
}

impl DiscretizePolicy {
    fn validate(&self) -> Result<(), DiscretizeError> {
        if !(self.chord_tolerance.is_finite() && self.chord_tolerance > 0.0) {
            return Err(DiscretizeError::InvalidTolerance);
        }
        if self.max_segment_edges == 0
            || self.min_arc_edges == 0
            || self.min_arc_edges > self.max_segment_edges
        {
            return Err(DiscretizeError::InvalidEdgeBounds);
        }
        Ok(())
    }
}

/// Edge-count constraints for one circular sweep.
///
/// Minimum topology, maximum work, and optional angular alignment are
/// independent choices. Set [`Self::edge_multiple`] to `1` when no alignment
/// is required, or for example `4` when a full circle must include cardinal
/// axes. The multiple is caller-selected and is never imposed on profile arcs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CircularEdgeConstraints {
    /// Minimum permitted edge count.
    pub min_edges: u32,
    /// Maximum permitted edge count (the finite work budget).
    pub max_edges: u32,
    /// Required edge-count multiple; `1` means no additional alignment.
    pub edge_multiple: u32,
}

impl CircularEdgeConstraints {
    /// Creates constraints without an additional edge-count multiple.
    #[must_use]
    pub const fn new(min_edges: u32, max_edges: u32) -> Self {
        Self {
            min_edges,
            max_edges,
            edge_multiple: 1,
        }
    }

    /// Requires the result to be a multiple of `edge_multiple`.
    ///
    /// A zero multiple is rejected by [`circular_edge_count`].
    #[must_use]
    pub const fn with_edge_multiple(mut self, edge_multiple: u32) -> Self {
        self.edge_multiple = edge_multiple;
        self
    }
}

/// One discretized loop: a point ring plus edge-to-segment provenance.
///
/// Edge `i` runs `points[i] -> points[(i + 1) % len]` and was produced by
/// source segment `edge_seg[i]` (an index into the source loop's segments).
/// Point `i` is an exact source endpoint precisely when
/// `edge_seg[i] != edge_seg[(i + len - 1) % len]`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretizedLoop {
    /// Ring points, without a duplicated closing point.
    pub points: Vec<[f64; 2]>,
    /// Source segment index for each edge, parallel to `points`.
    pub edge_seg: Vec<u32>,
}

/// A discretized profile: outer ring plus hole rings.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretizedProfile {
    /// The outer ring (counter-clockwise).
    pub outer: DiscretizedLoop,
    /// Hole rings (clockwise), in source order.
    pub holes: Vec<DiscretizedLoop>,
}

/// Typed discretization failure.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiscretizeError {
    /// The chord tolerance is zero, negative, or non-finite.
    InvalidTolerance,
    /// An edge-count bound is zero or the minimum exceeds the maximum.
    InvalidEdgeBounds,
    /// A circle radius or sweep is zero, negative, non-finite, or the sweep
    /// exceeds one full turn.
    InvalidCircularGeometry,
    /// The requested accuracy, topology, or alignment constraints need more
    /// edges than the finite work budget.
    ToleranceBudgetExceeded {
        /// Smallest edge count that satisfies accuracy, topology, and any
        /// requested edge-count multiple.
        required: u32,
        /// Maximum edge count permitted by the caller.
        maximum: u32,
    },
    /// The edge count required by the requested accuracy cannot fit in `u32`.
    EdgeCountOverflow,
    /// Finite inputs produced intermediate or output coordinates that cannot
    /// be represented reliably in f64.
    NumericLimit,
}

impl core::fmt::Display for DiscretizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTolerance => write!(f, "chord tolerance must be finite and positive"),
            Self::InvalidEdgeBounds => {
                write!(
                    f,
                    "edge-count bounds must be nonzero with minimum <= maximum"
                )
            }
            Self::InvalidCircularGeometry => write!(
                f,
                "circle radius and sweep must be finite and positive, with sweep <= tau"
            ),
            Self::ToleranceBudgetExceeded { required, maximum } => write!(
                f,
                "subdivision constraints require {required} edges but the budget allows {maximum}"
            ),
            Self::EdgeCountOverflow => {
                write!(f, "required edge count exceeds the u32 count domain")
            }
            Self::NumericLimit => write!(
                f,
                "curve cannot be discretized reliably within f64 numeric limits"
            ),
        }
    }
}

impl core::error::Error for DiscretizeError {}

/// Returns the smallest circular edge count that satisfies a chord tolerance
/// and explicit count constraints.
///
/// The count uses the cancellation-resistant identity
/// `acos(1 - x) = 2 asin(sqrt(x / 2))`. This remains stable when tolerance is
/// tiny relative to radius. A requested multiple rounds upward only; a work
/// budget never silently coarsens the result.
///
/// # Errors
///
/// Returns a typed error for invalid tolerance, circle geometry, constraints,
/// an unrepresentable required count, or a maximum below the required count.
pub fn circular_edge_count(
    radius: f64,
    sweep: f64,
    chord_tolerance: f64,
    constraints: CircularEdgeConstraints,
) -> Result<u32, DiscretizeError> {
    if !(chord_tolerance.is_finite() && chord_tolerance > 0.0) {
        return Err(DiscretizeError::InvalidTolerance);
    }
    if !(radius.is_finite()
        && radius > 0.0
        && sweep.is_finite()
        && sweep > 0.0
        && sweep <= core::f64::consts::TAU)
    {
        return Err(DiscretizeError::InvalidCircularGeometry);
    }
    if constraints.min_edges == 0
        || constraints.max_edges == 0
        || constraints.edge_multiple == 0
        || constraints.min_edges > constraints.max_edges
    {
        return Err(DiscretizeError::InvalidEdgeBounds);
    }

    // sqrt(tol / (2r)), factored so tol/r cannot underflow before sqrt.
    let ratio_root = libm::sqrt(chord_tolerance) / libm::sqrt(radius) / core::f64::consts::SQRT_2;
    if ratio_root == 0.0 {
        return Err(DiscretizeError::EdgeCountOverflow);
    }
    let needed = if ratio_root >= 1.0 {
        1.0
    } else {
        let per_edge_angle = 4.0 * libm::asin(ratio_root);
        libm::ceil(sweep / per_edge_angle)
    };
    if !needed.is_finite() || needed > f64::from(u32::MAX) {
        return Err(DiscretizeError::EdgeCountOverflow);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "needed is a finite nonnegative ceil within the u32 domain"
    )]
    let needed = needed as u32;
    let mut required = needed.max(1).max(constraints.min_edges);
    required = round_up_to_multiple(required, constraints.edge_multiple)
        .ok_or(DiscretizeError::EdgeCountOverflow)?;

    // Guard a count that landed infinitesimally below the exact threshold due
    // to floating rounding. Incrementing by the requested multiple preserves
    // alignment and makes successful outcomes conservative.
    while !circular_sagitta_within(radius, sweep, chord_tolerance, required) {
        required = required
            .checked_add(constraints.edge_multiple)
            .ok_or(DiscretizeError::EdgeCountOverflow)?;
    }
    if required > constraints.max_edges {
        return Err(DiscretizeError::ToleranceBudgetExceeded {
            required,
            maximum: constraints.max_edges,
        });
    }
    Ok(required)
}

fn round_up_to_multiple(value: u32, multiple: u32) -> Option<u32> {
    let remainder = value % multiple;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(multiple - remainder)
    }
}

fn circular_sagitta_within(radius: f64, sweep: f64, tolerance: f64, edges: u32) -> bool {
    // sqrt(sagitta) = sqrt(2r) * sin(sweep / 4n). Compare after dividing by
    // sqrt(2r) to avoid forming either 2r or a tiny tolerance/radius ratio.
    let allowed_sine = libm::sqrt(tolerance) / libm::sqrt(radius) / core::f64::consts::SQRT_2;
    libm::sin(sweep / (4.0 * f64::from(edges))) <= allowed_sine
}

/// Discretizes one loop.
///
/// # Errors
///
/// Fails on invalid policy values, an insufficient edge budget, count
/// overflow, or numeric limits that prevent reliable curve realization.
pub fn discretize_loop(
    source: &Loop2,
    policy: &DiscretizePolicy,
) -> Result<DiscretizedLoop, DiscretizeError> {
    policy.validate()?;
    let mut out = DiscretizedLoop {
        points: Vec::new(),
        edge_seg: Vec::new(),
    };
    for (index, (start, seg)) in source.iter_with_starts().enumerate() {
        let seg_index = len_u32(index);
        // Each segment contributes its start point plus interior points;
        // its exact endpoint is contributed as the next segment's start.
        push_point(&mut out, [start.x, start.y], seg_index);
        emit_kind_interior(&mut out, start, seg.to, &seg.kind, policy, seg_index)?;
    }
    Ok(out)
}

/// Discretizes a profile: outer ring plus holes, in source order.
///
/// # Errors
///
/// Fails under the same conditions as [`discretize_loop`].
pub fn discretize_profile(
    profile: &Profile2,
    policy: &DiscretizePolicy,
) -> Result<DiscretizedProfile, DiscretizeError> {
    let outer = discretize_loop(profile.outer(), policy)?;
    let holes = profile
        .holes()
        .iter()
        .map(|hole| discretize_loop(hole, policy))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DiscretizedProfile { outer, holes })
}

/// Emits a segment kind's interior points; policy segments discretize
/// their realization (one level — nesting is rejected at validation).
fn emit_kind_interior(
    out: &mut DiscretizedLoop,
    start: Point,
    to: Point,
    kind: &SegKind,
    policy: &DiscretizePolicy,
    seg_index: u32,
) -> Result<(), DiscretizeError> {
    match kind {
        SegKind::Line => Ok(()),
        SegKind::Arc { bulge } => emit_arc_interior(out, start, to, *bulge, policy, seg_index),
        SegKind::Cubic { c1, c2 } => {
            emit_cubic_interior(out, start, *c1, *c2, to, policy, seg_index)
        }
        SegKind::PolicyTo {
            policy: _,
            realized,
        } => emit_kind_interior(out, start, to, realized, policy, seg_index),
    }
}

fn push_point(out: &mut DiscretizedLoop, p: [f64; 2], seg: u32) {
    // Skip exact duplicates (an interior point can coincide with an
    // endpoint at coarse subdivisions); the ring stays clean.
    if out.points.last() == Some(&p) {
        return;
    }
    out.points.push(p);
    out.edge_seg.push(seg);
}

/// Emits an arc's interior points via libm-only trigonometry.
///
/// The subdivision count derives once from the sagitta bound; interior
/// point `k` is evaluated independently at angle `start + k * step` (no
/// accumulated increments), so precision does not drift with count.
fn emit_arc_interior(
    out: &mut DiscretizedLoop,
    from: Point,
    to: Point,
    bulge: f64,
    policy: &DiscretizePolicy,
    seg_index: u32,
) -> Result<(), DiscretizeError> {
    let output_start = out.points.len() - 1;
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let chord = libm::hypot(dx, dy);
    let half_chord = 0.5 * chord;
    let reciprocal_bulge = 1.0 / bulge;
    let signed_radius = half_chord * 0.5 * (bulge + reciprocal_bulge);
    let radius = signed_radius.abs();
    let offset = half_chord * 0.5 * (reciprocal_bulge - bulge);
    // sweep = 4 atan(bulge); signed, counter-clockwise positive.
    let sweep = 4.0 * libm::atan(bulge);
    if !dx.is_finite()
        || !dy.is_finite()
        || !chord.is_finite()
        || !radius.is_finite()
        || radius == 0.0
        || !offset.is_finite()
        || !sweep.is_finite()
        || sweep == 0.0
    {
        return Err(DiscretizeError::NumericLimit);
    }
    // Center: midpoint plus left normal scaled by (signed radius - sagitta).
    // Signed radius carries the bulge sign, which puts the center on the
    // correct side for both sweep directions and both minor/major arcs.
    let mid = [from.x + dx * 0.5, from.y + dy * 0.5];
    let normal = [-dy / chord, dx / chord];
    let center = [mid[0] + normal[0] * offset, mid[1] + normal[1] * offset];
    if center.iter().any(|coordinate| !coordinate.is_finite()) {
        return Err(DiscretizeError::NumericLimit);
    }
    let realizable_tolerance = coordinate_tolerance(
        policy.chord_tolerance,
        &[from.x, from.y, to.x, to.y, center[0], center[1]],
    )
    .ok_or(DiscretizeError::NumericLimit)?;
    let edges = circular_edge_count(
        radius,
        libm::fabs(sweep),
        realizable_tolerance,
        CircularEdgeConstraints::new(policy.min_arc_edges, policy.max_segment_edges),
    )?;
    let start_angle = libm::atan2(from.y - center[1], from.x - center[0]);
    let step = sweep / f64::from(edges);
    for k in 1..edges {
        let angle = start_angle + step * f64::from(k);
        let p = [
            center[0] + radius * libm::cos(angle),
            center[1] + radius * libm::sin(angle),
        ];
        if p.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(DiscretizeError::NumericLimit);
        }
        push_point(out, p, seg_index);
    }
    if !arc_chords_within(
        &out.points[output_start..],
        [to.x, to.y],
        center,
        radius,
        realizable_tolerance,
    ) {
        return Err(DiscretizeError::NumericLimit);
    }
    Ok(())
}

fn arc_chords_within(
    emitted: &[[f64; 2]],
    endpoint: [f64; 2],
    center: [f64; 2],
    radius: f64,
    tolerance: f64,
) -> bool {
    emitted
        .iter()
        .copied()
        .zip(
            emitted
                .iter()
                .copied()
                .skip(1)
                .chain(core::iter::once(endpoint)),
        )
        .all(|(from, to)| {
            // Work relative to the center. Re-forming a global midpoint can
            // discard precisely the low bits this check is meant to audit.
            let a = [from[0] - center[0], from[1] - center[1]];
            let b = [to[0] - center[0], to[1] - center[1]];
            let delta = [b[0] - a[0], b[1] - a[1]];
            let chord = libm::hypot(delta[0], delta[1]);
            if !chord.is_finite() || chord == 0.0 {
                return false;
            }
            let direction = [delta[0] / chord, delta[1] / chord];
            let projection = -(a[0] * direction[0] + a[1] * direction[1]);
            let closest = if projection <= 0.0 {
                a
            } else if projection >= chord {
                b
            } else {
                [
                    a[0] + direction[0] * projection,
                    a[1] + direction[1] * projection,
                ]
            };
            let radii = [
                libm::hypot(a[0], a[1]),
                libm::hypot(b[0], b[1]),
                libm::hypot(closest[0], closest[1]),
            ];
            projection.is_finite()
                && radii.iter().all(|value| value.is_finite())
                && (radii[0] - radius).abs() <= tolerance
                && (radii[1] - radius).abs() <= tolerance
                && radius - radii[2] <= tolerance
        })
}

fn coordinate_tolerance(tolerance: f64, coordinates: &[f64]) -> Option<f64> {
    let max_ulp = coordinates
        .iter()
        .copied()
        .map(float_ulp)
        .fold(0.0_f64, f64::max);
    let margin = REALIZATION_ULPS * max_ulp;
    let remaining = tolerance - margin;
    (max_ulp.is_finite() && remaining.is_finite() && remaining > 0.0).then_some(remaining)
}

fn float_ulp(value: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return f64::from_bits(1);
    }
    f64::from_bits(magnitude.to_bits() + 1) - magnitude
}

/// Emits a cubic's interior points by uniform-parameter sampling.
///
/// The subdivision count derives once from the classic second-difference
/// flatness bound (`n >= sqrt(3 d / (4 tol))`, `d` the largest control
/// second difference). If the required count exceeds the policy cap, the
/// operation fails typed — so work is bounded up front even for hostile
/// control points, and the math is arithmetic
/// plus one square root: bit-deterministic everywhere and independent of
/// kurbo's flattener.
fn emit_cubic_interior(
    out: &mut DiscretizedLoop,
    from: Point,
    c1: Point,
    c2: Point,
    to: Point,
    policy: &DiscretizePolicy,
    seg_index: u32,
) -> Result<(), DiscretizeError> {
    let realizable_tolerance = coordinate_tolerance(
        policy.chord_tolerance,
        &[from.x, from.y, c1.x, c1.y, c2.x, c2.y, to.x, to.y],
    )
    .ok_or(DiscretizeError::NumericLimit)?;
    let output_start = out.points.len() - 1;
    let d1 = [
        (from.x - c1.x) - (c1.x - c2.x),
        (from.y - c1.y) - (c1.y - c2.y),
    ];
    let d2 = [(c1.x - c2.x) - (c2.x - to.x), (c1.y - c2.y) - (c2.y - to.y)];
    let d = libm::hypot(d1[0], d1[1]).max(libm::hypot(d2[0], d2[1]));
    if !d.is_finite() {
        return Err(DiscretizeError::NumericLimit);
    }
    let root_bound = libm::sqrt(d) * (libm::sqrt(3.0) * 0.5);
    let needed = libm::ceil(root_bound / libm::sqrt(realizable_tolerance));
    if !needed.is_finite() || needed > f64::from(u32::MAX) {
        return Err(DiscretizeError::EdgeCountOverflow);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "needed is a finite nonnegative ceil within the u32 domain"
    )]
    let mut edges = (needed as u32).max(1);
    while root_bound / f64::from(edges) > libm::sqrt(realizable_tolerance) {
        edges = edges
            .checked_add(1)
            .ok_or(DiscretizeError::EdgeCountOverflow)?;
    }
    if edges > policy.max_segment_edges {
        return Err(DiscretizeError::ToleranceBudgetExceeded {
            required: edges,
            maximum: policy.max_segment_edges,
        });
    }
    for k in 1..edges {
        let t = f64::from(k) / f64::from(edges);
        // Bernstein-basis cubic Bézier evaluation: pure arithmetic.
        let mt = 1.0 - t;
        let a = mt * mt * mt;
        let b = 3.0 * mt * mt * t;
        let c = 3.0 * mt * t * t;
        let e = t * t * t;
        let p = [
            a * from.x + b * c1.x + c * c2.x + e * to.x,
            a * from.y + b * c1.y + c * c2.y + e * to.y,
        ];
        if p.iter().any(|coordinate| !coordinate.is_finite()) {
            return Err(DiscretizeError::NumericLimit);
        }
        push_point(out, p, seg_index);
    }
    if out.points.len() - output_start != edges as usize
        || !cubic_chords_within(
            &out.points[output_start..],
            [from.x, from.y],
            [c1.x, c1.y],
            [c2.x, c2.y],
            [to.x, to.y],
            d,
            realizable_tolerance,
            edges,
        )
    {
        return Err(DiscretizeError::NumericLimit);
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cubic's four points, emitted samples, bound, and count are the audited contract"
)]
fn cubic_chords_within(
    emitted: &[[f64; 2]],
    from: [f64; 2],
    c1: [f64; 2],
    c2: [f64; 2],
    to: [f64; 2],
    second_difference: f64,
    tolerance: f64,
    edges: u32,
) -> bool {
    let local_c1 = [c1[0] - from[0], c1[1] - from[1]];
    let local_c2 = [c2[0] - from[0], c2[1] - from[1]];
    let local_to = [to[0] - from[0], to[1] - from[1]];
    if local_c1
        .iter()
        .chain(&local_c2)
        .chain(&local_to)
        .any(|value| !value.is_finite())
    {
        return false;
    }
    let mut max_emission_error = 0.0_f64;
    for k in 0..=edges {
        let actual = if k == edges { to } else { emitted[k as usize] };
        let actual_local = [actual[0] - from[0], actual[1] - from[1]];
        let t = f64::from(k) / f64::from(edges);
        let ideal_local = cubic_point_at([0.0, 0.0], local_c1, local_c2, local_to, t);
        let error = libm::hypot(
            actual_local[0] - ideal_local[0],
            actual_local[1] - ideal_local[1],
        );
        if !error.is_finite() {
            return false;
        }
        max_emission_error = max_emission_error.max(error);
    }
    let edges = f64::from(edges);
    let flatness_bound = 0.75 * second_difference / (edges * edges);
    flatness_bound.is_finite() && flatness_bound + max_emission_error <= tolerance
}

fn cubic_point_at(from: [f64; 2], c1: [f64; 2], c2: [f64; 2], to: [f64; 2], t: f64) -> [f64; 2] {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let c = 3.0 * mt * t * t;
    let d = t * t * t;
    [
        a * from[0] + b * c1[0] + c * c2[0] + d * to[0],
        a * from[1] + b * c1[1] + c * c2[1] + d * to[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Profile2, Seg2, SegTag};
    use alloc::vec;

    fn square() -> Loop2 {
        Loop2::new(vec![
            Seg2::line((1.0, 0.0)).tagged(SegTag(0)),
            Seg2::line((1.0, 1.0)).tagged(SegTag(1)),
            Seg2::line((0.0, 1.0)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("valid square")
    }

    #[test]
    fn polyline_loops_discretize_to_their_vertices() {
        let d = discretize_loop(&square(), &DiscretizePolicy::default()).expect("discretizes");
        assert_eq!(
            d.points,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        );
        assert_eq!(d.edge_seg, vec![0, 1, 2, 3]);
    }

    #[test]
    fn arc_endpoints_are_exact_and_sagitta_bounded() {
        // Quarter circle of radius 2 from (2, 0) to (0, 2): bulge =
        // tan(pi/8).
        let bulge = libm::tan(core::f64::consts::FRAC_PI_8);
        let quarter = Loop2::new(vec![
            Seg2::line((2.0, 0.0)),
            Seg2::arc((0.0, 2.0), bulge),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid pie slice");
        let policy = DiscretizePolicy {
            chord_tolerance: 1e-3,
            ..DiscretizePolicy::default()
        };
        let d = discretize_loop(&quarter, &policy).expect("discretizes");

        // Exact endpoints survive.
        assert!(d.points.contains(&[2.0, 0.0]));
        assert!(d.points.contains(&[0.0, 2.0]));

        // Every arc-owned point lies on the radius-2 circle about the
        // origin to within floating error, and every arc edge's midpoint
        // sagitta respects the tolerance.
        let arc_seg = 1_u32;
        let n = d.points.len();
        for i in 0..n {
            if d.edge_seg[i] == arc_seg {
                let a = d.points[i];
                let b = d.points[(i + 1) % n];
                for p in [a, b] {
                    let r = libm::sqrt(p[0] * p[0] + p[1] * p[1]);
                    assert!((r - 2.0).abs() < 1e-9, "point {p:?} off circle");
                }
                let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                let sag = 2.0 - libm::sqrt(mid[0] * mid[0] + mid[1] * mid[1]);
                assert!(sag <= policy.chord_tolerance * 1.0001, "sagitta {sag}");
            }
        }
    }

    #[test]
    fn bulge_sign_selects_deviation_side() {
        // Positive bulge = counter-clockwise sweep = deviation to the
        // RIGHT of the chord direction; negative bulge deviates left.
        // Arc from (0,0) to (2,0): positive dips below (right of +x),
        // negative rises above.
        let dip = Loop2::new(vec![
            Seg2::arc((2.0, 0.0), 0.5),
            Seg2::line((2.0, 1.0)),
            Seg2::line((0.0, 1.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let d = discretize_loop(&dip, &DiscretizePolicy::default()).expect("discretizes");
        let n = d.points.len();
        let arc_dips = (0..n).any(|i| d.edge_seg[i] == 0 && d.points[i][1] < 0.0);
        assert!(arc_dips, "positive bulge deviates right (below +x chord)");

        let rise = Loop2::new(vec![
            Seg2::arc((2.0, 0.0), -0.5),
            Seg2::line((2.0, -1.0)),
            Seg2::line((0.0, -1.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let d = discretize_loop(&rise, &DiscretizePolicy::default()).expect("discretizes");
        let n = d.points.len();
        let arc_rises = (0..n).any(|i| d.edge_seg[i] == 0 && d.points[i][1] > 0.0);
        assert!(arc_rises, "negative bulge deviates left (above +x chord)");
    }

    #[test]
    fn cubic_interior_points_are_emitted() {
        let s_curve = Loop2::new(vec![
            Seg2::cubic((2.0, 0.0), (0.5, 1.0), (1.5, -1.0)),
            Seg2::line((2.0, 1.5)),
            Seg2::line((0.0, 1.5)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let d = discretize_loop(&s_curve, &DiscretizePolicy::default()).expect("discretizes");
        assert!(d.points.len() > 4, "cubic contributes interior points");
        // Exact endpoints of the cubic survive as ring points.
        assert!(d.points.contains(&[0.0, 0.0]));
        assert!(d.points.contains(&[2.0, 0.0]));
    }

    #[test]
    fn discretization_is_deterministic() {
        let bulge = libm::tan(core::f64::consts::FRAC_PI_8);
        let shape = Loop2::new(vec![
            Seg2::arc((3.0, 0.0), bulge),
            Seg2::cubic((3.0, 2.0), (3.5, 0.5), (2.5, 1.5)),
            Seg2::line((0.0, 2.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid loop");
        let policy = DiscretizePolicy::default();
        let a = discretize_loop(&shape, &policy).expect("first");
        let b = discretize_loop(&shape, &policy).expect("second");
        assert_eq!(a, b);
    }

    #[test]
    fn golden_arc_interior_bits() {
        // Determinism pin: the first interior point of a known arc, as raw
        // f64 bit patterns. A change here means libm output or the
        // subdivision rule changed — bump EVAL_SCHEMA_VERSION deliberately.
        let half = Loop2::new(vec![Seg2::arc((1.0, 0.0), 1.0), Seg2::arc((0.0, 0.0), 1.0)])
            .expect("two-arc circle");
        let policy = DiscretizePolicy {
            chord_tolerance: 0.05,
            min_arc_edges: 4,
            max_segment_edges: 4096,
        };
        let d = discretize_loop(&half, &policy).expect("discretizes");
        // First interior point of the first half-circle arc.
        let p = d.points[1];
        assert_eq!(
            [p[0].to_bits(), p[1].to_bits()],
            [0x3FC2_BEC3_3301_8864, 0xBFD6_A09E_667F_3BCC],
            "libm arc math output changed; bump EVAL_SCHEMA_VERSION"
        );
    }

    #[test]
    fn policy_validation() {
        let square = square();
        let bad_tol = DiscretizePolicy {
            chord_tolerance: 0.0,
            ..DiscretizePolicy::default()
        };
        assert_eq!(
            discretize_loop(&square, &bad_tol),
            Err(DiscretizeError::InvalidTolerance)
        );
        let bad_edges = DiscretizePolicy {
            max_segment_edges: 0,
            ..DiscretizePolicy::default()
        };
        assert_eq!(
            discretize_loop(&square, &bad_edges),
            Err(DiscretizeError::InvalidEdgeBounds)
        );
    }

    #[test]
    fn inverted_edge_bounds_return_a_typed_error() {
        let bulge = libm::tan(core::f64::consts::FRAC_PI_8);
        let arc_loop = Loop2::new(vec![
            Seg2::line((2.0, 0.0)),
            Seg2::arc((0.0, 2.0), bulge),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid arc loop");
        let policy = DiscretizePolicy {
            min_arc_edges: 8,
            max_segment_edges: 4,
            ..DiscretizePolicy::default()
        };

        assert_eq!(
            discretize_loop(&arc_loop, &policy),
            Err(DiscretizeError::InvalidEdgeBounds)
        );
    }

    #[test]
    fn edge_bounds_are_validated_before_segment_discretization() {
        let line_loop = square();
        let arc_loop = Loop2::new(vec![
            Seg2::line((2.0, 0.0)),
            Seg2::arc((0.0, 2.0), libm::tan(core::f64::consts::FRAC_PI_8)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid arc loop");
        let arc_profile = Profile2::simple(arc_loop.clone()).expect("valid arc profile");

        for policy in [
            DiscretizePolicy {
                min_arc_edges: 0,
                ..DiscretizePolicy::default()
            },
            DiscretizePolicy {
                max_segment_edges: 0,
                ..DiscretizePolicy::default()
            },
            DiscretizePolicy {
                min_arc_edges: 8,
                max_segment_edges: 4,
                ..DiscretizePolicy::default()
            },
        ] {
            assert_eq!(
                discretize_loop(&line_loop, &policy),
                Err(DiscretizeError::InvalidEdgeBounds)
            );
            assert_eq!(
                discretize_loop(&arc_loop, &policy),
                Err(DiscretizeError::InvalidEdgeBounds)
            );
            assert_eq!(
                discretize_profile(&arc_profile, &policy),
                Err(DiscretizeError::InvalidEdgeBounds)
            );
        }

        let equal = DiscretizePolicy {
            chord_tolerance: 0.1,
            min_arc_edges: 4,
            max_segment_edges: 4,
        };
        assert!(discretize_loop(&line_loop, &equal).is_ok());
        assert!(discretize_loop(&arc_loop, &equal).is_ok());
        assert!(discretize_profile(&arc_profile, &equal).is_ok());
    }

    #[test]
    fn insufficient_arc_budget_does_not_claim_tolerance() {
        let semicircle = Loop2::new(vec![Seg2::arc((1.0, 0.0), 1.0), Seg2::line((-1.0, 0.0))])
            .expect("valid semicircle");
        let policy = DiscretizePolicy {
            chord_tolerance: 1.0e-9,
            min_arc_edges: 1,
            max_segment_edges: 4,
        };

        assert!(
            matches!(
                discretize_loop(&semicircle, &policy),
                Err(DiscretizeError::ToleranceBudgetExceeded { maximum: 4, .. })
            ),
            "four chords cannot satisfy a 1e-9 sagitta bound on a unit-radius semicircle"
        );
    }

    #[test]
    fn circular_count_is_stable_monotone_and_caller_aligned() {
        let constraints = CircularEdgeConstraints::new(8, u32::MAX).with_edge_multiple(4);
        let representative = circular_edge_count(1.0, core::f64::consts::TAU, 5.0e-4, constraints)
            .expect("representative circle count");
        assert_eq!(representative, 100);
        assert_eq!(representative % 4, 0, "caller alignment is honored");

        let mut previous = 0;
        for tolerance in [1.0, 0.1, 1.0e-3, 1.0e-6, 1.0e-12] {
            let edges = circular_edge_count(2.0, core::f64::consts::TAU, tolerance, constraints)
                .expect("representable count");
            assert!(
                edges >= previous,
                "tightening tolerance reduced {previous} edges to {edges}"
            );
            previous = edges;
        }
    }

    #[test]
    fn circular_count_reports_budget_overflow_and_invalid_inputs() {
        assert!(matches!(
            circular_edge_count(
                1.0,
                core::f64::consts::PI,
                1.0e-9,
                CircularEdgeConstraints::new(1, 4),
            ),
            Err(DiscretizeError::ToleranceBudgetExceeded { maximum: 4, .. })
        ));
        assert_eq!(
            circular_edge_count(
                1.0,
                core::f64::consts::TAU,
                1.0e-20,
                CircularEdgeConstraints::new(1, u32::MAX),
            ),
            Err(DiscretizeError::EdgeCountOverflow)
        );
        for (radius, sweep, tolerance) in [
            (0.0, 1.0, 0.1),
            (1.0, 0.0, 0.1),
            (1.0, core::f64::consts::TAU + 0.1, 0.1),
            (f64::INFINITY, 1.0, 0.1),
        ] {
            assert_eq!(
                circular_edge_count(
                    radius,
                    sweep,
                    tolerance,
                    CircularEdgeConstraints::new(1, 100),
                ),
                Err(DiscretizeError::InvalidCircularGeometry)
            );
        }
        assert_eq!(
            circular_edge_count(
                1.0,
                1.0,
                0.1,
                CircularEdgeConstraints::new(1, 100).with_edge_multiple(0),
            ),
            Err(DiscretizeError::InvalidEdgeBounds)
        );
    }

    #[test]
    fn cubic_deviation_is_bounded_and_straight_cubics_need_one_edge() {
        let curve = Loop2::new(vec![
            Seg2::cubic((2.0, 0.0), (0.5, 1.0), (1.5, -1.0)).tagged(SegTag(7)),
            Seg2::line((2.0, 2.0)),
            Seg2::line((0.0, 2.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid cubic loop");
        let policy = DiscretizePolicy {
            chord_tolerance: 1.0e-4,
            max_segment_edges: 4096,
            ..DiscretizePolicy::default()
        };
        let discretized = discretize_loop(&curve, &policy).expect("cubic fits budget");
        let edges = discretized
            .edge_seg
            .iter()
            .take_while(|&&segment| segment == 0)
            .count();
        assert!(edges > 1);
        for edge in 0..edges {
            let from = discretized.points[edge];
            let to = discretized.points[edge + 1];
            for sample in 1..10 {
                let u = f64::from(sample) / 10.0;
                let t = (edge as f64 + u) / edges as f64;
                let curve_point = cubic_point([0.0, 0.0], [0.5, 1.0], [1.5, -1.0], [2.0, 0.0], t);
                let chord_point = [
                    from[0] * (1.0 - u) + to[0] * u,
                    from[1] * (1.0 - u) + to[1] * u,
                ];
                let deviation = libm::hypot(
                    curve_point[0] - chord_point[0],
                    curve_point[1] - chord_point[1],
                );
                assert!(
                    deviation <= policy.chord_tolerance,
                    "cubic deviation {deviation} exceeded {}",
                    policy.chord_tolerance
                );
            }
        }

        let straight = Loop2::new(vec![
            Seg2::cubic((3.0, 0.0), (1.0, 0.0), (2.0, 0.0)),
            Seg2::line((3.0, 1.0)),
            Seg2::line((0.0, 1.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid straight cubic loop");
        let one_edge = DiscretizePolicy {
            max_segment_edges: 1,
            min_arc_edges: 1,
            ..DiscretizePolicy::default()
        };
        let discretized = discretize_loop(&straight, &one_edge).expect("straight cubic");
        assert_eq!(
            discretized
                .edge_seg
                .iter()
                .filter(|&&segment| segment == 0)
                .count(),
            1
        );
    }

    #[test]
    fn cubic_budget_and_numeric_limits_are_typed() {
        let curve = Loop2::new(vec![
            Seg2::cubic((2.0, 0.0), (0.5, 1.0), (1.5, -1.0)),
            Seg2::line((2.0, 2.0)),
            Seg2::line((0.0, 2.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid cubic loop");
        let policy = DiscretizePolicy {
            chord_tolerance: 1.0e-9,
            max_segment_edges: 4,
            min_arc_edges: 1,
        };
        assert!(matches!(
            discretize_loop(&curve, &policy),
            Err(DiscretizeError::ToleranceBudgetExceeded { maximum: 4, .. })
        ));

        let extreme_arc = Loop2::new(vec![
            Seg2::arc((f64::MAX, 0.0), 1.0),
            Seg2::line((-f64::MAX, 0.0)),
        ])
        .expect("finite endpoints are structurally valid");
        assert_eq!(
            discretize_loop(&extreme_arc, &DiscretizePolicy::default()),
            Err(DiscretizeError::NumericLimit)
        );

        for (center, tolerance, maximum) in [
            (1.0e8, 1.0e-7, 100_000),
            (1.0e8, 1.0e-9, 100_000),
            (1.0e12, 1.0e-9, 100_000),
        ] {
            let translated = Loop2::new(vec![
                Seg2::arc((center + 1.0, 0.0), 1.0),
                Seg2::line((center - 1.0, 0.0)),
            ])
            .expect("finite translated semicircle");
            let policy = DiscretizePolicy {
                chord_tolerance: tolerance,
                min_arc_edges: 1,
                max_segment_edges: maximum,
            };
            assert_eq!(
                discretize_loop(&translated, &policy),
                Err(DiscretizeError::NumericLimit),
                "origin {center:e} cannot realize tolerance {tolerance:e} reliably"
            );
        }

        for center in [1.0e8, 1.0e12] {
            let translated = Loop2::new(vec![
                Seg2::cubic((center + 1.0, 0.0), (center, 1.0), (center + 1.0, 1.0)),
                Seg2::line((center + 1.0, 2.0)),
                Seg2::line((center, 2.0)),
                Seg2::line((center, 0.0)),
            ])
            .expect("finite translated cubic");
            let policy = DiscretizePolicy {
                chord_tolerance: 1.0e-9,
                min_arc_edges: 1,
                max_segment_edges: 100_000,
            };
            assert_eq!(
                discretize_loop(&translated, &policy),
                Err(DiscretizeError::NumericLimit),
                "translated cubic cannot realize a sub-ulp tolerance at {center:e}"
            );
        }
    }

    fn cubic_point(from: [f64; 2], c1: [f64; 2], c2: [f64; 2], to: [f64; 2], t: f64) -> [f64; 2] {
        let mt = 1.0 - t;
        let weights = [mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t];
        [
            weights[0] * from[0] + weights[1] * c1[0] + weights[2] * c2[0] + weights[3] * to[0],
            weights[0] * from[1] + weights[1] * c1[1] + weights[2] * c2[1] + weights[3] * to[1],
        ]
    }
}
