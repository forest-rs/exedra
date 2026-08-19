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
//! - Cubic segments flatten through [`kurbo::flatten`], which is
//!   sqrt/arithmetic only (audited: deterministic under the pinned kurbo
//!   version; [`crate::EVAL_SCHEMA_VERSION`] guards upgrades).

use alloc::vec::Vec;

use kurbo::{PathEl, Point};

use crate::len_u32;
use crate::profile::{Loop2, Profile2, Seg2, SegKind};

/// Controls how finely curves are discretized.
///
/// The chord tolerance is an absolute sagitta bound in model units: no
/// discretized edge deviates from its source curve by more than this.
/// Callers (evaluation policies) choose it deliberately; the default is
/// suitable for millimeter-unit catalog geometry.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DiscretizePolicy {
    /// Maximum chord-to-curve deviation (sagitta), in model units.
    pub chord_tolerance: f64,
    /// Hard cap on edges produced per curve segment.
    pub max_segment_edges: u32,
    /// Minimum edges per arc segment, regardless of tolerance.
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
        if self.max_segment_edges == 0 || self.min_arc_edges == 0 {
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
    /// An edge-count bound is zero.
    InvalidEdgeBounds,
}

impl core::fmt::Display for DiscretizeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidTolerance => write!(f, "chord tolerance must be finite and positive"),
            Self::InvalidEdgeBounds => write!(f, "edge-count bounds must be nonzero"),
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
        match seg.kind {
            SegKind::Line => {}
            SegKind::Arc { bulge } => {
                emit_arc_interior(&mut out, start, seg, bulge, policy, seg_index);
            }
            SegKind::Cubic { c1, c2 } => {
                emit_cubic_interior(&mut out, start, c1, c2, seg.to, policy, seg_index);
            }
        }
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
    seg: &Seg2,
    bulge: f64,
    policy: &DiscretizePolicy,
    seg_index: u32,
) {
    let to = seg.to;
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

/// Emits a cubic's interior points via kurbo's flattener (sqrt-only math).
fn emit_cubic_interior(
    out: &mut DiscretizedLoop,
    from: Point,
    c1: Point,
    c2: Point,
    to: Point,
    policy: &DiscretizePolicy,
    seg_index: u32,
) {
    let elements = [PathEl::MoveTo(from), PathEl::CurveTo(c1, c2, to)];
    let mut interior: Vec<[f64; 2]> = Vec::new();
    kurbo::flatten(elements, policy.chord_tolerance, |el| {
        if let PathEl::LineTo(p) = el {
            interior.push([p.x, p.y]);
        }
    });
    // The flattener's last point is the exact endpoint, which the next
    // segment contributes as its start; drop it here.
    if let Some(last) = interior.pop()
        && last != [to.x, to.y]
    {
        // Defensive: kurbo always ends at the exact endpoint; if that ever
        // changes, keep the point rather than distort the curve.
        interior.push(last);
    }
    let cap = policy.max_segment_edges as usize;
    interior.truncate(cap.saturating_sub(1));
    for p in interior {
        push_point(out, p, seg_index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Seg2, SegTag};
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
}
