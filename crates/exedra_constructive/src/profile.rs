// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Kurbo-backed 2D profiles, closed by construction.
//!
//! A [`Loop2`] is an endpoint-chained cyclic segment list: segment `i` runs
//! from segment `i - 1`'s endpoint to its own (indices modulo the segment
//! count), so a loop is closed *structurally* — no open state exists, and
//! closure is never a tolerance question. Arcs are bulge-parameterized
//! (`bulge = tan(sweep / 4)`): both endpoints are stored exactly, and the
//! center is derived, which keeps loop closure bit-exact where a
//! center-parameterized arc could not.
//!
//! A [`Profile2`] is one outer loop (counter-clockwise) plus hole loops
//! (clockwise). Construction validates structure, finiteness, winding,
//! cheap containment, and that distinct holes do not overlap or touch;
//! deeper geometric validation (self-intersection) happens at
//! discretization, where the triangulator reports violations as typed
//! errors.
//!
//! All curve mathematics routes through [`kurbo`]: conversion to
//! [`kurbo::BezPath`] powers areas, bounding boxes, and interop. The only
//! curve math owned here is the bulge parameterization itself. Note that
//! arcs render into a `BezPath` as cubic approximations — exact for lines
//! and cubics, documented-approximate for arcs — which is fine for winding
//! signs and bounds, while the deterministic evaluation path discretizes
//! arcs directly (never through kurbo's trig).

use alloc::vec::Vec;

use kurbo::{BezPath, PathEl, Point, Shape};

/// Opaque per-segment provenance tag.
///
/// Frontends assign tags (typically indices into a recipe-level source
/// table); this crate round-trips them through discretization and source
/// maps without interpreting them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SegTag(pub u32);

/// Geometry of one segment; the start point is the previous segment's
/// endpoint, the end point lives in [`Seg2::to`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SegKind {
    /// Straight chord to the endpoint.
    Line,
    /// Circular arc to the endpoint.
    ///
    /// `bulge = tan(sweep / 4)`, DXF-style: positive bulge sweeps
    /// counter-clockwise (deviating to the *right* of the chord
    /// direction), negative sweeps clockwise (deviating left). Zero is
    /// invalid — use [`SegKind::Line`]. Magnitude 1 corresponds to a half
    /// circle; full circles need at least two arcs.
    Arc {
        /// Tangent of a quarter of the sweep angle.
        bulge: f64,
    },
    /// Cubic Bézier to the endpoint with two control points.
    Cubic {
        /// First control point.
        c1: Point,
        /// Second control point.
        c2: Point,
    },
    /// A policy-defined segment: the specification underdetermines this
    /// curve, so the frontend chose a realization under a named policy.
    ///
    /// `policy` is an opaque recipe-interned policy reference (for example
    /// `"spec.front-transition@1"`); `realized` is the concrete
    /// geometry chosen under it (never itself policy-defined). Evaluation
    /// discretizes the realization and reports the node as
    /// policy-defined rather than exact — the spec-ambiguity mechanism.
    PolicyTo {
        /// Recipe-interned policy reference.
        policy: crate::ir::PolicyId,
        /// The concrete realization (line, arc, or cubic).
        realized: alloc::boxed::Box<Self>,
    },
}

/// One segment of a loop: geometry, endpoint, and optional provenance tag.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg2 {
    /// Endpoint of this segment (start of the next).
    pub to: Point,
    /// Segment geometry.
    pub kind: SegKind,
    /// Optional frontend-assigned provenance tag.
    pub tag: Option<SegTag>,
}

impl Seg2 {
    /// A line segment to `to`.
    #[must_use]
    pub fn line(to: impl Into<Point>) -> Self {
        Self {
            to: to.into(),
            kind: SegKind::Line,
            tag: None,
        }
    }

    /// An arc segment to `to` with the given bulge.
    #[must_use]
    pub fn arc(to: impl Into<Point>, bulge: f64) -> Self {
        Self {
            to: to.into(),
            kind: SegKind::Arc { bulge },
            tag: None,
        }
    }

    /// A cubic segment to `to` with control points `c1`, `c2`.
    #[must_use]
    pub fn cubic(to: impl Into<Point>, c1: impl Into<Point>, c2: impl Into<Point>) -> Self {
        Self {
            to: to.into(),
            kind: SegKind::Cubic {
                c1: c1.into(),
                c2: c2.into(),
            },
            tag: None,
        }
    }

