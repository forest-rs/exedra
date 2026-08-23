// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Profile offsets: clearance geometry derived from one nominal profile.
//!
//! [`Profile2::offset`] grows or shrinks a profile by a finite signed
//! distance. A **positive** distance grows the material: the outer loop
//! moves outward and every hole shrinks. A **negative** distance does the
//! reverse. Both sides of a fit can therefore be derived from a single
//! nominal interface profile — the inserted side keeps it, the receiving
//! side takes its clearance offset — instead of being two hand-built
//! profiles that agree only by discipline.
//!
//! ## Exact path versus fitted path
//!
//! - **Lines and bulge arcs offset exactly.** A line becomes a parallel
//!   line. An arc becomes a *concentric* arc: same center, same sweep, and
//!   a radius moved by the offset distance (grown when the arc bulges away
//!   from the material, shrunk when it bulges into it). Nothing is
//!   flattened, no curve is refitted, and the arithmetic is libm-only, so
//!   this path is bit-identical on every platform.
//! - **Cubics are fitted.** A cubic offsets through kurbo's `offset_cubic`
//!   at [`CUBIC_OFFSET_TOLERANCE_RATIO`] relative tolerance and comes back
//!   as one or more cubics. kurbo is the only curve-math engine here; this
//!   crate owns no parallel offset mathematics. kurbo's offset fitting uses
//!   `atan2`, `sin`, and `cos` that dispatch to `std` when the `std`
//!   feature is unified in by any crate in the build, so **the fitted path
//!   is not covered by this crate's cross-platform bit-identity
//!   contract**; the exact path is. Joinery profiles are overwhelmingly
//!   rectilinear and rounded-rectilinear, and the exact path covers them.
//!
//! ## Corners
//!
//! Where adjacent offset segments no longer meet, [`CornerPolicy`] decides
//! what fills the gap: a round arc of the offset radius, or a sharp miter
//! bounded by an explicit limit. Where they overlap instead — the inside of
//! a turn — the two offset curves are trimmed back to their intersection.
//! Trimming is analytic and local: line/line, line/arc, and arc/arc only.
//! A corner that needs trimming next to a fitted cubic is rejected with
//! [`ProfileError::OffsetCornerUnsupported`] rather than approximated.
//!
//! ## Rejection, never repair
//!
//! Consistent with the crate's no-auto-repair rule, a degenerate result is
//! a typed error, never a quietly patched profile. After an offset the
//! result is checked, in this order, for: structural constructibility
//! ([`ProfileError::OffsetLoopDegenerate`]), self-intersection or
//! self-touching ([`ProfileError::OffsetSelfIntersects`]), lost winding
//! ([`ProfileError::OffsetLoopDegenerate`] again), material collapse — any part
//! of the result closer to its source loop than the offset distance, which
//! is how a consumed hole shows up
//! ([`ProfileError::OffsetUndercut`]) — and contact between a hole and the
//! outer loop ([`ProfileError::OffsetLoopContact`]). Self-intersection is
//! *detected and rejected*; no loop trimming or offset-curve cleanup
//! happens.
//!
//! ## Provenance
//!
//! [`SegTag`]s travel with their segments on the
//! exact path, where the mapping is one-to-one: an offset line is still one
//! line, an offset arc still one arc, and trimming or mitering only moves
//! their endpoints. The mapping is *not* one-to-one in two places, both
//! documented here rather than silently: a fitted cubic can become several
//! cubics, and each of them carries the source segment's tag (one-to-many);
//! segments inserted to fill a corner belong to no single source segment
//! and carry no tag.
//!
//! An offset profile is a plain segment profile like any other, so it
//! content-hashes through the ordinary [`CanonBytes`](crate::profile::CanonBytes)
//! encoding and needs no IR node and no [`EVAL_SCHEMA_VERSION`](crate::EVAL_SCHEMA_VERSION)
//! change.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveDeriv, PathEl, Point, Shape, Vec2};

use crate::ir::PolicyId;
use crate::profile::{
    Loop2, Profile2, ProfileError, Seg2, SegKind, SegTag, bulge_arc_center_radius,
    distance_to_ring, flattened_ring, ring_self_intersects, rings_intersect,
};

/// Relative tolerance for fitting the offset of a cubic segment.
///
/// The absolute tolerance is this ratio times a scale taken from the
/// source cubic's bounding box plus the offset distance, which keeps the
/// fitted segment count scale-independent.
pub const CUBIC_OFFSET_TOLERANCE_RATIO: f64 = 1e-9;

/// Relative tolerance for the flattening behind the result checks.
///
/// Coarser than the fitting tolerance on purpose: the checks look for gross
/// failures — a loop folded through itself, a hole eaten away — and the
/// pairwise edge tests are quadratic in the vertex count, so a nanometric
/// ring would cost far more than it detects. Contacts finer than this
/// relative scale are not distinguished from contact-free.
const RING_TOLERANCE_RATIO: f64 = 1e-6;

/// Relative slack allowed before the result counts as undercutting.
///
/// Two orders of magnitude above the check flattening tolerance, so chord
/// sag and cubic fitting error never masquerade as collapsed material.
const UNDERCUT_SLACK_RATIO: f64 = 1e-4;

/// Turn angle, in radians, below which a corner counts as tangent
/// continuous.
///
/// Junctions that are smooth by construction — a rounded rectangle's
/// line-to-arc joints — reach this test with a turn of a few times
/// [`f64::EPSILON`], because a derived arc center is only correct to
/// rounding. Snapping them keeps the offset of a smooth profile smooth,
/// and a real corner below a nanoradian is noise either way.
const TANGENT_EPSILON: f64 = 1e-9;

