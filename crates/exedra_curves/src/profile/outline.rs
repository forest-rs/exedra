// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Line and circular-arc loops and profiles with validated arrangements.

use alloc::vec::Vec;
use core::f64::consts::PI;

use exedra_math::predicates::{Orientation, orient2d};

use crate::intersect::{BezierEnclosure, HullSeparation};
use crate::profile::Bezier;
use crate::profile::arc::{
    Arc, Piece, bounds_may_contact, cross, dot, length, sub, turns, winding_angle,
};
use crate::profile::bezier::BezierPiece;

/// Opaque caller tag carried by a segment across geometry representations.
///
/// Constructive profiles reexport this same type. Lowering, segment reversal
/// and analytic extrusion preserve it unchanged, so a feature can be tagged
/// once and selected with the same value after realization. Tags name caller
/// features; they do not establish ownership or replace scoped element ids.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegTag(pub u32);

/// Geometry of one segment between its start (the previous endpoint) and
/// [`Segment::to`].
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// A straight chord.
    Line,
    /// A circular arc with `bulge = tan(sweep / 4)`: positive sweeps
    /// counter-clockwise and lies to the right of the chord direction. A
    /// magnitude of one is a half circle; larger magnitudes are major arcs.
    Arc {
        /// Tangent of a quarter of the signed sweep; finite and nonzero.
        bulge: f64,
    },
    /// A circular arc with an authoritative center and travel direction.
    /// Radius and sweep are derived from the stored endpoints, never fitted.
    ArcCenter {
        /// Center in profile coordinates.
        center: [f64; 2],
        /// Clockwise travel; false means counter-clockwise.
        clockwise: bool,
    },
    /// A rational Bezier curve whose end controls are the segment's start
    /// and end.
    Bezier(Bezier),
}

/// One segment of a loop.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// End point; the next segment starts here.
    pub to: [f64; 2],
    /// Straight, circular or rational Bezier geometry.
    pub curve: Curve,
    /// Optional caller tag.
    pub tag: Option<SegTag>,
}

impl Segment {
    /// A straight segment to `to`.
    #[must_use]
    pub const fn line(to: [f64; 2]) -> Self {
        Self {
            to,
            curve: Curve::Line,
            tag: None,
        }
    }

    /// A circular arc to `to` with the given bulge.
    #[must_use]
    pub const fn arc(to: [f64; 2], bulge: f64) -> Self {
        Self {
            to,
            curve: Curve::Arc { bulge },
            tag: None,
        }
    }

    /// A center-defined arc ending at `to`; its start is the preceding endpoint.
    /// [`Loop::new`] validates radius agreement under fixed local f64 roundoff.
    #[must_use]
    pub const fn arc_centered(to: [f64; 2], center: [f64; 2], clockwise: bool) -> Self {
        Self {
            to,
            curve: Curve::ArcCenter { center, clockwise },
            tag: None,
        }
    }

    /// A rational Bezier segment to `to` through `bezier`'s controls.
    #[must_use]
    pub const fn bezier(to: [f64; 2], bezier: Bezier) -> Self {
        Self {
            to,
            curve: Curve::Bezier(bezier),
            tag: None,
        }
    }

    /// Attaches a tag.
    #[must_use]
    pub const fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
}

/// A closed loop: segment `i` starts at the end of segment `i - 1`, cyclically.
///
/// Construction checks the loop's own structure. Whether it is simple, and
/// how it relates to other loops, is checked by [`Profile::new`].
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    segments: Vec<Segment>,
}

/// Why segments do not form a loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopError {
    /// A loop needs at least two segments; a full circle is two arcs.
    TooFewSegments {
        /// Number of segments provided.
        count: usize,
    },
    /// The segment count does not fit in `u32`.
    TooManySegments,
    /// An endpoint is NaN or infinite.
    NonFinite {
        /// Offending segment.
        segment: usize,
    },
    /// A segment starts where it ends.
    ZeroLength {
        /// Offending segment.
        segment: usize,
    },
    /// An arc has invalid input, inconsistent endpoint radii, or no representable circle.
    /// [`Arc::resolve`] exposes the detailed geometry evidence for its segment.
    InvalidArc {
        /// Offending segment.
        segment: usize,
    },
    /// A Bezier segment has too many controls, a weight count other than
    /// its order, a control or weight [`crate::Nurbs::validate`] would
    /// refuse, weights spread too far or controls too close for `f64`
    /// ([`Bezier`]).
    InvalidBezier {
        /// Offending segment.
        segment: usize,
    },
}

impl core::fmt::Display for LoopError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooFewSegments { count } => {
                write!(f, "a loop needs at least two segments, got {count}")
            }
            Self::TooManySegments => write!(f, "segment count exceeds u32"),
            Self::NonFinite { segment } => write!(f, "segment {segment} is not finite"),
            Self::ZeroLength { segment } => write!(f, "segment {segment} has zero length"),
            Self::InvalidArc { segment } => write!(f, "segment {segment} has no valid arc"),
            Self::InvalidBezier { segment } => {
                write!(f, "segment {segment} has no valid Bezier controls")
            }
        }
    }
}

impl core::error::Error for LoopError {}

impl Loop {
    /// Creates a loop from its segments.
    ///
    /// Requires at least two segments, finite endpoints (`-0.0` is stored as
    /// `0.0`), distinct consecutive endpoints, and arcs with a representable
    /// circle.
    ///
    /// # Errors
    ///
    /// Returns the first violated requirement in segment order.
    pub fn new(mut segments: Vec<Segment>) -> Result<Self, LoopError> {
        if segments.len() < 2 {
            return Err(LoopError::TooFewSegments {
                count: segments.len(),
            });
        }
        if u32::try_from(segments.len()).is_err() {
            return Err(LoopError::TooManySegments);
        }
        for (index, segment) in segments.iter_mut().enumerate() {
            if !segment.to.iter().all(|v| v.is_finite()) {
                return Err(LoopError::NonFinite { segment: index });
            }
            // Adding zero turns `-0.0` into `0.0` and changes nothing else.
            segment.to = segment.to.map(|v| v + 0.0);
        }
        let loop_ = Self { segments };
        for (index, (from, segment)) in loop_.iter_with_starts().enumerate() {
            if from == segment.to {
                return Err(LoopError::ZeroLength { segment: index });
            }
            if Arc::resolve(from, segment.to, &segment.curve).is_err() {
                return Err(LoopError::InvalidArc { segment: index });
            }
            if let Curve::Bezier(bezier) = &segment.curve
                && !bezier.valid(from, segment.to)
            {
                return Err(LoopError::InvalidBezier { segment: index });
            }
        }
        Ok(loop_)
    }