    /// A policy-defined segment to `to`, realized as `realized`.
    #[must_use]
    pub fn policy(to: impl Into<Point>, policy: crate::ir::PolicyId, realized: SegKind) -> Self {
        Self {
            to: to.into(),
            kind: SegKind::PolicyTo {
                policy,
                realized: alloc::boxed::Box::new(realized),
            },
            tag: None,
        }
    }

    /// Attaches a provenance tag.
    #[must_use]
    pub fn tagged(mut self, tag: SegTag) -> Self {
        self.tag = Some(tag);
        self
    }
}

/// A closed loop: an endpoint-chained cyclic segment list.
///
/// Segment `i` starts at segment `i - 1`'s endpoint (cyclically), so the
/// loop is closed by construction. Loops are validated at creation; see
/// [`Loop2::new`].
#[derive(Clone, Debug, PartialEq)]
pub struct Loop2 {
    segs: Vec<Seg2>,
}

impl Loop2 {
    /// Creates a validated loop from its segments.
    ///
    /// Requirements: at least two segments; every coordinate finite (with
    /// `-0.0` normalized to `0.0`); consecutive endpoints distinct (no
    /// zero-length chords); arc bulges finite and nonzero.
    ///
    /// # Errors
    ///
    /// Returns the first violated requirement.
    pub fn new(mut segs: Vec<Seg2>) -> Result<Self, ProfileError> {
        if segs.len() < 2 {
            return Err(ProfileError::TooFewSegments { count: segs.len() });
        }
        if segs.len() > u32::MAX as usize {
            return Err(ProfileError::TooManySegments);
        }
        for (index, seg) in segs.iter_mut().enumerate() {
            normalize_point(&mut seg.to);
            if !seg.to.is_finite() {
                return Err(ProfileError::NonFinite { seg: index });
            }
            match &mut seg.kind {
                SegKind::Line => {}
                SegKind::Arc { bulge } => {
                    if !bulge.is_finite() || *bulge == 0.0 {
                        return Err(ProfileError::InvalidBulge { seg: index });
                    }
                }
                SegKind::Cubic { c1, c2 } => {
                    normalize_point(c1);
                    normalize_point(c2);
                    if !c1.is_finite() || !c2.is_finite() {
                        return Err(ProfileError::NonFinite { seg: index });
                    }
                }
                SegKind::PolicyTo {
                    policy: _,
                    realized,
                } => match realized.as_mut() {
                    SegKind::Line => {}
                    SegKind::Arc { bulge } => {
                        if !bulge.is_finite() || *bulge == 0.0 {
                            return Err(ProfileError::InvalidBulge { seg: index });
                        }
                    }
                    SegKind::Cubic { c1, c2 } => {
                        normalize_point(c1);
                        normalize_point(c2);
                        if !c1.is_finite() || !c2.is_finite() {
                            return Err(ProfileError::NonFinite { seg: index });
                        }
                    }
                    SegKind::PolicyTo { .. } => {
                        return Err(ProfileError::NestedPolicy { seg: index });
                    }
                },
            }
        }
        for index in 0..segs.len() {
            let start = segs[(index + segs.len() - 1) % segs.len()].to;
            if start == segs[index].to {
                return Err(ProfileError::ZeroLengthChord { seg: index });
            }
        }
        Ok(Self { segs })
    }

    /// The segments in order.
    #[must_use]
    pub fn segs(&self) -> &[Seg2] {
        &self.segs
    }