/// What fills a corner where adjacent offset segments no longer meet.
///
/// A policy only applies on the *outside* of a turn, where offsetting opens
/// a gap. On the inside of a turn the offset curves overlap and are trimmed
/// to their intersection regardless of policy.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CornerPolicy {
    /// Fill the gap with a circular arc of the offset radius centered on
    /// the source vertex, so every point of the result stays exactly the
    /// offset distance from the source.
    Round,
    /// Fill the gap with a sharp corner: the two offset segments are
    /// carried to the intersection of their tangent lines.
    Miter {
        /// Maximum ratio of miter length to offset distance, at least
        /// `1.0`. A corner needing a longer miter is rejected with
        /// [`ProfileError::OffsetMiterLimitExceeded`]; unlike a stroker,
        /// this operation never quietly bevels, because a bevel would
        /// remove clearance the caller asked for.
        limit: f64,
    },
}

impl Profile2 {
    /// Offsets this profile by a finite signed `distance`.
    ///
    /// A positive distance grows the material — the outer loop moves
    /// outward and holes shrink — and a negative distance shrinks it. A
    /// zero distance returns the profile unchanged. The result is a
    /// validated [`Profile2`] with the same closure-by-construction and
    /// winding guarantees as any other profile.
    ///
    /// Lines and arcs offset exactly; cubics are refitted through kurbo.
    /// See the [module documentation](self) for the exact/fitted split,
    /// corner semantics, provenance, and the rejection rules.
    ///
    /// # Errors
    ///
    /// [`ProfileError::OffsetDistanceNotFinite`] for a NaN or infinite
    /// distance and [`ProfileError::InvalidDimension`] for a miter limit
    /// below `1.0` or not finite. Per-loop failures are
    /// [`ProfileError::OffsetArcCollapsed`],
    /// [`ProfileError::OffsetMiterLimitExceeded`],
    /// [`ProfileError::OffsetCornerUnsupported`],
    /// [`ProfileError::OffsetLoopDegenerate`],
    /// [`ProfileError::OffsetSelfIntersects`], and
    /// [`ProfileError::OffsetUndercut`]; whole-profile failures are
    /// [`ProfileError::OffsetLoopContact`] plus the ordinary
    /// [`Profile2::new`] validations.
    ///
    /// # Examples
    ///
    /// ```
    /// use exedra_constructive::builders;
    /// use exedra_constructive::offset::CornerPolicy;
    ///
    /// let tenon = builders::rect(40.0, 20.0).unwrap();
    /// // The receiving side is the same profile with 0.2 of clearance.
    /// let mortise = tenon.offset(0.2, CornerPolicy::Miter { limit: 2.0 }).unwrap();
    /// assert_eq!(mortise.outer().segs().len(), 4);
    /// ```
    pub fn offset(&self, distance: f64, corners: CornerPolicy) -> Result<Self, ProfileError> {
        if !distance.is_finite() {
            return Err(ProfileError::OffsetDistanceNotFinite);
        }
        if let CornerPolicy::Miter { limit } = corners
            && !(limit.is_finite() && limit >= 1.0)
        {
            return Err(ProfileError::InvalidDimension);
        }
        if distance == 0.0 {
            return Ok(self.clone());
        }

        let outer = offset_loop(self.outer(), distance, corners, None)?;
        let mut holes = Vec::with_capacity(self.holes().len());
        for (index, hole) in self.holes().iter().enumerate() {
            holes.push(offset_loop(hole, distance, corners, Some(index))?);
        }
        check_loop_separation(&outer, &holes)?;
        Self::new(outer, holes)
    }
}

/// Rejects offset holes that cross, touch, or escape the offset outer loop.
fn check_loop_separation(outer: &Loop2, holes: &[Loop2]) -> Result<(), ProfileError> {
    if holes.is_empty() {
        return Ok(());
    }
    let outer_path = outer.to_bez_path();
    let outer_ring = flattened_ring(&outer_path, ring_tolerance(&outer_path));
    for (index, hole) in holes.iter().enumerate() {
        let hole_path = hole.to_bez_path();
        let hole_ring = flattened_ring(&hole_path, ring_tolerance(&hole_path));
        let contact = rings_intersect(&outer_ring, &hole_ring)
            || hole_ring.first().is_none_or(|p| !outer_path.contains(*p));
        if contact {
            return Err(ProfileError::OffsetLoopContact { hole: index });
        }
    }
    Ok(())
}

fn ring_tolerance(path: &BezPath) -> f64 {
    let bounds = path.bounding_box();
    let scale = bounds.width().abs().max(bounds.height().abs());
    (scale * RING_TOLERANCE_RATIO).max(f64::MIN_POSITIVE)
}

/// One source segment's offset geometry, before corner resolution.
struct Piece {
    /// Offset image of the segment's start point.
    start: Point,
    /// Offset image of the segment's endpoint.
    end: Point,
    /// Unit right normal of the source segment at its start point.
    start_normal: Vec2,
    /// Unit right normal of the source segment at its endpoint.
    end_normal: Vec2,
    kind: PieceKind,
    tag: Option<SegTag>,
    policy: Option<PolicyId>,
}

enum PieceKind {
    /// A parallel line; endpoints fully describe it.
    Line,
    /// A concentric arc.
    Arc {
        center: Point,
        /// Offset radius, always positive.
        radius: f64,
        /// Source bulge, kept verbatim when neither end is trimmed.
        bulge: f64,
        /// Source sweep in radians, signed.
        sweep: f64,
    },
    /// A refitted cubic run; never trimmed, never extended.
    Fitted(Vec<Seg2>),
}

impl PieceKind {
    fn is_line(&self) -> bool {
        matches!(self, Self::Line)
    }
}