    /// The segments in order.
    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Iterates `(start, segment)` pairs.
    pub fn iter_with_starts(&self) -> impl Iterator<Item = ([f64; 2], &Segment)> + '_ {
        let n = self.segments.len();
        (0..n).map(move |i| (self.segments[(i + n - 1) % n].to, &self.segments[i]))
    }

    /// The arc of segment `index`, or `None` for a line or an index out of range.
    #[must_use]
    pub fn arc(&self, index: usize) -> Option<Arc> {
        let segment = self.segments.get(index)?;
        let from = self.segments[(index + self.segments.len() - 1) % self.segments.len()].to;
        Arc::resolve(from, segment.to, &segment.curve)
            .ok()
            .flatten()
    }

    /// This loop traversed the other way: segment order reverses, bulges
    /// negate, center-defined directions toggle, and tags stay with segments.
    #[must_use]
    pub fn reversed(&self) -> Self {
        let n = self.segments.len();
        let segments = (0..n)
            .map(|i| {
                let source = &self.segments[n - 1 - i];
                let curve = match &source.curve {
                    Curve::Line => Curve::Line,
                    Curve::Arc { bulge } => Curve::Arc { bulge: -bulge },
                    Curve::ArcCenter { center, clockwise } => Curve::ArcCenter {
                        center: *center,
                        clockwise: !clockwise,
                    },
                    Curve::Bezier(bezier) => Curve::Bezier(bezier.reversed()),
                };
                Segment {
                    to: self.segments[(2 * n - 2 - i) % n].to,
                    curve,
                    tag: source.tag,
                }
            })
            .collect();
        Self { segments }
    }

    /// Signed enclosed area: positive when counter-clockwise. Lines, arcs
    /// and polynomial Bezier segments are closed forms, exact to rounding;
    /// a rational Bezier segment's part is an estimate, adaptive
    /// Gauss-Legendre quadrature to `2^-44` of its extent squared. Nothing
    /// a profile decides rests on it.
    #[must_use]
    pub fn signed_area(&self) -> f64 {
        // Shoelace over chords relative to the first vertex, which keeps the
        // products small for loops far from the origin, plus each arc's
        // circular segment.
        let origin = self.segments[self.segments.len() - 1].to;
        self.pieces()
            .map(|piece| {
                let chord = 0.5 * cross(sub(piece.from(), origin), sub(piece.to(), origin));
                match piece {
                    Piece::Line(..) => chord,
                    Piece::Arc(arc) => chord + arc.segment_area(),
                    Piece::Bezier(bezier) => chord + bezier.segment_integrals().0,
                }
            })
            .sum()
    }

    /// Whether the loop winds counter-clockwise, for a loop whose segments
    /// meet only at their shared vertices, as [`Profile::validate`] proves
    /// before asking; `None` where that cannot be told.
    ///
    /// The sign of [`Self::signed_area`] decides wherever the area exceeds
    /// a bound on the rounding that computed it. A sliver thinner than
    /// that rounding is decided, for a loop of lines, by the exact turn at
    /// its lowest-leftmost vertex, which a simple polygon makes towards its
    /// inside; a sliver with arcs is not decided.
    pub(crate) fn counter_clockwise(&self) -> Option<bool> {
        if self.has_bezier() {
            // Their areas are quadratures; their chord polygons, once
            // proved, decide instead.
            return None;
        }
        let origin = self.segments[self.segments.len() - 1].to;
        let (mut area, mut chords, mut arcs) = (0.0_f64, 0.0_f64, 0.0_f64);
        for piece in self.pieces() {
            let (u, v) = (sub(piece.from(), origin), sub(piece.to(), origin));
            area += 0.5 * cross(u, v);
            chords += 0.5 * ((u[0] * v[1]).abs() + (u[1] * v[0]).abs());
            if let Piece::Arc(arc) = piece {
                let segment = arc.segment_area();
                area += segment;
                arcs += segment.abs();
            }
        }
        // Each chord term rounds in its two differences, two products and
        // one subtraction, and the sum in at most one addition per term:
        // within `(n + 6) EPSILON` of the chords' magnitude, with a
        // subnormal floor per term. An arc's segment area carries the
        // rounding of its resolved radius and sweep and of `angle -
        // sin(angle)`, generously within `2^16 EPSILON` of itself.
        let n = self.segments.len() as f64;
        let bound = 2.0 * (n + 6.0) * f64::EPSILON * chords
            + 65536.0 * f64::EPSILON * arcs
            + 16.0 * n * f64::from_bits(1);
        if area.abs() > bound {
            return Some(area > 0.0);
        }
        if !self
            .segments
            .iter()
            .all(|segment| segment.curve == Curve::Line)
        {
            return None;
        }
        let k = self.lowest_leftmost();
        let m = self.segments.len();
        let previous = self.segments[(k + m - 1) % m].to;
        let next = self.segments[(k + 1) % m].to;
        match orient2d(previous, self.segments[k].to, next) {
            Orientation::Ccw => Some(true),
            Orientation::Cw => Some(false),
            Orientation::Collinear => None,
        }
    }

    /// Whether any segment is a rational Bezier.
    pub(crate) fn has_bezier(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| matches!(segment.curve, Curve::Bezier(_)))
    }

    /// The index of the vertex with the least `y`, then the least `x`.
    fn lowest_leftmost(&self) -> usize {
        (0..self.segments.len())
            .min_by(|&a, &b| {
                let ([ax, ay], [bx, by]) = (self.segments[a].to, self.segments[b].to);
                ay.total_cmp(&by).then(ax.total_cmp(&bx))
            })
            .expect("a loop has at least two segments")
    }

    /// An axis-aligned box containing the loop, as `[min, max]`: the
    /// smallest, but around a Bezier segment, whose controls' box it
    /// includes.
    #[must_use]
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        self.pieces().fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[min, max], piece| {
                let [lo, hi] = match piece {
                    Piece::Line(from, to) => [
                        [from[0].min(to[0]), from[1].min(to[1])],
                        [from[0].max(to[0]), from[1].max(to[1])],
                    ],
                    Piece::Arc(arc) => arc.bounds(),
                    Piece::Bezier(bezier) => bezier.bounds(),
                };
                [
                    [min[0].min(lo[0]), min[1].min(lo[1])],
                    [max[0].max(hi[0]), max[1].max(hi[1])],
                ]
            },
        )
    }

    /// Total length of the loop; a Bezier segment's is an estimate, by
    /// adaptive Gauss-Legendre quadrature of its speed to `2^-44` of its
    /// extent, as no closed form exists.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.pieces().map(|piece| piece_length(&piece)).sum()
    }

    /// Length of segment `index`, estimated for a Bezier segment as
    /// [`Self::length`] says, or `None` past the last segment.
    #[must_use]
    pub fn segment_length(&self, index: usize) -> Option<f64> {
        self.pieces().nth(index).map(|piece| piece_length(&piece))
    }

    pub(crate) fn pieces(&self) -> impl Iterator<Item = Piece> + '_ {
        self.iter_with_starts()
            .map(|(from, segment)| match &segment.curve {
                Curve::Line => Piece::Line(from, segment.to),
                Curve::Bezier(bezier) => Piece::Bezier(BezierPiece {
                    from,
                    to: segment.to,
                    bezier: bezier.clone(),
                }),
                curve => Piece::Arc(
                    Arc::resolve(from, segment.to, curve)
                        .expect("arcs are checked by Loop::new")
                        .expect("non-line curve"),
                ),
            })
    }

    /// Whole turns the loop winds around `point`, which must not lie on it:
    /// for a loop of lines, its signed crossings of the ray rightward from
    /// the point, each decided by exact comparisons and orientation, so no
    /// product of tiny differences can underflow to a wrong answer.
    pub(crate) fn winding(&self, point: [f64; 2]) -> i64 {
        if self
            .segments
            .iter()
            .all(|segment| segment.curve == Curve::Line)
        {
            let mut winding = 0;
            for (from, segment) in self.iter_with_starts() {
                let to = segment.to;
                // Upward crossings with the point on the left count one
                // way, downward ones with it on the right the other.
                if from[1] <= point[1] {
                    if to[1] > point[1] && orient2d(from, to, point) == Orientation::Ccw {
                        winding += 1;
                    }
                } else if to[1] <= point[1] && orient2d(from, to, point) == Orientation::Cw {
                    winding -= 1;
                }
            }
            return winding;
        }
        turns(
            self.pieces()
                .map(|piece| winding_angle(&piece, point, 0.0))
                .sum(),
        )
    }
}

fn piece_length(piece: &Piece) -> f64 {
    match piece {
        Piece::Line(from, to) => length(sub(*to, *from)),
        Piece::Arc(arc) => arc.length(),
        Piece::Bezier(bezier) => bezier.length(),
    }
}

/// Validation limits for [`Profile::new`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ProfilePolicy {
    /// Smallest permitted distance between nonadjacent segments, and between
    /// adjacent segments away from their shared vertex, in model units.
    /// Contact is judged at the larger of this and the rounding of
    /// distances at the two segments' own coordinates, `16 EPSILON` times
    /// the largest of them, since a finer separation cannot be told from
    /// contact there.
    pub min_separation: f64,
    /// Largest total segment count; pairwise checks are quadratic in it.
    /// For a profile with Bezier segments, also the most pieces its proof
    /// may subdivide them into ([`Profile::validate`]).
    pub max_segments: u32,
}

impl ProfilePolicy {
    /// A policy with the given separation and a budget of 4096 segments.
    #[must_use]
    pub const fn new(min_separation: f64) -> Self {
        Self {
            min_separation,
            max_segments: 4096,
        }
    }
}

/// A loop of a [`Profile`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LoopRef {
    /// The outer loop.
    Outer,
    /// The hole at this index.
    Hole(usize),
}

/// A segment of a [`Profile`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SegmentRef {
    /// The segment's loop.
    pub loop_ref: LoopRef,
    /// The segment's index in its loop.
    pub segment: usize,
}