    /// Iterates `(start, segment)` pairs; each segment's start is the
    /// previous segment's endpoint.
    pub fn iter_with_starts(&self) -> impl Iterator<Item = (Point, &Seg2)> + '_ {
        let n = self.segs.len();
        (0..n).map(move |i| (self.segs[(i + n - 1) % n].to, &self.segs[i]))
    }

    /// This loop with reversed orientation.
    ///
    /// Segment order reverses, arc bulges negate, cubic control points
    /// swap; tags travel with their segments. Reversal is the *only*
    /// orientation change — the loop is never re-rooted, so tag-to-geometry
    /// association is preserved exactly.
    #[must_use]
    pub fn reversed(&self) -> Self {
        let n = self.segs.len();
        let segs = (0..n)
            .map(|i| {
                // Reversed segment i covers the same geometry as forward
                // segment n-1-i, traversed backwards: it ends at that
                // segment's start point.
                let src = &self.segs[n - 1 - i];
                let to = self.segs[(n - i + n - 2) % n].to;
                let kind = reverse_kind(&src.kind);
                Seg2 {
                    to,
                    kind,
                    tag: src.tag,
                }
            })
            .collect();
        Self { segs }
    }

    /// Renders the loop as a closed [`BezPath`].
    ///
    /// Lines and cubics convert exactly; arcs are approximated by cubics
    /// via [`kurbo::Arc`]. Suitable for areas, bounding boxes, and
    /// interop — not for the deterministic discretization path.
    #[must_use]
    pub fn to_bez_path(&self) -> BezPath {
        let n = self.segs.len();
        let start = self.segs[n - 1].to;
        let mut path = BezPath::new();
        path.move_to(start);
        for (from, seg) in self.iter_with_starts() {
            append_kind(&mut path, from, seg.to, &seg.kind);
        }
        path.close_path();
        path
    }

    /// Builds a loop from a single closed `BezPath` subpath.
    ///
    /// Lines and cubics convert exactly; quadratics are elevated to cubics.
    /// The path must contain exactly one subpath, explicitly closed.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::UnsupportedPath`] for multi-subpath or
    /// unclosed inputs, and loop-validation errors for degenerate geometry.
    pub fn try_from_bez_path(path: &BezPath) -> Result<Self, ProfileError> {
        let mut segs: Vec<Seg2> = Vec::new();
        let mut start: Option<Point> = None;
        let mut current: Option<Point> = None;
        let mut closed = false;
        for el in path.elements() {
            if closed {
                return Err(ProfileError::UnsupportedPath);
            }
            match *el {
                PathEl::MoveTo(p) => {
                    if start.is_some() {
                        return Err(ProfileError::UnsupportedPath);
                    }
                    start = Some(p);
                    current = Some(p);
                }
                PathEl::LineTo(p) => {
                    current = Some(p);
                    segs.push(Seg2::line(p));
                }
                PathEl::QuadTo(c, p) => {
                    let from = current.ok_or(ProfileError::UnsupportedPath)?;
                    // Degree elevation: exact cubic equivalent.
                    let c1 = from + (c - from) * (2.0 / 3.0);
                    let c2 = p + (c - p) * (2.0 / 3.0);
                    current = Some(p);
                    segs.push(Seg2::cubic(p, c1, c2));
                }
                PathEl::CurveTo(c1, c2, p) => {
                    current = Some(p);
                    segs.push(Seg2::cubic(p, c1, c2));
                }
                PathEl::ClosePath => {
                    let (Some(s), Some(c)) = (start, current) else {
                        return Err(ProfileError::UnsupportedPath);
                    };
                    if c != s {
                        segs.push(Seg2::line(s));
                    }
                    closed = true;
                }
            }
        }
        if !closed {
            return Err(ProfileError::UnsupportedPath);
        }
        Self::new(segs)
    }

    /// Signed area of the loop (positive = counter-clockwise), computed via
    /// the [`BezPath`] rendering — approximate for arcs, exact for lines
    /// and cubics; the sign is reliable for valid loops.
    #[must_use]
    pub fn signed_area(&self) -> f64 {
        self.to_bez_path().area()
    }
}

fn append_kind(path: &mut BezPath, from: Point, to: Point, kind: &SegKind) {
    match kind {
        SegKind::Line => path.line_to(to),
        SegKind::Cubic { c1, c2 } => path.curve_to(*c1, *c2, to),
        SegKind::Arc { bulge } => append_bulge_arc(path, from, to, *bulge),
        SegKind::PolicyTo {
            policy: _,
            realized,
        } => append_kind(path, from, to, realized),
    }
}

fn reverse_kind(kind: &SegKind) -> SegKind {
    match kind {
        SegKind::Line => SegKind::Line,
        SegKind::Arc { bulge } => SegKind::Arc { bulge: -bulge },
        SegKind::Cubic { c1, c2 } => SegKind::Cubic { c1: *c2, c2: *c1 },
        SegKind::PolicyTo { policy, realized } => SegKind::PolicyTo {
            policy: *policy,
            realized: alloc::boxed::Box::new(reverse_kind(realized)),
        },
    }
}