/// Corner resolution between two adjacent pieces.
struct Corner {
    /// Final endpoint of the earlier piece.
    end: Point,
    /// Final start point of the later piece.
    start: Point,
    /// Whether an endpoint moved along its own curve (arc sweeps change).
    trimmed: bool,
    /// Segments bridging `end` to `start`, in order.
    inserts: Vec<Seg2>,
}

fn offset_loop(
    src: &Loop2,
    distance: f64,
    corners: CornerPolicy,
    hole: Option<usize>,
) -> Result<Loop2, ProfileError> {
    let count = src.segs().len();
    let mut pieces = Vec::with_capacity(count);
    for (index, (from, seg)) in src.iter_with_starts().enumerate() {
        pieces.push(build_piece(from, seg, distance, hole, index)?);
    }

    let mut ends: Vec<Point> = pieces.iter().map(|p| p.end).collect();
    let mut starts: Vec<Point> = pieces.iter().map(|p| p.start).collect();
    let mut trimmed = vec![false; count];
    let mut inserts: Vec<Vec<Seg2>> = (0..count).map(|_| Vec::new()).collect();

    for index in 0..count {
        let next = (index + 1) % count;
        let corner = resolve_corner(
            &pieces[index],
            &pieces[next],
            src.segs()[index].to,
            distance,
            corners,
            hole,
            index,
        )?;
        ends[index] = corner.end;
        starts[next] = corner.start;
        if corner.trimmed {
            trimmed[index] = true;
            trimmed[next] = true;
        }
        inserts[index] = corner.inserts;
    }

    let mut segs: Vec<Seg2> = Vec::with_capacity(count * 2);
    for index in 0..count {
        let piece = &pieces[index];
        match &piece.kind {
            PieceKind::Line => segs.push(Seg2 {
                to: ends[index],
                kind: wrap(SegKind::Line, piece.policy),
                tag: piece.tag,
            }),
            PieceKind::Arc {
                center,
                bulge,
                sweep,
                ..
            } => {
                let bulge = if trimmed[index] {
                    trimmed_bulge(*center, starts[index], ends[index], *sweep)
                        .ok_or(ProfileError::OffsetArcCollapsed { hole, seg: index })?
                } else {
                    *bulge
                };
                segs.push(Seg2 {
                    to: ends[index],
                    kind: wrap(SegKind::Arc { bulge }, piece.policy),
                    tag: piece.tag,
                });
            }
            PieceKind::Fitted(fitted) => segs.extend(fitted.iter().cloned()),
        }
        segs.extend(inserts[index].iter().cloned());
    }

    let result = Loop2::new(segs).map_err(|_| ProfileError::OffsetLoopDegenerate { hole })?;
    check_result(src, &result, distance, hole)?;
    Ok(result)
}

/// Validates one offset loop against its source.
///
/// Winding must survive, the result must not touch itself, and no point of
/// the result may sit closer to the source than the offset distance — the
/// symptom of material eaten away by an over-large offset, which no local
/// corner rule can notice.
fn check_result(
    src: &Loop2,
    result: &Loop2,
    distance: f64,
    hole: Option<usize>,
) -> Result<(), ProfileError> {
    let src_path = src.to_bez_path();
    let result_path = result.to_bez_path();
    let tolerance = ring_tolerance(&src_path).max(ring_tolerance(&result_path));
    let src_ring = flattened_ring(&src_path, tolerance);
    let result_ring = flattened_ring(&result_path, tolerance);

    // Self-intersection first: a loop folded through itself usually flips
    // its winding too, and the fold is the more useful diagnosis.
    if ring_self_intersects(&result_ring) {
        return Err(ProfileError::OffsetSelfIntersects { hole });
    }
    if (result.signed_area() > 0.0) != (src.signed_area() > 0.0) {
        return Err(ProfileError::OffsetLoopDegenerate { hole });
    }

    let bounds = src_path.bounding_box();
    let scale = bounds.width().abs().max(bounds.height().abs());
    let slack = distance.abs().max(scale) * UNDERCUT_SLACK_RATIO;
    let floor = distance.abs() - slack;
    if src_ring.is_empty() {
        return Ok(());
    }
    for point in &result_ring {
        if distance_to_ring(*point, &src_ring) < floor {
            return Err(ProfileError::OffsetUndercut { hole });
        }
    }
    Ok(())
}

fn wrap(kind: SegKind, policy: Option<PolicyId>) -> SegKind {
    match policy {
        None => kind,
        Some(policy) => SegKind::PolicyTo {
            policy,
            realized: Box::new(kind),
        },
    }
}

/// Unit right normal of a direction: the side the offset moves toward for a
/// positive distance, which is outward for counter-clockwise outer loops
/// and into the void for clockwise hole loops.
fn right_normal(direction: Vec2) -> Option<Vec2> {
    let length = libm::sqrt(direction.x * direction.x + direction.y * direction.y);
    if !positive_finite(length) {
        return None;
    }
    Some(Vec2::new(direction.y / length, -direction.x / length))
}

/// Whether a magnitude is usable as a divisor or a radius.
fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

/// Tangent direction whose right normal is `normal`.
fn tangent_of(normal: Vec2) -> Vec2 {
    Vec2::new(-normal.y, normal.x)
}

