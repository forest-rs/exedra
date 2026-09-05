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
//!   derived once per arc.
//! - Cubic interior points are sampled at uniform parameters with a count
//!   from the second-difference flatness bound — arithmetic plus square
//!   roots, owned here (kurbo does not sit on the deterministic path at
//!   all; [`crate::EVAL_SCHEMA_VERSION`] guards rule changes).

use alloc::vec::Vec;

use kurbo::Point;

use crate::len_u32;
use crate::profile::{Loop2, Profile2, SegKind};

/// Controls how finely curves are discretized.
///
/// The chord tolerance is an absolute sagitta bound in model units: no
/// discretized edge deviates from its source curve by more than this.
/// Callers (evaluation policies) choose it deliberately; the default is
/// suitable for millimeter-unit parametric spec geometry.
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
        }
    }
}

impl core::error::Error for DiscretizeError {}

/// Discretizes one loop.
///
/// # Errors
///
/// Fails only on invalid policy values; valid loops always discretize.
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
        emit_kind_interior(&mut out, start, seg.to, &seg.kind, policy, seg_index);
    }
    Ok(out)
}

/// Discretizes a profile: outer ring plus holes, in source order.
///
/// # Errors
///
/// Fails only on invalid policy values.
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
) {
    match kind {
        SegKind::Line => {}
        SegKind::Arc { bulge } => {
            emit_arc_interior(out, start, to, *bulge, policy, seg_index);
        }
        SegKind::Cubic { c1, c2 } => {
            emit_cubic_interior(out, start, *c1, *c2, to, policy, seg_index);
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
) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let half_chord = 0.5 * libm::sqrt(dx * dx + dy * dy);
    let sagitta = bulge * half_chord;
    let radius = ((half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta)).abs();
    // sweep = 4 atan(bulge); signed, counter-clockwise positive.
    let sweep = 4.0 * libm::atan(bulge);
    debug_assert!(radius.is_finite(), "validated bulges give finite radii");

    // Edges needed so each sub-arc's sagitta stays within tolerance:
    // s = r (1 - cos(theta / 2n))  =>  n >= theta / (2 acos(1 - tol / r)).
    let edges = if policy.chord_tolerance >= 2.0 * radius {
        1
    } else {
        let per_edge = 2.0 * libm::acos(1.0 - policy.chord_tolerance / radius);
        let needed = libm::ceil(libm::fabs(sweep) / per_edge);
        if needed >= f64::from(policy.max_segment_edges) {
            policy.max_segment_edges
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "needed is a ceil of a finite positive value below the u32 cap"
            )]
            {
                needed as u32
            }
        }
    };
    let edges = edges.clamp(policy.min_arc_edges, policy.max_segment_edges);

    // Center: midpoint plus left normal scaled by (signed radius - sagitta).
    // Signed radius carries the bulge sign, which puts the center on the
    // correct side for both sweep directions and both minor/major arcs.
    let signed_radius = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta);
    let mid = [(from.x + to.x) * 0.5, (from.y + to.y) * 0.5];
    let inv_chord = 0.5 / half_chord;
    let normal = [-dy * inv_chord, dx * inv_chord];
    let offset = signed_radius - sagitta;
    let center = [mid[0] + normal[0] * offset, mid[1] + normal[1] * offset];
    let start_angle = libm::atan2(from.y - center[1], from.x - center[0]);
    let step = sweep / f64::from(edges);
    for k in 1..edges {
        let angle = start_angle + step * f64::from(k);
        let p = [
            center[0] + radius * libm::cos(angle),
            center[1] + radius * libm::sin(angle),
        ];
        push_point(out, p, seg_index);
    }
}

/// Emits a cubic's interior points by uniform-parameter sampling.
///
/// The subdivision count derives once from the classic second-difference
/// flatness bound (`n >= sqrt(3 d / (4 tol))`, `d` the largest control
/// second difference), clamped by the policy cap — so the work is bounded
/// up front even for hostile control points, and the math is arithmetic
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
) {
    let d1 = [from.x - 2.0 * c1.x + c2.x, from.y - 2.0 * c1.y + c2.y];
    let d2 = [c1.x - 2.0 * c2.x + to.x, c1.y - 2.0 * c2.y + to.y];
    let d =
        libm::sqrt(d1[0] * d1[0] + d1[1] * d1[1]).max(libm::sqrt(d2[0] * d2[0] + d2[1] * d2[1]));
    let needed = libm::ceil(libm::sqrt(3.0 * d / (4.0 * policy.chord_tolerance)));
    let edges = if needed.is_finite() && needed >= 1.0 {
        if needed >= f64::from(policy.max_segment_edges) {
            policy.max_segment_edges
        } else {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "needed is a finite positive ceil below the u32 cap"
            )]
            {
                needed as u32
            }
        }
    } else {
        policy.max_segment_edges
    };
    for k in 1..edges {
        let t = f64::from(k) / f64::from(edges);
        // Horner-form cubic Bézier evaluation: pure arithmetic.
        let mt = 1.0 - t;
        let a = mt * mt * mt;
        let b = 3.0 * mt * mt * t;
        let c = 3.0 * mt * t * t;
        let e = t * t * t;
        let p = [
            a * from.x + b * c1.x + c * c2.x + e * to.x,
            a * from.y + b * c1.y + c * c2.y + e * to.y,
        ];
        push_point(out, p, seg_index);
    }
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
            min_arc_edges: 4,
            max_segment_edges: 4,
            ..DiscretizePolicy::default()
        };
        assert!(discretize_loop(&line_loop, &equal).is_ok());
        assert!(discretize_loop(&arc_loop, &equal).is_ok());
        assert!(discretize_profile(&arc_profile, &equal).is_ok());
    }
}