/// Appends the bulge arc `from -> to` to `path` as kurbo cubics.
///
/// The center and radius derive from the chord and bulge with only
/// arithmetic and one square root; the cubic rendering goes through
/// [`kurbo::Arc`] and is approximate (interop only — the deterministic
/// discretization path never uses it).
fn append_bulge_arc(path: &mut BezPath, from: Point, to: Point, bulge: f64) {
    let (center, radius, start_angle, sweep) = bulge_arc_parts(from, to, bulge);
    let arc = kurbo::Arc {
        center,
        radii: kurbo::Vec2::new(radius, radius),
        start_angle,
        sweep_angle: sweep,
        x_rotation: 0.0,
    };
    // Radius-relative tolerance: this rendering only feeds areas, bounds,
    // and interop, and a relative bound keeps the subdivision count small
    // and scale-independent — a hostile 1e300-radius arc must not explode
    // validation into astronomically many cubics.
    let tolerance = (radius * 1e-9).max(1e-12);
    arc.to_cubic_beziers(tolerance, |c1, c2, p| {
        path.curve_to(c1, c2, p);
    });
}

/// Derives `(center, radius, start_angle, sweep)` for a bulge arc.
///
/// Pure arithmetic plus `sqrt` and `atan2`; used for interop rendering.
/// The deterministic discretization path re-derives what it needs with
/// libm-only calls.
pub(crate) fn bulge_arc_parts(from: Point, to: Point, bulge: f64) -> (Point, f64, f64, f64) {
    let chord = to - from;
    let half_chord = 0.5 * chord.hypot();
    // sagitta = bulge * half chord; radius from the intersecting chords
    // theorem: r = (h² + s²) / (2 s).
    let sagitta = bulge * half_chord;
    let radius = (half_chord * half_chord + sagitta * sagitta) / (2.0 * sagitta);
    let mid = from.midpoint(to);
    // Unit normal to the left of the chord.
    let norm = kurbo::Vec2::new(-chord.y, chord.x) * (0.5 / half_chord);
    let center = mid + norm * (radius - sagitta);
    let start = from - center;
    let start_angle = start.atan2();
    // sweep = 4 * atan(bulge), positive counter-clockwise.
    let sweep = 4.0 * libm::atan(bulge);
    (center, radius.abs(), start_angle, sweep)
}

/// A profile: one counter-clockwise outer loop plus clockwise hole loops.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile2 {
    outer: Loop2,
    holes: Vec<Loop2>,
}

impl Profile2 {
    /// Creates a validated profile.
    ///
    /// The outer loop must wind counter-clockwise and each hole clockwise
    /// (checked by signed area; use [`Loop2::reversed`] to fix winding
    /// deliberately — orientation is never changed implicitly). Each hole's
    /// bounding box must lie within the outer loop's bounding box (a cheap
    /// necessary condition; exact containment is verified at
    /// discretization). Distinct holes must not overlap, contain one
    /// another, or touch. Curved boundaries are compared through kurbo's
    /// cubic path representation, flattened to a relative `1e-9` tolerance.
    ///
    /// # Errors
    ///
    /// Returns the first violated requirement.
    pub fn new(outer: Loop2, holes: Vec<Loop2>) -> Result<Self, ProfileError> {
        if outer.signed_area() <= 0.0 {
            return Err(ProfileError::WrongWinding { hole: None });
        }
        let outer_bbox = outer.to_bez_path().bounding_box();
        for (index, hole) in holes.iter().enumerate() {
            if hole.signed_area() >= 0.0 {
                return Err(ProfileError::WrongWinding { hole: Some(index) });
            }
            let hole_bbox = hole.to_bez_path().bounding_box();
            if !outer_bbox.union(hole_bbox).same_as(outer_bbox) {
                return Err(ProfileError::HoleOutsideOuter { hole: index });
            }
        }
        for first in 0..holes.len() {
            for second in first + 1..holes.len() {
                if loops_conflict(&holes[first], &holes[second]) {
                    return Err(ProfileError::OverlappingHoles { first, second });
                }
            }
        }
        Ok(Self { outer, holes })
    }

    /// A profile with no holes.
    ///
    /// # Errors
    ///
    /// Same requirements as [`Profile2::new`].
    pub fn simple(outer: Loop2) -> Result<Self, ProfileError> {
        Self::new(outer, Vec::new())
    }

    /// The outer loop.
    #[must_use]
    pub fn outer(&self) -> &Loop2 {
        &self.outer
    }

    /// The hole loops.
    #[must_use]
    pub fn holes(&self) -> &[Loop2] {
        &self.holes
    }
}