fn build_piece(
    from: Point,
    seg: &Seg2,
    distance: f64,
    hole: Option<usize>,
    index: usize,
) -> Result<Piece, ProfileError> {
    let (kind, policy) = match &seg.kind {
        SegKind::PolicyTo { policy, realized } => (realized.as_ref(), Some(*policy)),
        other => (other, None),
    };
    let to = seg.to;
    let degenerate = ProfileError::OffsetLoopDegenerate { hole };

    match kind {
        SegKind::Line => {
            let normal = right_normal(to - from).ok_or(degenerate)?;
            Ok(Piece {
                start: from + distance * normal,
                end: to + distance * normal,
                start_normal: normal,
                end_normal: normal,
                kind: PieceKind::Line,
                tag: seg.tag,
                policy,
            })
        }
        SegKind::Arc { bulge } => {
            let (center, radius) = bulge_arc_center_radius(from, to, *bulge);
            if !positive_finite(radius) {
                return Err(ProfileError::OffsetArcCollapsed { hole, seg: index });
            }
            // A positive bulge sweeps counter-clockwise, which puts the
            // right normal on the far side of the center: offsetting grows
            // such an arc and shrinks a clockwise one.
            let sign = if *bulge > 0.0 { 1.0 } else { -1.0 };
            let offset_radius = radius + sign * distance;
            if !positive_finite(offset_radius) {
                return Err(ProfileError::OffsetArcCollapsed { hole, seg: index });
            }
            let radial = |point: Point| {
                Vec2::new(
                    (point.x - center.x) / radius * sign,
                    (point.y - center.y) / radius * sign,
                )
            };
            let start_normal = radial(from);
            let end_normal = radial(to);
            Ok(Piece {
                start: from + distance * start_normal,
                end: to + distance * end_normal,
                start_normal,
                end_normal,
                kind: PieceKind::Arc {
                    center,
                    radius: offset_radius,
                    bulge: *bulge,
                    sweep: 4.0 * libm::atan(*bulge),
                },
                tag: seg.tag,
                policy,
            })
        }
        SegKind::Cubic { c1, c2 } => {
            build_cubic_piece(from, to, *c1, *c2, seg.tag, policy, distance, hole)
        }
        SegKind::PolicyTo { .. } => Err(ProfileError::NestedPolicy { seg: index }),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one private constructor for a fitted piece; splitting it would only move the arguments"
)]
fn build_cubic_piece(
    from: Point,
    to: Point,
    c1: Point,
    c2: Point,
    tag: Option<SegTag>,
    policy: Option<PolicyId>,
    distance: f64,
    hole: Option<usize>,
) -> Result<Piece, ProfileError> {
    let degenerate = ProfileError::OffsetLoopDegenerate { hole };
    let cubic = CubicBez::new(from, c1, c2, to);
    let deriv = cubic.deriv();
    let start_normal = right_normal(deriv.eval(0.0).to_vec2())
        .or_else(|| right_normal(c2 - from))
        .or_else(|| right_normal(to - from))
        .ok_or(degenerate)?;
    let end_normal = right_normal(deriv.eval(1.0).to_vec2())
        .or_else(|| right_normal(to - c1))
        .or_else(|| right_normal(to - from))
        .ok_or(degenerate)?;

    let bounds = cubic.bounding_box();
    let scale = bounds.width().abs() + bounds.height().abs() + distance.abs();
    let tolerance = (scale * CUBIC_OFFSET_TOLERANCE_RATIO).max(f64::MIN_POSITIVE);
    let mut path = BezPath::new();
    // kurbo offsets along the left normal, this crate along the right one.
    kurbo::offset::offset_cubic(cubic, -distance, tolerance, &mut path);

    let mut fitted: Vec<Seg2> = Vec::new();
    let mut start = None;
    let mut current = Point::ZERO;
    let mut push = |seg: Seg2, current: &mut Point| {
        if seg.to != *current {
            *current = seg.to;
            fitted.push(seg);
        }
    };
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(point) => {
                start = Some(point);
                current = point;
            }
            PathEl::LineTo(point) => push(Seg2::line(point), &mut current),
            PathEl::QuadTo(control, point) => {
                // Degree elevation: exact cubic equivalent.
                let a = current + (control - current) * (2.0 / 3.0);
                let b = point + (control - point) * (2.0 / 3.0);
                push(Seg2::cubic(point, a, b), &mut current);
            }
            PathEl::CurveTo(a, b, point) => push(Seg2::cubic(point, a, b), &mut current),
            PathEl::ClosePath => {}
        }
    }
    let start = start.ok_or(degenerate)?;
    if fitted.is_empty() {
        return Err(degenerate);
    }
    // Provenance is one-to-many here: every fitted piece names the source
    // segment it came from.
    for seg in &mut fitted {
        seg.tag = tag;
        seg.kind = wrap(seg.kind.clone(), policy);
    }

    Ok(Piece {
        start,
        end: current,
        start_normal,
        end_normal,
        kind: PieceKind::Fitted(fitted),
        tag,
        policy,
    })
}

fn resolve_corner(
    before: &Piece,
    after: &Piece,
    vertex: Point,
    distance: f64,
    corners: CornerPolicy,
    hole: Option<usize>,
    index: usize,
) -> Result<Corner, ProfileError> {
    let plain = Corner {
        end: before.end,
        start: after.start,
        trimmed: false,
        inserts: Vec::new(),
    };
    if before.end == after.start {
        return Ok(plain);
    }

    let n0 = before.end_normal;
    let n1 = after.start_normal;
    let cross = n0.cross(n1);
    let dot = n0.dot(n1);
    let turn = libm::atan2(cross, dot);
    if libm::fabs(turn) <= TANGENT_EPSILON {
        // Tangent continuous: the two offset endpoints are the same point
        // up to rounding, so the chain simply carries on from the first.
        return Ok(Corner {
            end: before.end,
            start: before.end,
            trimmed: false,
            inserts: Vec::new(),
        });
    }
    let gap = distance * turn > 0.0;
    let overlap = distance * turn < 0.0;

    if gap {
        return match corners {
            CornerPolicy::Round => Ok(Corner {
                inserts: round_corner(after.start, turn),
                ..plain
            }),
            CornerPolicy::Miter { limit } => {
                miter_corner(before, after, vertex, distance, limit, hole, index)
            }
        };
    }
    if overlap {
        return trim_corner(before, after, vertex, hole, index);
    }
    // Unreachable for finite inputs (a nonzero distance and a turn past
    // the tangent epsilon always classify); bridging keeps the chain valid
    // rather than trusting that.
    Ok(Corner {
        inserts: vec![Seg2::line(after.start)],
        ..plain
    })
}