/// Why loops do not form a valid profile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// The separation is not finite and positive, or the segment budget is zero.
    InvalidPolicy,
    /// The loops have more segments than [`ProfilePolicy::max_segments`].
    TooManySegments {
        /// Total segment count.
        count: usize,
        /// The budget.
        maximum: u32,
    },
    /// The outer loop must wind counter-clockwise and holes clockwise.
    WrongWinding {
        /// The offending loop.
        loop_ref: LoopRef,
    },
    /// The proof for a profile with Bezier segments could not settle a
    /// segment, or a pair of them, within
    /// [`ProfilePolicy::max_segments`] pieces: they may cross, touch, come
    /// nearer than the separation, or simply need more subdivision than the
    /// budget allows.
    Unresolved {
        /// The segment, in outer-then-hole order.
        first: SegmentRef,
        /// The other segment of an unresolved pair.
        second: Option<SegmentRef>,
    },
    /// A loop is so thin that its winding cannot be told: its area is
    /// within the rounding of computing it, and it has arcs, so no exact
    /// turn decides it.
    UnresolvedWinding {
        /// The offending loop.
        loop_ref: LoopRef,
    },
    /// Two segments cross, touch, overlap, or come closer than the
    /// separation. Adjacent segments may meet only at their shared vertex.
    Contact {
        /// The earlier segment, in outer-then-hole order.
        first: SegmentRef,
        /// The later segment.
        second: SegmentRef,
    },
    /// A hole lies outside the outer loop.
    HoleOutside {
        /// The offending hole.
        hole: usize,
    },
    /// A hole lies inside another hole.
    NestedHole {
        /// The inner hole.
        hole: usize,
        /// The hole containing it.
        within: usize,
    },
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPolicy => write!(f, "profile separation or segment budget is invalid"),
            Self::TooManySegments { count, maximum } => {
                write!(f, "{count} segments exceed the budget of {maximum}")
            }
            Self::WrongWinding { loop_ref } => match loop_ref {
                LoopRef::Outer => write!(f, "the outer loop must wind counter-clockwise"),
                LoopRef::Hole(hole) => write!(f, "hole {hole} must wind clockwise"),
            },
            Self::Unresolved { first, second } => {
                write!(
                    f,
                    "{} segment {} could not be proved",
                    LoopName(first.loop_ref),
                    first.segment
                )?;
                if let Some(second) = second {
                    write!(
                        f,
                        " apart from {} segment {}",
                        LoopName(second.loop_ref),
                        second.segment
                    )?;
                }
                Ok(())
            }
            Self::UnresolvedWinding { loop_ref } => {
                write!(
                    f,
                    "{} loop is too thin to tell its winding",
                    LoopName(*loop_ref)
                )
            }
            Self::Contact { first, second } => write!(
                f,
                "{} segment {} and {} segment {} touch or come too close",
                LoopName(first.loop_ref),
                first.segment,
                LoopName(second.loop_ref),
                second.segment
            ),
            Self::HoleOutside { hole } => write!(f, "hole {hole} lies outside the outer loop"),
            Self::NestedHole { hole, within } => write!(f, "hole {hole} lies inside hole {within}"),
        }
    }
}

impl core::error::Error for ProfileError {}

struct LoopName(LoopRef);

impl core::fmt::Display for LoopName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            LoopRef::Outer => write!(f, "outer"),
            LoopRef::Hole(hole) => write!(f, "hole {hole}"),
        }
    }
}

/// A counter-clockwise outer loop with clockwise holes, whose segments keep
/// a minimum separation.
///
/// ```
/// use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
///
/// // A 4 x 2 rectangle with a round hole of radius 0.5.
/// let outer = Loop::new(vec![
///     Segment::line([4.0, 0.0]),
///     Segment::line([4.0, 2.0]),
///     Segment::line([0.0, 2.0]),
///     Segment::line([0.0, 0.0]),
/// ])?;
/// let hole = Loop::new(vec![
///     Segment::arc([1.5, 1.0], -1.0),
///     Segment::arc([2.5, 1.0], -1.0),
/// ])?;
/// let profile = Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9))?;
/// let expected = 8.0 - core::f64::consts::PI * 0.25;
/// assert!((profile.area() - expected).abs() < 1e-12);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    outer: Loop,
    holes: Vec<Loop>,
}

impl Profile {
    /// Validates the arrangement of `outer` and `holes`.
    ///
    /// Checks, in order: the policy and segment budget; that no two
    /// segments of any loops cross, touch, overlap or come within
    /// [`ProfilePolicy::min_separation`] (adjacent segments meet only at their
    /// shared vertex, and are checked away from it); winding, now that each
    /// loop is known to be simple; then that every hole lies inside the
    /// outer loop and outside every other hole. Nothing is moved, merged or
    /// healed.
    ///
    /// Where any loop holds a Bezier segment, these checks are a proof by
    /// outward-rounded enclosures, and what it cannot settle is refused as
    /// [`ProfileError::Unresolved`]. There, a center-defined arc, whose
    /// stored ends lie at distances from its center that agree only to
    /// rounding, is checked as every circle about the center between those
    /// distances; where it meets the next segment, the two are proved to
    /// meet nowhere outside a ball about their shared vertex, 64 times as
    /// wide as that disagreement and the rounding of the arc's ends reach
    /// (never narrower than `4096 EPSILON` of their coordinates). Within the
    /// ball, which is part of the vertex, the arc's readings may meet the
    /// other segment.
    ///
    /// # Errors
    ///
    /// Returns the first violation found.
    pub fn new(
        outer: Loop,
        holes: Vec<Loop>,
        policy: &ProfilePolicy,
    ) -> Result<Self, ProfileError> {
        let profile = Self { outer, holes };
        profile.validate(policy)?;
        Ok(profile)
    }