/// Extension for comparing kurbo rects exactly.
trait RectExt {
    fn same_as(&self, other: Self) -> bool;
}

impl RectExt for kurbo::Rect {
    fn same_as(&self, other: Self) -> bool {
        self.x0 == other.x0 && self.y0 == other.y0 && self.x1 == other.x1 && self.y1 == other.y1
    }
}

fn loops_conflict(first: &Loop2, second: &Loop2) -> bool {
    let first_path = first.to_bez_path();
    let second_path = second.to_bez_path();
    let first_bounds = first_path.bounding_box();
    let second_bounds = second_path.bounding_box();
    if first_bounds.x1 < second_bounds.x0
        || second_bounds.x1 < first_bounds.x0
        || first_bounds.y1 < second_bounds.y0
        || second_bounds.y1 < first_bounds.y0
    {
        return false;
    }

    let bounds = first_bounds.union(second_bounds);
    let scale = bounds.width().abs().max(bounds.height().abs());
    let tolerance = (scale * 1e-9).max(f64::MIN_POSITIVE);
    let first_points = flattened_ring(&first_path, tolerance);
    let second_points = flattened_ring(&second_path, tolerance);
    if rings_intersect(&first_points, &second_points) {
        return true;
    }

    second_path.contains(first_points[0]) || first_path.contains(second_points[0])
}

fn flattened_ring(path: &BezPath, tolerance: f64) -> Vec<Point> {
    let mut points = Vec::new();
    kurbo::flatten(path.iter(), tolerance, |element| match element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => points.push(point),
        PathEl::ClosePath => {}
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {
            unreachable!("kurbo::flatten emits only lines")
        }
    });
    if points.first() == points.last() {
        points.pop();
    }
    points
}

fn rings_intersect(first: &[Point], second: &[Point]) -> bool {
    first.iter().enumerate().any(|(first_index, &first_start)| {
        let first_end = first[(first_index + 1) % first.len()];
        second
            .iter()
            .enumerate()
            .any(|(second_index, &second_start)| {
                let second_end = second[(second_index + 1) % second.len()];
                segments_intersect(first_start, first_end, second_start, second_end)
            })
    })
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_c = cross(a, b, c);
    let ab_d = cross(a, b, d);
    let cd_a = cross(c, d, a);
    let cd_b = cross(c, d, b);
    ((ab_c > 0.0 && ab_d < 0.0) || (ab_c < 0.0 && ab_d > 0.0))
        && ((cd_a > 0.0 && cd_b < 0.0) || (cd_a < 0.0 && cd_b > 0.0))
        || ab_c == 0.0 && point_on_segment(c, a, b)
        || ab_d == 0.0 && point_on_segment(d, a, b)
        || cd_a == 0.0 && point_on_segment(a, c, d)
        || cd_b == 0.0 && point_on_segment(b, c, d)
}