/// The arc filling a gap corner: radius `|distance|`, centered on the
/// source vertex, sweeping through the corner's turn angle.
fn round_corner(to: Point, turn: f64) -> Vec<Seg2> {
    let bulge = libm::tan(turn * 0.25);
    if bulge == 0.0 || !bulge.is_finite() {
        // A turn too small for a representable bulge: the chord is within
        // a rounding error of the arc it would replace.
        vec![Seg2::line(to)]
    } else {
        vec![Seg2::arc(to, bulge)]
    }
}

fn miter_corner(
    before: &Piece,
    after: &Piece,
    vertex: Point,
    distance: f64,
    limit: f64,
    hole: Option<usize>,
    index: usize,
) -> Result<Corner, ProfileError> {
    let n0 = before.end_normal;
    let n1 = after.start_normal;
    // The miter apex sits at V + d (n0 + n1) / (1 + n0 . n1), whose length
    // over |d| is 1 / cos(turn / 2) — the classic miter ratio.
    let denominator = 1.0 + n0.dot(n1);
    let exceeded = ProfileError::OffsetMiterLimitExceeded { hole, seg: index };
    if !positive_finite(denominator) {
        return Err(exceeded);
    }
    let unit_miter = (n0 + n1) / denominator;
    let ratio = libm::sqrt(unit_miter.x * unit_miter.x + unit_miter.y * unit_miter.y);
    if !ratio.is_finite() || ratio > limit {
        return Err(exceeded);
    }
    let apex = vertex + distance * unit_miter;

    // A line piece absorbs the miter by moving its endpoint along its own
    // direction, which keeps its tag and keeps the segment count down;
    // curved pieces get an inserted tangent line instead.
    let mut inserts = Vec::new();
    let mut cursor;
    let end = if before.kind.is_line() {
        cursor = apex;
        apex
    } else {
        cursor = before.end;
        if apex != cursor {
            inserts.push(Seg2::line(apex));
            cursor = apex;
        }
        before.end
    };
    let start = if after.kind.is_line() {
        cursor
    } else {
        if after.start != cursor {
            inserts.push(Seg2::line(after.start));
        }
        after.start
    };
    Ok(Corner {
        end,
        start,
        trimmed: false,
        inserts,
    })
}

/// Analytic primitive backing an offset piece, extended past its endpoints.
enum Prim {
    Line { point: Point, direction: Vec2 },
    Circle { center: Point, radius: f64 },
}

fn trim_corner(
    before: &Piece,
    after: &Piece,
    vertex: Point,
    hole: Option<usize>,
    index: usize,
) -> Result<Corner, ProfileError> {
    let unsupported = ProfileError::OffsetCornerUnsupported { hole, seg: index };
    let first = prim_of(before, before.end, before.end_normal).ok_or(unsupported)?;
    let second = prim_of(after, after.start, after.start_normal).ok_or(unsupported)?;
    let point =
        intersect(&first, &second, vertex).ok_or(ProfileError::OffsetLoopDegenerate { hole })?;
    Ok(Corner {
        end: point,
        start: point,
        trimmed: true,
        inserts: Vec::new(),
    })
}

fn prim_of(piece: &Piece, at: Point, normal: Vec2) -> Option<Prim> {
    match piece.kind {
        PieceKind::Line => Some(Prim::Line {
            point: at,
            direction: tangent_of(normal),
        }),
        PieceKind::Arc { center, radius, .. } => Some(Prim::Circle { center, radius }),
        PieceKind::Fitted(_) => None,
    }
}

/// Intersects two offset primitives, taking the candidate nearest `near`.
///
/// Corner resolution is local: the true trim point lies within a miter
/// length of the source vertex, so the nearest candidate is the right one.
fn intersect(first: &Prim, second: &Prim, near: Point) -> Option<Point> {
    let candidates = match (first, second) {
        (
            Prim::Line {
                point: p0,
                direction: d0,
            },
            Prim::Line {
                point: p1,
                direction: d1,
            },
        ) => intersect_lines(*p0, *d0, *p1, *d1),
        (Prim::Line { point, direction }, Prim::Circle { center, radius })
        | (Prim::Circle { center, radius }, Prim::Line { point, direction }) => {
            intersect_line_circle(*point, *direction, *center, *radius)
        }
        (
            Prim::Circle {
                center: c0,
                radius: r0,
            },
            Prim::Circle {
                center: c1,
                radius: r1,
            },
        ) => intersect_circles(*c0, *r0, *c1, *r1),
    };
    candidates
        .into_iter()
        .filter(|p| p.is_finite())
        .min_by(|a, b| {
            let da = (*a - near).hypot2();
            let db = (*b - near).hypot2();
            da.total_cmp(&db)
        })
}

fn intersect_lines(p0: Point, d0: Vec2, p1: Point, d1: Vec2) -> Vec<Point> {
    let denominator = d0.cross(d1);
    if denominator == 0.0 {
        return Vec::new();
    }
    let t = (p1 - p0).cross(d1) / denominator;
    vec![p0 + t * d0]
}

fn intersect_line_circle(point: Point, direction: Vec2, center: Point, radius: f64) -> Vec<Point> {
    let rel = point - center;
    // |rel + t d|^2 = r^2 with |d| = 1.
    let b = rel.dot(direction);
    let c = rel.dot(rel) - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return Vec::new();
    }
    let root = libm::sqrt(discriminant);
    vec![
        point + (-b - root) * direction,
        point + (-b + root) * direction,
    ]
}