    /// Checks this profile against another explicit separation and work budget.
    /// Geometry and tags are unchanged; this avoids cloning loops when a later
    /// operation needs stronger validation than the original authoring policy.
    ///
    /// # Errors
    ///
    /// Returns the first arrangement or policy violation in the same order as
    /// [`Self::new`].
    pub fn validate(&self, policy: &ProfilePolicy) -> Result<(), ProfileError> {
        let outer = &self.outer;
        let holes = &self.holes;
        let separation = policy.min_separation;
        if !(separation.is_finite() && separation > 0.0) || policy.max_segments == 0 {
            return Err(ProfileError::InvalidPolicy);
        }
        let count =
            outer.segments.len() + holes.iter().map(|hole| hole.segments.len()).sum::<usize>();
        if count > policy.max_segments as usize {
            return Err(ProfileError::TooManySegments {
                count,
                maximum: policy.max_segments,
            });
        }
        if outer.has_bezier() || holes.iter().any(Loop::has_bezier) {
            let chords = proved_chords(outer, holes, policy)?;
            return arrangement(&chords[0], &chords[1..]);
        }
        let loops: Vec<_> = core::iter::once((LoopRef::Outer, outer))
            .chain(
                holes
                    .iter()
                    .enumerate()
                    .map(|(i, hole)| (LoopRef::Hole(i), hole)),
            )
            .map(|(loop_ref, loop_)| {
                (
                    loop_ref,
                    loop_
                        .pieces()
                        .map(|p| (p.bounds(), p))
                        .map(|(b, p)| (p, b))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for (first_loop, (first_ref, first)) in loops.iter().enumerate() {
            for (second_loop, (second_ref, second)) in loops.iter().enumerate().skip(first_loop) {
                for (i, (p, p_bounds)) in first.iter().enumerate() {
                    let start = if first_loop == second_loop { i + 1 } else { 0 };
                    for (j, (q, q_bounds)) in second.iter().enumerate().skip(start) {
                        if !bounds_may_contact(*p_bounds, *q_bounds, separation) {
                            continue;
                        }
                        // No finer than the rounding of distances at these
                        // two segments' coordinates.
                        let scale = [*p_bounds, *q_bounds]
                            .into_iter()
                            .flatten()
                            .flatten()
                            .fold(0.0_f64, |m, c| m.max(c.abs()));
                        let separation = separation.max(16.0 * f64::EPSILON * scale);
                        let adjacent = first_loop == second_loop
                            && (j == i + 1 || (i == 0 && j == first.len() - 1));
                        let contact = if adjacent {
                            adjacent_contact(p, q, separation)
                        } else {
                            p.distance(q) < separation
                        };
                        if contact {
                            return Err(ProfileError::Contact {
                                first: SegmentRef {
                                    loop_ref: *first_ref,
                                    segment: i,
                                },
                                second: SegmentRef {
                                    loop_ref: *second_ref,
                                    segment: j,
                                },
                            });
                        }
                    }
                }
            }
        }
        arrangement(outer, holes)
    }

    /// Replaces every boundary segment's tag without changing geometry or storage.
    /// Generated transitions can subsequently receive a more specific tag.
    #[must_use]
    pub fn tagged(mut self, tag: SegTag) -> Self {
        for boundary in core::iter::once(&mut self.outer).chain(self.holes.iter_mut()) {
            for segment in &mut boundary.segments {
                segment.tag = Some(tag);
            }
        }
        self
    }

    /// The counter-clockwise outer loop.
    #[must_use]
    pub fn outer(&self) -> &Loop {
        &self.outer
    }

    /// The clockwise holes.
    #[must_use]
    pub fn holes(&self) -> &[Loop] {
        &self.holes
    }

    /// An axis-aligned box containing the profile: its outer loop's bounds
    /// ([`Loop::bounds`]).
    #[must_use]
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        self.outer.bounds()
    }

    /// The first rational Bezier segment, in outer-then-hole order, if
    /// any: a consumer that builds only lines and arcs refuses the profile
    /// there rather than read the segment as a chord.
    #[must_use]
    pub fn bezier_segment(&self) -> Option<SegmentRef> {
        core::iter::once((LoopRef::Outer, &self.outer))
            .chain(
                self.holes
                    .iter()
                    .enumerate()
                    .map(|(i, hole)| (LoopRef::Hole(i), hole)),
            )
            .find_map(|(loop_ref, loop_)| {
                loop_
                    .segments
                    .iter()
                    .position(|s| matches!(s.curve, Curve::Bezier(_)))
                    .map(|segment| SegmentRef { loop_ref, segment })
            })
    }

    /// The outer loop's area less the holes' areas.
    #[must_use]
    pub fn area(&self) -> f64 {
        self.outer.signed_area() + self.holes.iter().map(Loop::signed_area).sum::<f64>()
    }

    /// Area integrals of `(x-origin.x, y-origin.y)`, in cubed model units.
    ///
    /// Includes circular and polynomial Bezier segments in closed form,
    /// rational Bezier segments as quadrature estimates (as
    /// [`Loop::signed_area`]), and subtracts holes. Choosing an
    /// origin near the profile avoids cancellation for far-translated shapes.
    /// Returns `None` for a non-finite origin or an overflowing integral.
    #[must_use]
    pub fn first_moments(&self, origin: [f64; 2]) -> Option<[f64; 2]> {
        if origin.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let mut moment = [0.0; 2];
        for ring in core::iter::once(&self.outer).chain(&self.holes) {
            for (i, (from, segment)) in ring.iter_with_starts().enumerate() {
                let a = sub(from, origin);
                let b = sub(segment.to, origin);
                let chord = cross(a, b) / 6.0;
                for k in 0..2 {
                    moment[k] += (a[k] + b[k]) * chord;
                }
                if let Some(arc) = ring.arc(i) {
                    let area = arc.segment_area();
                    let h = 0.5 * arc.sweep;
                    let mid = arc.start_angle + h;
                    // The circular-segment centroid measured from its chord
                    // midpoint, multiplied by signed area. The small-angle
                    // series avoids subtracting nearly equal terms to get O(h⁵).
                    let radial = if h.abs() < 0.5 {
                        let h2 = h * h;
                        let rh = arc.radius * h;
                        let coefficients = [
                            2.0 / 15.0,
                            -11.0 / 315.0,
                            17.0 / 3780.0,
                            -461.0 / 1247400.0,
                            8303.0 / 389188800.0,
                            -24911.0 / 27243216000.0,
                            168151.0 / 5557616064000.0,
                            -1513361.0 / 1900704693888000.0,
                            7913.0 / 463788509184000.0,
                        ];
                        let factor = coefficients.into_iter().rev().fold(0.0, |p, c| c + h2 * p);
                        rh * rh * rh * h2 * factor
                    } else {
                        let s = libm::sin(h);
                        arc.radius
                            * arc.radius
                            * arc.radius
                            * (s - s * s * s / 3.0 - h * libm::cos(h))
                    };
                    let direction = [libm::cos(mid), libm::sin(mid)];
                    for k in 0..2 {
                        moment[k] += area * (0.5 * a[k] + 0.5 * b[k]) + radial * direction[k];
                    }
                }
                if let Curve::Bezier(bezier) = &segment.curve {
                    // The region between chord and curve, its moments about
                    // its start moved to the origin.
                    let (area, about_start) = BezierPiece {
                        from,
                        to: segment.to,
                        bezier: bezier.clone(),
                    }
                    .segment_integrals();
                    for k in 0..2 {
                        moment[k] += about_start[k] + area * a[k];
                    }
                }
            }
        }
        moment.iter().all(|v| v.is_finite()).then_some(moment)
    }
}

/// The winding of simple, mutually apart loops, then where each hole lies.
fn arrangement(outer: &Loop, holes: &[Loop]) -> Result<(), ProfileError> {
    // Each loop is simple, so its winding is that of its turn at an
    // extreme, which its area's sign gives wherever it outweighs its
    // rounding.
    for (loop_ref, loop_, counter_clockwise) in core::iter::once((LoopRef::Outer, outer, true))
        .chain(
            holes
                .iter()
                .enumerate()
                .map(|(hole, loop_)| (LoopRef::Hole(hole), loop_, false)),
        )
    {
        match loop_.counter_clockwise() {
            Some(winds) if winds == counter_clockwise => {}
            Some(_) => return Err(ProfileError::WrongWinding { loop_ref }),
            None => return Err(ProfileError::UnresolvedWinding { loop_ref }),
        }
    }
    // Loops no longer meet, so one vertex classifies each hole.
    for (hole, loop_) in holes.iter().enumerate() {
        let point = loop_.segments[0].to;
        if outer.winding(point) != 1 {
            return Err(ProfileError::HoleOutside { hole });
        }
        for (within, other) in holes.iter().enumerate() {
            if within != hole && other.winding(point) != 0 {
                return Err(ProfileError::NestedHole { hole, within });
            }
        }
    }
    Ok(())
}

/// A piece of a segment in an enclosure proof.
struct Proved {
    enclosure: BezierEnclosure,
    loop_index: usize,
    segment: usize,
    /// How far the enclosures' ends reach from the segment's stored ends:
    /// a center-defined arc's are boxes about its radii's disagreement.
    slack: f64,
}

/// Proves loops holding Bezier segments simple and apart, and returns
/// polygons through the proof's pieces' ends that wind as the loops do
/// and leave every vertex of the other loops on the same side.
///
/// Every segment is enclosed as rational Bezier pieces: a line as itself,
/// an arc as rational quadratics, a Bezier segment as itself. Pieces are
/// halved until each is monotone in a coordinate, so cannot cross itself;
/// pieces sharing an end are proved to meet only there; other pieces of
/// one segment are proved apart; pieces of two segments meeting at a
/// vertex are proved apart within that corner's radius ([`Corner`]) and
/// must lie within it where they share the vertex; and every other pair is
/// proved apart by at least the separation, no finer than the rounding at
/// their coordinates.
///
/// A center-defined arc's stored ends need not lie on one circle about its
/// center: they lie at distances from it that agree only to rounding. It
/// is enclosed as every reading those radii allow, every circle about the
/// center between the two distances, so its ends are boxes about its
/// stored ends rather than the points themselves. Where it meets another
/// segment, the curves are proved to meet nowhere outside a ball about
/// the vertex, 64 times as wide as those boxes reach (never narrower than
/// `4096 EPSILON` of their coordinates), within which the readings may
/// meet. That ball is part of the vertex.
///
/// The hulls hold the curves and meet only at shared ends and within
/// those balls, so the chords between pieces' ends, each within its
/// piece's hull, wind about every point outside the hulls as the loops do.
fn proved_chords(
    outer: &Loop,
    holes: &[Loop],
    policy: &ProfilePolicy,
) -> Result<Vec<Loop>, ProfileError> {
    let loops: Vec<&Loop> = core::iter::once(outer).chain(holes).collect();
    let reference = |loop_index: usize, segment: usize| SegmentRef {
        loop_ref: if loop_index == 0 {
            LoopRef::Outer
        } else {
            LoopRef::Hole(loop_index - 1)
        },
        segment,
    };
    let mut pieces = Vec::new();
    for (loop_index, loop_) in loops.iter().enumerate() {
        for (segment, (from, s)) in loop_.iter_with_starts().enumerate() {
            let unresolved = || ProfileError::Unresolved {
                first: reference(loop_index, segment),
                second: None,
            };
            let (enclosures, slack) = segment_enclosures(from, s).ok_or_else(unresolved)?;
            pieces.extend(enclosures.into_iter().map(|enclosure| Proved {
                enclosure,
                loop_index,
                segment,
                slack,
            }));
        }
    }
    let corners: Vec<Vec<Corner>> = loops.iter().map(|l| Corner::of(l, policy)).collect();
    let budget = policy.max_segments as usize;
    // Pairs not yet proved, and pieces not yet proved monotone. A proof
    // concerns the curves, so it holds for every part of them: only pairs
    // that failed, and pieces that become neighbours by halving, are
    // tested again.
    let mut pending: Vec<(usize, usize)> = (0..pieces.len())
        .flat_map(|i| (i + 1..pieces.len()).map(move |j| (i, j)))
        .collect();
    let mut unproved: Vec<usize> = (0..pieces.len()).collect();
    for _ in 0..64 {
        let mut halve = alloc::vec![false; pieces.len()];
        let mut failed = None;
        for &i in &unproved {
            if !pieces[i].enclosure.is_monotone() {
                // Pieces halving cannot sharpen.
                if pieces[i].enclosure.is_unresolvable() {
                    return Err(ProfileError::Unresolved {
                        first: reference(pieces[i].loop_index, pieces[i].segment),
                        second: None,
                    });
                }
                halve[i] = true;
                failed.get_or_insert((i, None));
            }
        }
        let (first, last) = loop_ends(&pieces, loops.len());
        let mut failing = Vec::new();
        for &(i, j) in &pending {
            let (a, b) = (&pieces[i], &pieces[j]);
            let (p, q) = (a.enclosure.bounds(), b.enclosure.bounds());
            let scale = [p, q]
                .into_iter()
                .flatten()
                .flatten()
                .fold(0.0_f64, |m, c| m.max(c.abs()));
            let margin = policy.min_separation.max(16.0 * f64::EPSILON * scale);
            if !bounds_may_contact(p, q, margin) {
                continue;
            }
            // Consecutive, or the loop's last and first pieces.
            let adjacent = a.loop_index == b.loop_index
                && (j == i + 1 || (first[a.loop_index] == i && last[a.loop_index] == j));
            // Parts of one segment come as near each other as the curve
            // runs, and parts of two segments as near as their corner
            // makes them within its radius: those need only not touch.
            // The separation holds everywhere else.
            let same = a.loop_index == b.loop_index && a.segment == b.segment;
            let cornered = a.loop_index == b.loop_index
                && !same
                && corners[a.loop_index]
                    .iter()
                    .filter(|c| c.joins(a.segment, b.segment))
                    .any(|c| c.holds(p) && c.holds(q));
            let crosses_corner = adjacent && !same;
            // Where a center-defined arc meets another segment, the
            // pieces meeting there share no exact vertex, only a box about
            // it: they are proved once both lie within the vertex's ball.
            let balled = crosses_corner
                && (a.slack > 0.0 || b.slack > 0.0)
                && corners[a.loop_index]
                    .iter()
                    // The corner they meet at: its vertex in both boxes.
                    .filter(|c| {
                        c.joins(a.segment, b.segment) && within(c.vertex, p) && within(c.vertex, q)
                    })
                    .any(|c| {
                        // Never finer than the coordinates resolve.
                        let ball = Corner {
                            vertex: c.vertex,
                            segments: c.segments,
                            radius: 64.0 * (a.slack + b.slack) + 4096.0 * f64::EPSILON * scale,
                        };
                        ball.holds(p) && ball.holds(q)
                    });
            let proved = if adjacent {
                // Pieces meeting at a corner meet only there, and lie
                // within its radius, past which the separation holds.
                balled
                    || (a.enclosure.separation(&b.enclosure).is_some()
                        && (!crosses_corner || cornered))
            } else {
                let margin = if same || cornered { 0.0 } else { margin };
                a.enclosure.separation_by(&b.enclosure, margin) == Some(HullSeparation::Disjoint)
            };
            if !proved {
                let pair = || ProfileError::Unresolved {
                    first: reference(a.loop_index, a.segment),
                    second: Some(reference(b.loop_index, b.segment)),
                };
                // Points of the two curves already nearer than the
                // separation, outside any corner the two segments share:
                // no subdivision can prove them apart.
                let ends = [a.enclosure.endpoints(), b.enclosure.endpoints()];
                let shared: Vec<&Corner> = if a.loop_index == b.loop_index {
                    corners[a.loop_index]
                        .iter()
                        .filter(|c| c.joins(a.segment, b.segment))
                        .collect()
                } else {
                    Vec::new()
                };
                let clear =
                    |p: [f64; 2]| shared.iter().all(|c| length(sub(p, c.vertex)) > c.radius);
                let violated = ends[0].iter().any(|&p| {
                    ends[1]
                        .iter()
                        .any(|&q| length(sub(p, q)) < margin && clear(p) && clear(q))
                });
                if !same && violated {
                    return Err(pair());
                }
                if a.enclosure.is_unresolvable() || b.enclosure.is_unresolvable() {
                    return Err(pair());
                }
                halve[i] = true;
                halve[j] = true;
                failing.push((i, j));
                // Name a pair of distinct segments where there is one.
                if failed.is_none_or(|(f, g): (usize, Option<usize>)| {
                    g.is_none_or(|g| pieces[f].segment == pieces[g].segment)
                        && a.segment != b.segment
                }) {
                    failed = Some((i, Some(j)));
                }
            }
        }
        let Some((first_failed, second_failed)) = failed else {
            return chords(&pieces, loops.len());
        };
        let unresolved = ProfileError::Unresolved {
            first: reference(
                pieces[first_failed].loop_index,
                pieces[first_failed].segment,
            ),
            second: second_failed.map(|j| reference(pieces[j].loop_index, pieces[j].segment)),
        };
        let more = halve.iter().filter(|h| **h).count();
        if pieces.len() + more > budget {
            return Err(unresolved);
        }
        // Halve, remembering where each old piece's parts went.
        let mut parts = Vec::with_capacity(pieces.len());
        let mut refined = Vec::with_capacity(pieces.len() + more);
        for (piece, halve) in pieces.into_iter().zip(&halve) {
            let at = refined.len();
            if !halve {
                refined.push(piece);
                parts.push(at..at + 1);
                continue;
            }
            let (left, right) = piece.enclosure.bisect().map_err(|_| unresolved)?;
            for enclosure in [left, right] {
                refined.push(Proved { enclosure, ..piece });
            }
            parts.push(at..at + 2);
        }
        pieces = refined;
        // Again: the parts of failed pairs, each halved piece's two parts
        // and its parts beside its old neighbours, which may no longer be
        // their neighbours.
        let (first, last) = loop_ends(&pieces, loops.len());
        let mut next = Vec::new();
        for (i, j) in failing {
            for a in parts[i].clone() {
                for b in parts[j].clone() {
                    next.push((a.min(b), a.max(b)));
                }
            }
        }
        unproved.clear();
        for part in &parts {
            if part.len() < 2 {
                continue;
            }
            unproved.extend(part.clone());
            next.push((part.start, part.start + 1));
            let loop_index = pieces[part.start].loop_index;
            let before = if part.start == first[loop_index] {
                last[loop_index]
            } else {
                part.start - 1
            };
            let after = if part.end - 1 == last[loop_index] {
                first[loop_index]
            } else {
                part.end
            };
            for a in part.clone() {
                for b in [before, after] {
                    if a != b {
                        next.push((a.min(b), a.max(b)));
                    }
                }
            }
        }
        next.sort_unstable();
        next.dedup();
        pending = next;
    }
    Err(ProfileError::Unresolved {
        first: reference(0, 0),
        second: None,
    })
}

/// Where two consecutive segments meet, and how far from there their
/// parts may come nearer than the separation: twice the separation over
/// the sine of the angle between them (any angle past a right angle
/// counting as one), so that past it rays at that angle are a separation
/// apart, nor less than `4096 EPSILON` of their coordinates, which the
/// rounding of the pieces meeting there allows them to reach, but never
/// more than half the shorter segment's chord, so a sliver's far halves
/// are held to the separation however small its angle.
struct Corner {
    vertex: [f64; 2],
    segments: [usize; 2],
    radius: f64,
}

impl Corner {
    fn of(loop_: &Loop, policy: &ProfilePolicy) -> Vec<Self> {
        let pieces: Vec<Piece> = loop_.pieces().collect();
        let n = pieces.len();
        (0..n)
            .map(|k| {
                let (before, after) = (&pieces[k], &pieces[(k + 1) % n]);
                let back = before.tangent(true);
                let ray = after.tangent(false);
                // The rays from the vertex along both segments.
                let (cos, sin) = (-dot(back, ray), cross(back, ray).abs());
                let sine = if cos <= 0.0 { 1.0 } else { sin };
                let scale = before
                    .bounds()
                    .into_iter()
                    .chain(after.bounds())
                    .flatten()
                    .fold(0.0_f64, |m, c| m.max(c.abs()));
                let separation = policy.min_separation.max(16.0 * f64::EPSILON * scale);
                let chord = |p: &Piece| length(sub(p.to(), p.from()));
                let cap = 0.5 * chord(before).min(chord(after));
                // Never finer than subdivision's rounding at these
                // coordinates lets the pieces meeting here fit within it.
                let resolved = 4096.0 * f64::EPSILON * scale;
                let radius = if sine > 0.0 {
                    (2.0 * separation / sine).max(resolved).min(cap)
                } else {
                    cap
                };
                Self {
                    vertex: before.to(),
                    segments: [k, (k + 1) % n],
                    radius,
                }
            })
            .collect()
    }

    /// Whether segments `a` and `b` meet here.
    fn joins(&self, a: usize, b: usize) -> bool {
        self.segments == [a, b] || self.segments == [b, a]
    }

    /// Whether a box lies within the corner's radius of its vertex.
    fn holds(&self, [lo, hi]: [[f64; 2]; 2]) -> bool {
        let far = [0, 1].map(|k| {
            (self.vertex[k] - lo[k])
                .abs()
                .max((hi[k] - self.vertex[k]).abs())
        });
        libm::hypot(far[0], far[1]) <= self.radius
    }
}

/// Whether `point` lies in a box.
fn within(point: [f64; 2], [lo, hi]: [[f64; 2]; 2]) -> bool {
    (0..2).all(|k| lo[k] <= point[k] && point[k] <= hi[k])
}

/// Each loop's first and last piece.
fn loop_ends(pieces: &[Proved], loops: usize) -> (Vec<usize>, Vec<usize>) {
    let mut first = alloc::vec![usize::MAX; loops];
    let mut last = alloc::vec![0; loops];
    for (i, piece) in pieces.iter().enumerate() {
        first[piece.loop_index] = first[piece.loop_index].min(i);
        last[piece.loop_index] = i;
    }
    (first, last)
}

/// A segment's enclosures from `from`, and how far their ends reach from
/// its stored ends, or `None` where they cannot be formed.
fn segment_enclosures(from: [f64; 2], segment: &Segment) -> Option<(Vec<BezierEnclosure>, f64)> {
    let to = segment.to;
    match &segment.curve {
        Curve::Line => Some((
            alloc::vec![BezierEnclosure::new(&[from, to], None).ok()?],
            0.0,
        )),
        Curve::Bezier(bezier) => Some((
            alloc::vec![
                BezierPiece {
                    from,
                    to,
                    bezier: bezier.clone()
                }
                .enclosure()
                .ok()?
            ],
            0.0,
        )),
        Curve::Arc { bulge } => Some((BezierEnclosure::arc(from, to, [*bulge; 2]).ok()?, 0.0)),
        Curve::ArcCenter { center, clockwise } => {
            // Every reading its radii allow, in exact quarter turns and a
            // rest of between an eighth and three eighths of a turn, which
            // its sweep's rounding cannot mistake.
            let arc = Arc::resolve(from, to, &segment.curve).ok()??;
            let turn = arc.sweep.abs() / PI;
            let quarters = [0.75, 1.25, 1.75]
                .into_iter()
                .filter(|&eighths| turn > eighths)
                .count();
            let enclosures = BezierEnclosure::band(*center, from, to, *clockwise, quarters).ok()?;
            let slack = enclosures
                .iter()
                .map(BezierEnclosure::end_spread)
                .fold(0.0, f64::max);
            Some((enclosures, slack))
        }
    }
}

/// Polygons through the ends of each loop's proved pieces, in order.
fn chords(pieces: &[Proved], loops: usize) -> Result<Vec<Loop>, ProfileError> {
    (0..loops)
        .map(|loop_index| {
            let segments = pieces
                .iter()
                .filter(|p| p.loop_index == loop_index)
                .map(|p| Segment::line(p.enclosure.endpoints()[1]))
                .collect();
            Loop::new(segments).map_err(|_| ProfileError::Unresolved {
                first: SegmentRef {
                    loop_ref: if loop_index == 0 {
                        LoopRef::Outer
                    } else {
                        LoopRef::Hole(loop_index - 1)
                    },
                    segment: 0,
                },
                second: None,
            })
        })
        .collect()
}

/// Sine below which adjacent pieces count as tangent at their shared vertex.
const TANGENT_SINE: f64 = 1e-12;

/// Whether adjacent pieces meet anywhere but their shared vertices, or one
/// doubles back along the other.
///
/// Pieces tangent at a shared vertex meet only there: a line tangent to a
/// circle touches it once, and so do two tangent circles. Otherwise their
/// curves meet at the vertex and at most once more, and that other meeting
/// is a crossing wherever it lies on both pieces, however close to the
/// vertex.
fn adjacent_contact(first: &Piece, second: &Piece, separation: f64) -> bool {
    // Each shared vertex, with whether the pieces are tangent there.
    let shared: Vec<([f64; 2], bool)> = [(true, false), (false, true)]
        .into_iter()
        .filter_map(|(first_end, second_end)| {
            let vertex = if first_end { first.to() } else { first.from() };
            let other = if second_end {
                second.to()
            } else {
                second.from()
            };
            (vertex == other).then(|| {
                let (a, b) = (first.tangent(first_end), second.tangent(second_end));
                (vertex, cross(a, b).abs() <= TANGENT_SINE)
            })
        })
        .collect();
    let mut meetings: Vec<[f64; 2]> = first.meetings(second).iter().collect();
    for &(vertex, tangent) in &shared {
        if tangent {
            meetings.clear();
        } else if let Some(nearest) = (0..meetings.len()).min_by(|&a, &b| {
            length(sub(meetings[a], vertex)).total_cmp(&length(sub(meetings[b], vertex)))
        }) {
            meetings.swap_remove(nearest);
        }
    }
    let away = |point: &[f64; 2]| {
        shared
            .iter()
            .all(|(vertex, _)| length(sub(*point, *vertex)) >= separation)
    };
    let probes = |piece: &Piece| [piece.from(), piece.to(), piece.midpoint()];
    meetings
        .iter()
        .any(|&point| first.holds(point) && second.holds(point))
        || probes(second)
            .iter()
            .any(|point| away(point) && first.distance_to(*point) < separation)
        || probes(first)
            .iter()
            .any(|point| away(point) && second.distance_to(*point) < separation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::f64::consts::PI;

    const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);

    fn rect(min: [f64; 2], max: [f64; 2]) -> Loop {
        Loop::new(vec![
            Segment::line([max[0], min[1]]),
            Segment::line(max),
            Segment::line([min[0], max[1]]),
            Segment::line(min),
        ])
        .unwrap()
    }

    /// A counter-clockwise circle as two half arcs starting at its east point.
    fn circle(center: [f64; 2], radius: f64) -> Loop {
        let east = [center[0] + radius, center[1]];
        let west = [center[0] - radius, center[1]];
        Loop::new(vec![Segment::arc(west, 1.0), Segment::arc(east, 1.0)]).unwrap()
    }

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn required_fixtures_validate_with_exact_areas() {
        let disk = Profile::new(circle([0.0, 0.0], 2.0), vec![], &POLICY).unwrap();
        close(disk.area(), 4.0 * PI);
        close(disk.outer().length(), 4.0 * PI);

        let annulus = Profile::new(
            circle([0.0, 0.0], 2.0),
            vec![circle([0.0, 0.0], 1.0).reversed()],
            &POLICY,
        )
        .unwrap();
        close(annulus.area(), 3.0 * PI);

        // Rounded rectangle 6 x 4 with corner radius 1, quarter arcs.
        let q = libm::tan(PI / 8.0);
        let rounded = Loop::new(vec![
            Segment::line([5.0, 0.0]),
            Segment::arc([6.0, 1.0], q),
            Segment::line([6.0, 3.0]),
            Segment::arc([5.0, 4.0], q),
            Segment::line([1.0, 4.0]),
            Segment::arc([0.0, 3.0], q),
            Segment::line([0.0, 1.0]),
            Segment::arc([1.0, 0.0], q),
        ])
        .unwrap();
        let rounded = Profile::new(rounded, vec![], &POLICY).unwrap();
        close(rounded.area(), 24.0 - 4.0 + PI);

        // Mixed: a square with one side replaced by a major arc bulging out.
        let major = libm::tan(3.0 * PI / 8.0);
        let mixed = Loop::new(vec![
            Segment::line([2.0, 0.0]),
            Segment::arc([2.0, 2.0], major),
            Segment::line([0.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        let arc = mixed.arc(1).unwrap();
        close(arc.sweep, 1.5 * PI);
        let mixed = Profile::new(mixed, vec![], &POLICY).unwrap();
        close(mixed.area(), 4.0 + arc.segment_area());

        // A panel with several disjoint holes of both kinds.
        let panel = Profile::new(
            rect([0.0, 0.0], [10.0, 4.0]),
            vec![
                circle([2.0, 2.0], 1.0).reversed(),
                rect([4.0, 1.0], [6.0, 3.0]).reversed(),
                circle([8.0, 2.0], 0.5).reversed(),
            ],
            &POLICY,
        )
        .unwrap();
        close(panel.area(), 40.0 - PI - 4.0 - 0.25 * PI);
    }

    #[test]
    fn bounds_include_swept_axis_extremes_only() {
        assert_eq!(circle([1.0, 2.0], 3.0).bounds(), [[-2.0, -1.0], [4.0, 5.0]]);
        // A quarter arc from east to north reaches no other extreme.
        let quarter = Arc::from_bulge([1.0, 0.0], [0.0, 1.0], libm::tan(PI / 8.0)).unwrap();
        let [min, max] = quarter.bounds();
        assert_eq!((min, max), ([0.0, 0.0], [1.0, 1.0]));
        // The major arc the other way round passes west and south.
        let major = Arc::from_bulge([1.0, 0.0], [0.0, 1.0], -libm::tan(3.0 * PI / 8.0)).unwrap();
        let [min, max] = major.bounds();
        assert!((min[0] + 1.0).abs() < 1e-15 && (min[1] + 1.0).abs() < 1e-15);
        assert_eq!(max, [1.0, 1.0]);
    }

    #[test]
    fn far_offsets_and_tiny_radii_keep_their_arrangement() {
        for offset in [1e6, -1e9] {
            let base = [offset, 2.0 * offset];
            let profile = Profile::new(
                circle(base, 3.0),
                vec![circle(base, 1e-3).reversed()],
                &POLICY,
            )
            .unwrap();
            assert!((profile.area() - (9.0 - 1e-6) * PI).abs() < 1e-6);
        }
        // Boundaries a hair apart stay valid above the separation.
        let close_holes = [
            circle([1.0, 1.0], 0.5).reversed(),
            circle([2.0 + 1e-6, 1.0], 0.5).reversed(),
        ];
        Profile::new(rect([0.0, 0.0], [4.0, 2.0]), close_holes.to_vec(), &POLICY).unwrap();
        assert_eq!(
            Profile::new(
                rect([0.0, 0.0], [4.0, 2.0]),
                close_holes.to_vec(),
                &ProfilePolicy::new(1e-5)
            ),
            Err(ProfileError::Contact {
                first: SegmentRef {
                    loop_ref: LoopRef::Hole(0),
                    segment: 0
                },
                second: SegmentRef {
                    loop_ref: LoopRef::Hole(1),
                    segment: 0
                },
            })
        );
    }

    #[test]
    fn tiny_nested_holes_are_found_without_underflow() {
        // Clockwise diamonds of radii 4s and s about the origin: products
        // of their coordinates underflow, yet the inner lies in the outer.
        let s = 1e-200;
        let diamond = |r: f64| {
            Loop::new(vec![
                Segment::line([0.0, r]),
                Segment::line([r, 0.0]),
                Segment::line([0.0, -r]),
                Segment::line([-r, 0.0]),
            ])
            .unwrap()
        };
        assert_eq!(
            Profile::new(
                rect([-1.0, -1.0], [1.0, 1.0]),
                vec![diamond(4.0 * s), diamond(s)],
                &ProfilePolicy::new(1e-202)
            ),
            Err(ProfileError::NestedHole { hole: 1, within: 0 })
        );
    }

    /// A triangle whose exact area is `-2^-105`, clockwise, though its
    /// floating-point shoelace sums to `+2^-53`.
    fn sliver() -> [[f64; 2]; 3] {
        let s = f64::EPSILON;
        [
            [-1.0 - 2.0 * s, -1.0 - 5.0 * s],
            [-0.5 - 2.0 * s, -0.5 - 4.0 * s],
            [2.0 - s, 2.0 + 2.0 * s],
        ]
    }

    #[test]
    fn a_sliver_thinner_than_its_rounding_winds_as_it_turns() {
        let [a, b, c] = sliver();
        let mut misled = false;
        for vertices in [[a, b, c], [b, c, a], [c, a, b]] {
            let triangle = Loop::new(vertices.map(Segment::line).to_vec()).unwrap();
            misled |= triangle.signed_area() > 0.0;
            assert_eq!(triangle.counter_clockwise(), Some(false));
            assert_eq!(triangle.reversed().counter_clockwise(), Some(true));
        }
        assert!(misled, "some starting vertex's shoelace has the wrong sign");
    }

    #[test]
    fn a_sliver_with_an_arc_is_not_guessed() {
        let [a, b, c] = sliver();
        // The long side bows by far less than the sliver's rounding.
        let triangle = Loop::new(vec![
            Segment::line(b),
            Segment::line(c),
            Segment::arc(a, 1e-40),
        ])
        .unwrap();
        assert_eq!(triangle.counter_clockwise(), None);
    }

    #[test]
    fn contact_finer_than_rounding_is_contact_either_way_round() {
        let [a, b, c] = sliver();
        // The middle vertex is 1e-32 from the long side: below any
        // separation the coordinates resolve, so in contact in both
        // directions, however fine the separation asked for.
        for vertices in [[a, b, c], [c, b, a]] {
            let triangle = Loop::new(vertices.map(Segment::line).to_vec()).unwrap();
            for separation in [1e-20, 1e-12] {
                assert!(matches!(
                    Profile::new(triangle.clone(), vec![], &ProfilePolicy::new(separation)),
                    Err(ProfileError::Contact { .. })
                ));
            }
        }
    }

    #[test]
    fn touching_crossing_and_misplaced_loops_are_refused() {
        let outer = || rect([0.0, 0.0], [4.0, 2.0]);
        let hole = |center, radius| circle(center, radius).reversed();
        // A hole touching the outer boundary, two touching holes, and a hole
        // crossing the outer boundary.
        assert!(matches!(
            Profile::new(outer(), vec![hole([1.0, 1.0], 1.0)], &POLICY),
            Err(ProfileError::Contact { .. })
        ));
        assert!(matches!(
            Profile::new(
                outer(),
                vec![hole([1.0, 1.0], 0.5), hole([2.0, 1.0], 0.5)],
                &POLICY
            ),
            Err(ProfileError::Contact { .. })
        ));
        assert!(matches!(
            Profile::new(outer(), vec![hole([3.8, 1.0], 0.5)], &POLICY),
            Err(ProfileError::Contact { .. })
        ));
        assert_eq!(
            Profile::new(outer(), vec![hole([9.0, 1.0], 0.5)], &POLICY),
            Err(ProfileError::HoleOutside { hole: 0 })
        );
        assert_eq!(
            Profile::new(
                rect([0.0, 0.0], [10.0, 10.0]),
                vec![hole([5.0, 5.0], 1.0), hole([5.0, 5.0], 3.0)],
                &POLICY
            ),
            Err(ProfileError::NestedHole { hole: 0, within: 1 })
        );
        assert_eq!(
            Profile::new(outer().reversed(), vec![], &POLICY),
            Err(ProfileError::WrongWinding {
                loop_ref: LoopRef::Outer
            })
        );
        assert_eq!(
            Profile::new(outer(), vec![circle([1.0, 1.0], 0.5)], &POLICY),
            Err(ProfileError::WrongWinding {
                loop_ref: LoopRef::Hole(0)
            })
        );
    }

    #[test]
    fn self_contact_within_one_loop_is_refused() {
        // A bow tie: nonadjacent edges cross.
        let bow_tie = Loop::new(vec![
            Segment::line([2.0, 2.0]),
            Segment::line([2.0, 0.0]),
            Segment::line([0.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        assert!(matches!(
            Profile::new(bow_tie, vec![], &POLICY),
            Err(ProfileError::Contact { .. } | ProfileError::WrongWinding { .. })
        ));
        // A spike that doubles back along the previous edge.
        let spike = Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::line([2.0, 0.0]),
            Segment::line([2.0, 2.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        assert!(matches!(
            Profile::new(spike, vec![], &POLICY),
            Err(ProfileError::Contact { .. })
        ));
        // An adjacent arc that dips below the previous line and crosses it
        // on the way back up.
        let crossing = Loop::new(vec![
            Segment::line([4.0, 0.0]),
            Segment::arc([2.0, 1.0], -0.8),
            Segment::line([2.0, 3.0]),
            Segment::line([0.0, 3.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        assert!(crossing.signed_area() > 0.0);
        assert_eq!(
            Profile::new(crossing, vec![], &POLICY),
            Err(ProfileError::Contact {
                first: SegmentRef {
                    loop_ref: LoopRef::Outer,
                    segment: 0
                },
                second: SegmentRef {
                    loop_ref: LoopRef::Outer,
                    segment: 1
                },
            })
        );
    }

    /// The bulge of the arc from `start` through `via` to `end`.
    fn bulge_through(start: [f64; 2], via: [f64; 2], end: [f64; 2]) -> f64 {
        let (u, v) = (sub(start, via), sub(end, via));
        let n = length(u) * length(v);
        -(n + u[0] * v[0] + u[1] * v[1]) / cross(u, v)
    }

    #[test]
    fn adjacent_crossings_near_the_shared_vertex_are_refused() {
        // A circle through the corner (1, 0) and (0.9998, 0): its arc leaves
        // the corner just below the previous line, crosses back over it
        // 0.0002 from the corner, and climbs well clear of every other side.
        let corner = [1.0, 0.0];
        let (center, k) = (0.9999, 0.05);
        let radius = libm::hypot(1e-4, k);
        let lowest = [center, k - radius];
        let end = [center - radius, k];
        let bulge = bulge_through(corner, lowest, end);
        let crossing = Loop::new(vec![
            Segment::line(corner),
            Segment::arc(end, bulge),
            Segment::line([0.0, 1.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        assert_eq!(
            Profile::new(crossing, vec![], &ProfilePolicy::new(0.001)),
            Err(ProfileError::Contact {
                first: SegmentRef {
                    loop_ref: LoopRef::Outer,
                    segment: 0
                },
                second: SegmentRef {
                    loop_ref: LoopRef::Outer,
                    segment: 1
                },
            })
        );
        // Tangent corners, where a line runs into an arc or an arc into an
        // arc, meet only at the shared vertex.
        let q = libm::tan(PI / 8.0);
        let rounded = Loop::new(vec![
            Segment::line([5.0, 0.0]),
            Segment::arc([6.0, 1.0], q),
            Segment::arc([7.0, 2.0], -q),
            Segment::line([7.0, 3.0]),
            Segment::line([0.0, 3.0]),
            Segment::line([0.0, 0.0]),
        ])
        .unwrap();
        Profile::new(rounded, vec![], &ProfilePolicy::new(0.5)).unwrap();
    }

    #[test]
    fn loops_reject_structural_defects_and_reverse_exactly() {
        assert_eq!(
            Loop::new(vec![Segment::line([1.0, 0.0])]),
            Err(LoopError::TooFewSegments { count: 1 })
        );
        assert_eq!(
            Loop::new(vec![Segment::line([1.0, 0.0]), Segment::line([1.0, 0.0])]),
            Err(LoopError::ZeroLength { segment: 0 })
        );
        assert_eq!(
            Loop::new(vec![
                Segment::line([f64::NAN, 0.0]),
                Segment::line([0.0, 0.0])
            ]),
            Err(LoopError::NonFinite { segment: 0 })
        );
        assert_eq!(
            Loop::new(vec![
                Segment::arc([1.0, 0.0], 0.0),
                Segment::line([0.0, 0.0])
            ]),
            Err(LoopError::InvalidArc { segment: 0 })
        );
        let tagged = Loop::new(vec![
            Segment::line([2.0, 0.0]).tagged(SegTag(1)),
            Segment::arc([0.0, 0.0], 0.5).tagged(SegTag(2)),
        ])
        .unwrap();
        let reversed = tagged.reversed();
        assert_eq!(
            reversed.segments(),
            [
                Segment::arc([2.0, 0.0], -0.5).tagged(SegTag(2)),
                Segment::line([0.0, 0.0]).tagged(SegTag(1)),
            ]
        );
        assert_eq!(reversed.reversed(), tagged);
        close(reversed.signed_area(), -tagged.signed_area());
    }

    #[test]
    fn shallow_arc_segment_areas_match_their_series() {
        // Area between chord and arc for a sweep of 4 atan(b): for small b the
        // closed form cancels, so compare with the leading series term.
        for bulge in [1e-3, 1e-6, -1e-8] {
            let arc = Arc::from_bulge([0.0, 0.0], [2.0, 0.0], bulge).unwrap();
            let sweep = 4.0 * libm::atan(bulge);
            let expected = arc.radius * arc.radius * sweep.powi(3) / 12.0;
            assert!((arc.segment_area() - expected).abs() <= 1e-6 * expected.abs());
        }
    }

    #[test]
    fn first_moments_include_curved_segments_holes_and_shallow_caps() {
        let disk = |x: f64, y: f64, r: f64| {
            Loop::new(alloc::vec![
                Segment::arc([x - r, y], 1.0),
                Segment::arc([x + r, y], 1.0)
            ])
            .unwrap()
        };
        let origin = [1e8, -1e8];
        let p = Profile::new(
            disk(origin[0], origin[1], 10.0),
            alloc::vec![disk(origin[0] + 2.0, origin[1] + 3.0, 1.0).reversed()],
            &ProfilePolicy::new(1e-8),
        )
        .unwrap();
        let m = p.first_moments(origin).unwrap();
        assert!((m[0] + 2.0 * PI).abs() < 1e-10);
        assert!((m[1] + 3.0 * PI).abs() < 1e-10);
        let h = 1e-4_f64;
        let x = libm::sin(h);
        let cap = Loop::new(alloc::vec![
            Segment::line([x, 0.0]),
            Segment::arc([-x, 0.0], libm::tan(h * 0.5))
        ])
        .unwrap();
        let p = Profile::new(cap, alloc::vec![], &ProfilePolicy::new(1e-15)).unwrap();
        let m = p.first_moments([0.0; 2]).unwrap();
        let expected = 2.0 / 15.0 * h.powi(5);
        assert!(
            (m[1] / expected - 1.0).abs() < 1e-8,
            "shallow cap moment {m:?}"
        );
        assert!(p.first_moments([f64::NAN, 0.0]).is_none());
    }
}

#[cfg(test)]
mod centered_measure_tests {
    use super::*;
    use alloc::vec;
    use core::f64::consts::PI;
    #[test]
    fn directed_major_arc_bounds_area_and_moments_agree_with_sector_integrals() {
        let policy = ProfilePolicy::new(1e-9);
        for (clockwise, sign, magnitude) in [(false, 1.0, PI / 2.0), (true, -1.0, 3.0 * PI / 2.0)] {
            let boundary = Loop::new(vec![
                Segment::line([1.0, 0.0]),
                Segment::arc_centered([0.0, 1.0], [0.0; 2], clockwise),
                Segment::line([0.0; 2]),
            ])
            .unwrap();
            assert!((boundary.signed_area() - sign * magnitude / 2.0).abs() < 1e-15);
            let outer = if clockwise {
                boundary.reversed()
            } else {
                boundary
            };
            let profile = Profile::new(outer, vec![], &policy).unwrap();
            let moments = profile.first_moments([0.0; 2]).unwrap();
            let expected = if clockwise { -1.0 / 3.0 } else { 1.0 / 3.0 };
            for moment in moments {
                assert!((moment - expected).abs() < 1e-15);
            }
            assert_eq!(
                profile.bounds(),
                if clockwise {
                    [[-1.0; 2], [1.0; 2]]
                } else {
                    [[0.0; 2], [1.0; 2]]
                }
            );
        }
    }
}