fn cross(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn point_on_segment(point: Point, start: Point, end: Point) -> bool {
    point.x >= start.x.min(end.x)
        && point.x <= start.x.max(end.x)
        && point.y >= start.y.min(end.y)
        && point.y <= start.y.max(end.y)
}

fn normalize_point(p: &mut Point) {
    if p.x == 0.0 {
        p.x = 0.0;
    }
    if p.y == 0.0 {
        p.y = 0.0;
    }
}

/// Typed profile-construction failure.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// A loop needs at least two segments.
    TooFewSegments {
        /// Number of segments provided.
        count: usize,
    },
    /// Segment count exceeds the `u32` budget.
    TooManySegments,
    /// A coordinate or control point is NaN or infinite.
    NonFinite {
        /// Index of the offending segment.
        seg: usize,
    },
    /// An arc bulge is zero, NaN, or infinite.
    InvalidBulge {
        /// Index of the offending segment.
        seg: usize,
    },
    /// Consecutive segment endpoints coincide.
    ZeroLengthChord {
        /// Index of the zero-length segment.
        seg: usize,
    },
    /// A loop winds the wrong way: outer must be counter-clockwise, holes
    /// clockwise. Fix deliberately with [`Loop2::reversed`].
    WrongWinding {
        /// Hole index, or `None` for the outer loop.
        hole: Option<usize>,
    },
    /// A hole's bounding box escapes the outer loop's bounding box.
    HoleOutsideOuter {
        /// Index of the offending hole.
        hole: usize,
    },
    /// Two hole interiors overlap, one contains the other, or their
    /// boundaries touch.
    OverlappingHoles {
        /// Index of the first conflicting hole.
        first: usize,
        /// Index of the second conflicting hole.
        second: usize,
    },
    /// A `BezPath` conversion input was not a single closed subpath of
    /// supported elements.
    UnsupportedPath,
    /// A builder dimension is zero, negative, non-finite, or inconsistent
    /// (for example a corner radius that does not fit).
    InvalidDimension,
    /// A policy-defined segment's realization is itself policy-defined.
    NestedPolicy {
        /// Index of the offending segment.
        seg: usize,
    },
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooFewSegments { count } => {
                write!(f, "loop needs at least two segments, got {count}")
            }
            Self::TooManySegments => write!(f, "segment count exceeds u32 budget"),
            Self::NonFinite { seg } => write!(f, "segment {seg} has a non-finite coordinate"),
            Self::InvalidBulge { seg } => {
                write!(f, "segment {seg} has a zero or non-finite bulge")
            }
            Self::ZeroLengthChord { seg } => write!(f, "segment {seg} has zero length"),
            Self::WrongWinding { hole: None } => {
                write!(f, "outer loop must wind counter-clockwise")
            }
            Self::WrongWinding { hole: Some(i) } => write!(f, "hole {i} must wind clockwise"),
            Self::HoleOutsideOuter { hole } => {
                write!(f, "hole {hole} bounding box escapes the outer loop")
            }
            Self::OverlappingHoles { first, second } => {
                write!(f, "holes {first} and {second} overlap or touch")
            }
            Self::UnsupportedPath => {
                write!(
                    f,
                    "path is not a single closed subpath of supported elements"
                )
            }
            Self::InvalidDimension => {
                write!(
                    f,
                    "builder dimension is non-positive, non-finite, or inconsistent"
                )
            }
            Self::NestedPolicy { seg } => {
                write!(f, "segment {seg}: policy realizations must be concrete")
            }
        }
    }
}

impl core::error::Error for ProfileError {}

/// Deterministic canonical byte encoding.
///
/// The encoding — explicit tags, `f64::to_bits` little-endian, `u32`
/// little-endian lengths — is the compatibility contract for content
/// hashing. `-0.0` never appears (normalized at validation), NaN never
/// appears (rejected at validation), so equal values encode to equal bytes.
pub trait CanonBytes {
    /// Appends this value's canonical bytes to `out`.
    fn canon_bytes(&self, out: &mut Vec<u8>);
}

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

impl CanonBytes for Point {
    fn canon_bytes(&self, out: &mut Vec<u8>) {
        put_f64(out, self.x);
        put_f64(out, self.y);
    }
}

impl CanonBytes for Seg2 {
    fn canon_bytes(&self, out: &mut Vec<u8>) {
        match &self.kind {
            SegKind::Line => out.push(0),
            SegKind::Arc { bulge } => {
                out.push(1);
                put_f64(out, *bulge);
            }
            SegKind::Cubic { c1, c2 } => {
                out.push(2);
                c1.canon_bytes(out);
                c2.canon_bytes(out);
            }
            SegKind::PolicyTo { policy, realized } => {
                out.push(3);
                put_u32(out, policy.0);
                let inner = Self {
                    to: self.to,
                    kind: (**realized).clone(),
                    tag: None,
                };
                inner.canon_bytes(out);
            }
        }
        self.to.canon_bytes(out);
        match self.tag {
            None => out.push(0),
            Some(SegTag(t)) => {
                out.push(1);
                put_u32(out, t);
            }
        }
    }
}

impl CanonBytes for Loop2 {
    fn canon_bytes(&self, out: &mut Vec<u8>) {
        put_u32(out, crate::len_u32(self.segs.len()));
        for seg in &self.segs {
            seg.canon_bytes(out);
        }
    }
}