fn intersect_circles(c0: Point, r0: f64, c1: Point, r1: f64) -> Vec<Point> {
    let delta = c1 - c0;
    let distance = libm::sqrt(delta.x * delta.x + delta.y * delta.y);
    if !positive_finite(distance) || distance > r0 + r1 || distance < libm::fabs(r0 - r1) {
        return Vec::new();
    }
    let a = (r0 * r0 - r1 * r1 + distance * distance) / (2.0 * distance);
    let height = libm::sqrt((r0 * r0 - a * a).max(0.0));
    let base = c0 + delta * (a / distance);
    let perpendicular = Vec2::new(-delta.y, delta.x) * (height / distance);
    vec![base + perpendicular, base - perpendicular]
}

/// Recomputes the bulge of an arc whose endpoints moved along its circle.
///
/// Center and radius are unchanged, so only the sweep is affected. Trimming
/// can only shorten an arc; a recomputed sweep that grew past the source
/// sweep means the corner consumed the segment, and the arc is reported as
/// collapsed.
fn trimmed_bulge(center: Point, start: Point, end: Point, sweep: f64) -> Option<f64> {
    let a0 = libm::atan2(start.y - center.y, start.x - center.x);
    let a1 = libm::atan2(end.y - center.y, end.x - center.x);
    let mut delta = a1 - a0;
    if sweep > 0.0 && delta <= 0.0 {
        delta += core::f64::consts::TAU;
    }
    if sweep < 0.0 && delta >= 0.0 {
        delta -= core::f64::consts::TAU;
    }
    if delta == 0.0 || !delta.is_finite() {
        return None;
    }
    if libm::fabs(delta) > libm::fabs(sweep) * (1.0 + 1e-9) + 1e-12 {
        return None;
    }
    let bulge = libm::tan(delta * 0.25);
    (bulge != 0.0 && bulge.is_finite()).then_some(bulge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders;
    use crate::profile::CanonBytes;

    const MITER: CornerPolicy = CornerPolicy::Miter { limit: 4.0 };

    fn endpoints(loop_: &Loop2) -> Vec<(f64, f64)> {
        loop_.segs().iter().map(|s| (s.to.x, s.to.y)).collect()
    }

    fn tags(loop_: &Loop2) -> Vec<Option<SegTag>> {
        loop_.segs().iter().map(|s| s.tag).collect()
    }

    #[test]
    fn rect_offset_outward_is_exact_and_keeps_tags() {
        let grown = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(2.0, MITER)
            .expect("offset");
        assert_eq!(
            endpoints(grown.outer()),
            vec![(12.0, -2.0), (12.0, 8.0), (-2.0, 8.0), (-2.0, -2.0)]
        );
        // One line in, one line out: provenance is one-to-one.
        assert_eq!(
            tags(grown.outer()),
            vec![
                Some(SegTag(0)),
                Some(SegTag(1)),
                Some(SegTag(2)),
                Some(SegTag(3))
            ]
        );
        assert!((builders::profile_area(&grown) - 14.0 * 10.0).abs() < 1e-12);
    }

    #[test]
    fn rect_offset_inward_is_exact() {
        let shrunk = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(-2.0, MITER)
            .expect("offset");
        assert_eq!(
            endpoints(shrunk.outer()),
            vec![(8.0, 2.0), (8.0, 4.0), (2.0, 4.0), (2.0, 2.0)]
        );
        assert_eq!(
            tags(shrunk.outer()),
            vec![
                Some(SegTag(0)),
                Some(SegTag(1)),
                Some(SegTag(2)),
                Some(SegTag(3))
            ]
        );
    }

    #[test]
    fn rect_offset_inward_past_half_width_is_rejected() {
        let err = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(-3.0, MITER)
            .expect_err("degenerate");
        assert_eq!(err, ProfileError::OffsetLoopDegenerate { hole: None });
        assert!(
            builders::rect(10.0, 6.0)
                .expect("rect")
                .offset(-4.0, MITER)
                .is_err()
        );
    }

    #[test]
    fn rect_offset_round_corners_insert_quarter_arcs() {
        let grown = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(2.0, CornerPolicy::Round)
            .expect("offset");
        let segs = grown.outer().segs();
        assert_eq!(segs.len(), 8, "four edges plus four corner arcs");
        let quarter = libm::tan(core::f64::consts::FRAC_PI_8);
        for (index, seg) in segs.iter().enumerate() {
            if index % 2 == 0 {
                assert!(matches!(seg.kind, SegKind::Line));
                let expected = u32::try_from(index).expect("small index") / 2;
                assert_eq!(seg.tag, Some(SegTag(expected)));
            } else {
                // Corner arcs belong to no single source segment.
                assert_eq!(seg.tag, None);
                let SegKind::Arc { bulge } = seg.kind else {
                    panic!("corner is an arc");
                };
                assert_eq!(bulge.to_bits(), quarter.to_bits());
            }
        }
        // Area = rect + four side strips + one full circle of radius 2.
        let expected = 10.0 * 6.0 + 2.0 * 2.0 * (10.0 + 6.0) + core::f64::consts::PI * 4.0;
        assert!((builders::profile_area(&grown) - expected).abs() < 1e-6);
    }

    #[test]
    fn rounded_rect_arcs_stay_concentric() {
        let source = builders::rounded_rect(10.0, 6.0, 1.5).expect("rounded rect");
        let grown = source.offset(0.5, MITER).expect("offset");
        assert_eq!(grown.outer().segs().len(), source.outer().segs().len());

        let mut checked = 0;
        let source_arcs = source.outer().iter_with_starts().collect::<Vec<_>>();
        for (index, (from, seg)) in grown.outer().iter_with_starts().enumerate() {
            let SegKind::Arc { bulge } = seg.kind else {
                continue;
            };
            let (source_from, source_seg) = source_arcs[index];
            let SegKind::Arc {
                bulge: source_bulge,
            } = source_seg.kind
            else {
                panic!("arcs stay arcs");
            };
            // Same sweep, bit for bit: no corner touched this arc.
            assert_eq!(bulge.to_bits(), source_bulge.to_bits());
            let (center, radius) = bulge_arc_center_radius(from, seg.to, bulge);
            let (source_center, source_radius) =
                bulge_arc_center_radius(source_from, source_seg.to, source_bulge);
            assert!((center - source_center).hypot() < 1e-12, "concentric");
            assert!(
                (radius - (source_radius + 0.5)).abs() < 1e-12,
                "radius grew"
            );
            assert_eq!(seg.tag, source_seg.tag);
            checked += 1;
        }
        assert_eq!(checked, 4, "all four corner arcs checked");
    }

    #[test]
    fn rounded_rect_inward_past_corner_radius_is_rejected() {
        let source = builders::rounded_rect(10.0, 6.0, 1.5).expect("rounded rect");
        assert!(source.offset(-1.0, MITER).is_ok(), "inside the radius");
        // At and beyond the corner radius no concentric arc exists.
        assert_eq!(
            source.offset(-1.5, MITER),
            Err(ProfileError::OffsetArcCollapsed { hole: None, seg: 1 })
        );
        assert_eq!(
            source.offset(-2.0, MITER),
            Err(ProfileError::OffsetArcCollapsed { hole: None, seg: 1 })
        );
    }

    fn square_hole_profile() -> Profile2 {
        let outer = Loop2::new(vec![
            Seg2::line((20.0, 0.0)).tagged(SegTag(0)),
            Seg2::line((20.0, 20.0)).tagged(SegTag(1)),
            Seg2::line((0.0, 20.0)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("outer");
        let hole = Loop2::new(vec![
            Seg2::line((14.0, 6.0)).tagged(SegTag(4)),
            Seg2::line((14.0, 14.0)).tagged(SegTag(5)),
            Seg2::line((6.0, 14.0)).tagged(SegTag(6)),
            Seg2::line((6.0, 6.0)).tagged(SegTag(7)),
        ])
        .expect("hole")
        .reversed();
        Profile2::new(outer, vec![hole]).expect("profile")
    }

    #[test]
    fn outward_offset_shrinks_a_hole() {
        let grown = square_hole_profile().offset(1.0, MITER).expect("offset");
        assert_eq!(grown.holes().len(), 1);
        let mut xs: Vec<f64> = grown.holes()[0].segs().iter().map(|s| s.to.x).collect();
        let mut ys: Vec<f64> = grown.holes()[0].segs().iter().map(|s| s.to.y).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        assert_eq!(xs, vec![7.0, 7.0, 13.0, 13.0], "hole shrank by one");
        assert_eq!(ys, vec![7.0, 7.0, 13.0, 13.0]);
        // Outer grew, hole shrank: material grows on both counts.
        assert!((builders::profile_area(&grown) - (22.0 * 22.0 - 6.0 * 6.0)).abs() < 1e-9);
    }

    #[test]
    fn large_outward_offset_collapses_a_hole() {
        // Half the hole width leaves nothing to keep.
        assert_eq!(
            square_hole_profile().offset(4.0, MITER),
            Err(ProfileError::OffsetLoopDegenerate { hole: Some(0) })
        );
        // Beyond it the corner trims invert into a phantom hole; the
        // undercut check is what notices.
        assert_eq!(
            square_hole_profile().offset(6.0, MITER),
            Err(ProfileError::OffsetUndercut { hole: Some(0) })
        );
    }

    #[test]
    fn inward_offset_rejects_holes_that_grow_into_contact() {
        let outer = Loop2::new(vec![
            Seg2::line((0.0, 0.0)),
            Seg2::line((20.0, 0.0)),
            Seg2::line((20.0, 20.0)),
            Seg2::line((0.0, 20.0)),
        ])
        .expect("outer");
        let hole = |x0: f64, x1: f64| {
            Loop2::new(vec![
                Seg2::line((x0, 4.0)),
                Seg2::line((x0, 6.0)),
                Seg2::line((x1, 6.0)),
                Seg2::line((x1, 4.0)),
            ])
            .expect("hole")
        };
        let profile =
            Profile2::new(outer, vec![hole(4.0, 6.0), hole(8.0, 10.0)]).expect("separated holes");

        assert_eq!(
            profile.offset(-1.0, MITER),
            Err(ProfileError::OverlappingHoles {
                first: 0,
                second: 1,
            })
        );
    }

    #[test]
    fn l_profile_inward_past_the_arm_is_rejected() {
        // Arms are four wide, so anything past two eats through them.
        let l = builders::l_profile(10.0, 10.0, 6.0, 6.0).expect("L");
        let inside = l.offset(-1.0, CornerPolicy::Round);
        assert!(inside.is_ok(), "{inside:?}");
        // Every edge of an L crosses its opposite at once, so the eaten
        // result comes out simple but inside out rather than knotted: the
        // winding check is what refuses it.
        assert_eq!(
            l.offset(-3.0, CornerPolicy::Round),
            Err(ProfileError::OffsetLoopDegenerate { hole: None })
        );
        assert!(l.offset(-2.0, CornerPolicy::Round).is_err());
    }

    /// A rectangle with a slot cut into its top edge, counter-clockwise.
    fn slotted_profile() -> Profile2 {
        let outer = Loop2::new(vec![
            Seg2::line((10.0, 0.0)).tagged(SegTag(0)),
            Seg2::line((10.0, 10.0)).tagged(SegTag(1)),
            Seg2::line((6.0, 10.0)).tagged(SegTag(2)),
            Seg2::line((6.0, 3.0)).tagged(SegTag(3)),
            Seg2::line((4.0, 3.0)).tagged(SegTag(4)),
            Seg2::line((4.0, 10.0)).tagged(SegTag(5)),
            Seg2::line((0.0, 10.0)).tagged(SegTag(6)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(7)),
        ])
        .expect("slotted outline");
        Profile2::simple(outer).expect("profile")
    }

    #[test]
    fn outward_offset_closing_a_slot_self_intersects() {
        let slotted = slotted_profile();
        // The slot is two wide, so half of that still leaves a gap.
        let narrowed = slotted.offset(0.5, CornerPolicy::Round);
        assert!(narrowed.is_ok(), "{narrowed:?}");
        // Past it the two slot walls swap sides and the boundary knots.
        assert_eq!(
            slotted.offset(1.5, CornerPolicy::Round),
            Err(ProfileError::OffsetSelfIntersects { hole: None })
        );
    }

    #[test]
    fn cubic_loop_offsets_and_stays_closed() {
        let source = Loop2::new(vec![
            Seg2::line((10.0, 0.0)).tagged(SegTag(0)),
            Seg2::cubic((10.0, 10.0), (14.0, 3.0), (14.0, 7.0)).tagged(SegTag(1)),
            Seg2::line((0.0, 10.0)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("loop");
        let profile = Profile2::simple(source).expect("profile");
        let grown = profile.offset(1.0, CornerPolicy::Round).expect("offset");

        let segs = grown.outer().segs();
        assert!(segs.len() >= 4);
        // Closure is structural, but the geometry must still chain.
        for (from, seg) in grown.outer().iter_with_starts() {
            assert!(from.is_finite() && seg.to.is_finite());
            assert_ne!(from, seg.to);
        }
        // Every fitted piece names its source segment: one-to-many.
        let fitted: Vec<_> = segs
            .iter()
            .filter(|s| matches!(s.kind, SegKind::Cubic { .. }))
            .collect();
        assert!(!fitted.is_empty(), "the cubic offsets to cubics");
        assert!(fitted.iter().all(|s| s.tag == Some(SegTag(1))));
        assert!(builders::profile_area(&grown) > builders::profile_area(&profile));
    }

    #[test]
    fn miter_limit_is_enforced_rather_than_beveled() {
        // A 20-degree spike needs a miter about 5.76 times the distance.
        let spike = Loop2::new(vec![
            Seg2::line((10.0, 0.0)).tagged(SegTag(0)),
            Seg2::line((0.0, 1.7632698070846498)).tagged(SegTag(1)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(2)),
        ])
        .expect("spike");
        let profile = Profile2::simple(spike).expect("profile");
        assert_eq!(
            profile.offset(0.1, CornerPolicy::Miter { limit: 2.0 }),
            Err(ProfileError::OffsetMiterLimitExceeded { hole: None, seg: 0 })
        );
        assert!(profile.offset(0.1, CornerPolicy::Round).is_ok());
    }

    #[test]
    fn zero_distance_is_the_identity() {
        let source = builders::rounded_rect(10.0, 6.0, 1.5).expect("rounded rect");
        assert_eq!(source.offset(0.0, MITER), Ok(source.clone()));
        assert_eq!(source.offset(0.0, CornerPolicy::Round), Ok(source));
    }

    #[test]
    fn invalid_inputs_are_rejected() {
        let source = builders::rect(10.0, 6.0).expect("rect");
        assert_eq!(
            source.offset(f64::NAN, MITER),
            Err(ProfileError::OffsetDistanceNotFinite)
        );
        assert_eq!(
            source.offset(f64::INFINITY, MITER),
            Err(ProfileError::OffsetDistanceNotFinite)
        );
        assert_eq!(
            source.offset(1.0, CornerPolicy::Miter { limit: 0.5 }),
            Err(ProfileError::InvalidDimension)
        );
        assert_eq!(
            source.offset(1.0, CornerPolicy::Miter { limit: f64::NAN }),
            Err(ProfileError::InvalidDimension)
        );
    }

    #[test]
    fn offset_canon_bytes_are_pinned() {
        let grown = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(2.0, CornerPolicy::Round)
            .expect("offset");

        // The offset of a rectangle is an ordinary segment profile, so it
        // must encode exactly like the same profile written by hand.
        let quarter = libm::tan(core::f64::consts::FRAC_PI_8);
        let expected = Profile2::simple(
            Loop2::new(vec![
                Seg2::line((10.0, -2.0)).tagged(SegTag(0)),
                Seg2::arc((12.0, 0.0), quarter),
                Seg2::line((12.0, 6.0)).tagged(SegTag(1)),
                Seg2::arc((10.0, 8.0), quarter),
                Seg2::line((0.0, 8.0)).tagged(SegTag(2)),
                Seg2::arc((-2.0, 6.0), quarter),
                Seg2::line((-2.0, 0.0)).tagged(SegTag(3)),
                Seg2::arc((0.0, -2.0), quarter),
            ])
            .expect("expected loop"),
        )
        .expect("expected profile");

        let mut actual_bytes = Vec::new();
        grown.canon_bytes(&mut actual_bytes);
        let mut expected_bytes = Vec::new();
        expected.canon_bytes(&mut expected_bytes);
        assert_eq!(actual_bytes, expected_bytes);

        // Recomputing the offset reproduces the same bits.
        let again = builders::rect(10.0, 6.0)
            .expect("rect")
            .offset(2.0, CornerPolicy::Round)
            .expect("offset");
        let mut again_bytes = Vec::new();
        again.canon_bytes(&mut again_bytes);
        assert_eq!(actual_bytes, again_bytes);
    }
}