impl CanonBytes for Profile2 {
    fn canon_bytes(&self, out: &mut Vec<u8>) {
        self.outer.canon_bytes(out);
        put_u32(out, crate::len_u32(self.holes.len()));
        for hole in &self.holes {
            hole.canon_bytes(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn unit_square() -> Loop2 {
        Loop2::new(vec![
            Seg2::line((1.0, 0.0)),
            Seg2::line((1.0, 1.0)),
            Seg2::line((0.0, 1.0)),
            Seg2::line((0.0, 0.0)),
        ])
        .expect("valid square")
    }

    #[test]
    fn loops_close_by_construction() {
        let square = unit_square();
        let (first_start, _) = square.iter_with_starts().next().expect("nonempty");
        assert_eq!(first_start, Point::new(0.0, 0.0));
        assert_eq!(square.segs().len(), 4);
    }

    #[test]
    fn validation_rejects_degenerate_loops() {
        assert_eq!(
            Loop2::new(vec![Seg2::line((1.0, 0.0))]),
            Err(ProfileError::TooFewSegments { count: 1 })
        );
        assert_eq!(
            Loop2::new(vec![
                Seg2::line((1.0, 0.0)),
                Seg2::line((1.0, 0.0)),
                Seg2::line((0.0, 0.0)),
            ]),
            Err(ProfileError::ZeroLengthChord { seg: 1 })
        );
        assert_eq!(
            Loop2::new(vec![
                Seg2::line((f64::NAN, 0.0)),
                Seg2::line((1.0, 1.0)),
                Seg2::line((0.0, 0.0)),
            ]),
            Err(ProfileError::NonFinite { seg: 0 })
        );
        assert_eq!(
            Loop2::new(vec![
                Seg2::arc((1.0, 0.0), 0.0),
                Seg2::line((0.0, 0.0)),
                Seg2::line((0.0, 1.0)),
            ]),
            Err(ProfileError::InvalidBulge { seg: 0 })
        );
    }

    #[test]
    fn signed_area_and_winding() {
        let square = unit_square();
        assert!((square.signed_area() - 1.0).abs() < 1e-12);
        assert!(square.reversed().signed_area() < 0.0);

        assert!(Profile2::simple(square.clone()).is_ok());
        assert_eq!(
            Profile2::simple(square.reversed()),
            Err(ProfileError::WrongWinding { hole: None })
        );
    }

    #[test]
    fn profile_hole_winding_and_bounds() {
        let outer = unit_square();
        let hole_ccw = Loop2::new(vec![
            Seg2::line((0.6, 0.4)),
            Seg2::line((0.6, 0.6)),
            Seg2::line((0.4, 0.6)),
            Seg2::line((0.4, 0.4)),
        ])
        .expect("valid hole");
        assert_eq!(
            Profile2::new(outer.clone(), vec![hole_ccw.clone()]),
            Err(ProfileError::WrongWinding { hole: Some(0) })
        );
        let hole_cw = hole_ccw.reversed();
        assert!(Profile2::new(outer.clone(), vec![hole_cw]).is_ok());

        let escaping = Loop2::new(vec![
            Seg2::line((3.0, 0.4)),
            Seg2::line((3.0, 0.6)),
            Seg2::line((2.4, 0.6)),
            Seg2::line((2.4, 0.4)),
        ])
        .expect("valid loop")
        .reversed();
        assert_eq!(
            Profile2::new(outer, vec![escaping]),
            Err(ProfileError::HoleOutsideOuter { hole: 0 })
        );
    }

    #[test]
    fn profile_rejects_overlapping_contained_and_touching_holes() {
        fn hole(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop2 {
            Loop2::new(vec![
                Seg2::line((x0, y0)),
                Seg2::line((x0, y1)),
                Seg2::line((x1, y1)),
                Seg2::line((x1, y0)),
            ])
            .expect("valid clockwise rectangle")
        }

        let outer = Loop2::new(vec![
            Seg2::line((0.0, 0.0)),
            Seg2::line((10.0, 0.0)),
            Seg2::line((10.0, 10.0)),
            Seg2::line((0.0, 10.0)),
        ])
        .expect("valid outer loop");
        let cases = [
            (hole(1.0, 1.0, 4.0, 4.0), hole(3.0, 3.0, 6.0, 6.0)),
            (hole(1.0, 1.0, 5.0, 5.0), hole(2.0, 2.0, 3.0, 3.0)),
            (hole(1.0, 1.0, 3.0, 3.0), hole(3.0, 1.0, 5.0, 3.0)),
        ];
        for (first, second) in cases {
            assert_eq!(
                Profile2::new(outer.clone(), vec![first, second]),
                Err(ProfileError::OverlappingHoles {
                    first: 0,
                    second: 1,
                })
            );
        }

        assert!(
            Profile2::new(
                outer,
                vec![hole(1.0, 1.0, 3.0, 3.0), hole(4.0, 1.0, 6.0, 3.0)]
            )
            .is_ok()
        );
    }

    #[test]
    fn profile_rejects_overlapping_curved_holes() {
        fn circular_hole(center_x: f64, radius: f64) -> Loop2 {
            Loop2::new(vec![
                Seg2::arc((center_x + radius, 0.0), 1.0),
                Seg2::arc((center_x - radius, 0.0), 1.0),
            ])
            .expect("valid circle")
            .reversed()
        }

        let outer = Loop2::new(vec![
            Seg2::line((-5.0, -5.0)),
            Seg2::line((5.0, -5.0)),
            Seg2::line((5.0, 5.0)),
            Seg2::line((-5.0, 5.0)),
        ])
        .expect("valid outer loop");
        assert_eq!(
            Profile2::new(
                outer,
                vec![circular_hole(-0.75, 1.0), circular_hole(0.75, 1.0)]
            ),
            Err(ProfileError::OverlappingHoles {
                first: 0,
                second: 1,
            })
        );
    }

    #[test]
    fn reversal_preserves_geometry_and_tags() {
        let tagged = Loop2::new(vec![
            Seg2::line((2.0, 0.0)).tagged(SegTag(10)),
            Seg2::arc((2.0, 2.0), 0.5).tagged(SegTag(11)),
            Seg2::line((0.0, 2.0)).tagged(SegTag(12)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(13)),
        ])
        .expect("valid loop");
        let reversed = tagged.reversed();
        let double = reversed.reversed();
        assert_eq!(tagged, double, "double reversal is identity");
        // The arc keeps its tag and negates its bulge.
        let arc = reversed
            .segs()
            .iter()
            .find(|s| matches!(s.kind, SegKind::Arc { .. }))
            .expect("arc survives");
        assert_eq!(arc.tag, Some(SegTag(11)));
        assert!(matches!(arc.kind, SegKind::Arc { bulge } if bulge == -0.5));
        // Area negates.
        assert!((tagged.signed_area() + reversed.signed_area()).abs() < 1e-9);
    }

    #[test]
    fn arc_loop_area_is_plausible() {
        // Two half-circle arcs (bulge = 1) over a unit-diameter chord make
        // a full circle of radius 0.5: area = pi / 4.
        let circle = Loop2::new(vec![Seg2::arc((1.0, 0.0), 1.0), Seg2::arc((0.0, 0.0), 1.0)])
            .expect("valid two-arc circle");
        let area = circle.signed_area();
        assert!(
            (area - core::f64::consts::FRAC_PI_4).abs() < 1e-6,
            "two-arc circle area {area} should be ~pi/4"
        );
    }

    #[test]
    fn bez_path_round_trip_for_polyline_loops() {
        let square = unit_square();
        let path = square.to_bez_path();
        let back = Loop2::try_from_bez_path(&path).expect("round trip");
        // The path rendering starts from the last endpoint, so segments
        // come back in the same order with the same endpoints.
        assert_eq!(
            back.segs().iter().map(|s| s.to).collect::<Vec<_>>(),
            square.segs().iter().map(|s| s.to).collect::<Vec<_>>()
        );
    }

    #[test]
    fn canon_bytes_are_stable() {
        let mut bytes = Vec::new();
        unit_square().canon_bytes(&mut bytes);
        let mut again = Vec::new();
        unit_square().canon_bytes(&mut again);
        assert_eq!(bytes, again);
        // -0.0 normalizes at validation, so it hashes like +0.0.
        let negzero = Loop2::new(vec![
            Seg2::line((1.0, 0.0)),
            Seg2::line((1.0, 1.0)),
            Seg2::line((-0.0, 1.0)),
            Seg2::line((-0.0, -0.0)),
        ])
        .expect("valid loop");
        let mut nz = Vec::new();
        negzero.canon_bytes(&mut nz);
        assert_eq!(bytes, nz, "-0.0 must canonicalize to +0.0");
    }

    #[test]
    fn golden_canon_bytes_prefix() {
        // Compatibility pin for the encoding: tag byte 0 (line), endpoint
        // (1.0, 0.0) as little-endian f64 bits, no tag.
        let mut bytes = Vec::new();
        unit_square().canon_bytes(&mut bytes);
        assert_eq!(bytes[..4], [4, 0, 0, 0], "segment count u32 LE");
        assert_eq!(bytes[4], 0, "line discriminant");
        assert_eq!(
            bytes[5..13],
            1.0_f64.to_bits().to_le_bytes(),
            "x coordinate bits"
        );
    }
}
