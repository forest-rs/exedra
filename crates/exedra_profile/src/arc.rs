// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Circular arcs derived from bulge segments, and distances between pieces.

use core::f64::consts::{PI, TAU};

/// A circular arc derived from a bulge segment.
///
/// The endpoints stay authoritative: [`Arc::from_bulge`] derives the center,
/// radius and angles from them, and sampling emits them unchanged.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Arc {
    /// Start point.
    pub from: [f64; 2],
    /// End point.
    pub to: [f64; 2],
    /// Circle center.
    pub center: [f64; 2],
    /// Circle radius; positive.
    pub radius: f64,
    /// Angle of `from` about the center, in radians.
    pub start_angle: f64,
    /// Signed sweep in radians, counter-clockwise positive, `0 < |sweep| < 2 pi`.
    pub sweep: f64,
}

impl Arc {
    /// Derives the arc from `from` to `to` with `bulge = tan(sweep / 4)`.
    ///
    /// Returns `None` for coincident endpoints, a zero or non-finite bulge, or
    /// geometry that overflows `f64`.
    #[must_use]
    pub fn from_bulge(from: [f64; 2], to: [f64; 2], bulge: f64) -> Option<Self> {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let chord = libm::hypot(dx, dy);
        if !bulge.is_finite() || bulge == 0.0 || !chord.is_finite() || chord == 0.0 {
            return None;
        }
        let half_chord = 0.5 * chord;
        let reciprocal = 1.0 / bulge;
        let radius = (half_chord * 0.5 * (bulge + reciprocal)).abs();
        // The center sits on the chord's left normal, offset by the signed
        // radius less the sagitta; the bulge sign selects the side.
        let offset = half_chord * 0.5 * (reciprocal - bulge);
        let center = [
            from[0] + 0.5 * dx - dy / chord * offset,
            from[1] + 0.5 * dy + dx / chord * offset,
        ];
        let sweep = 4.0 * libm::atan(bulge);
        let start_angle = libm::atan2(from[1] - center[1], from[0] - center[0]);
        let finite = radius.is_finite() && radius > 0.0 && center.iter().all(|v| v.is_finite());
        finite.then_some(Self {
            from,
            to,
            center,
            radius,
            start_angle,
            sweep,
        })
    }

    /// The point at fraction `t` of the sweep, with the authored endpoints at
    /// `t = 0` and `t = 1`.
    #[must_use]
    pub fn point_at(&self, t: f64) -> [f64; 2] {
        if t == 0.0 {
            return self.from;
        }
        if t == 1.0 {
            return self.to;
        }
        let angle = self.start_angle + t * self.sweep;
        [
            self.center[0] + self.radius * libm::cos(angle),
            self.center[1] + self.radius * libm::sin(angle),
        ]
    }

    /// Arc length.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.radius * self.sweep.abs()
    }

    /// Signed area between the chord and the arc: positive for a
    /// counter-clockwise sweep, which lies to the right of the chord.
    #[must_use]
    pub fn segment_area(&self) -> f64 {
        let angle = self.sweep.abs();
        // `angle - sin(angle)` cancels for small angles; its series converges
        // to full precision there.
        let excess = if angle < 0.1 {
            let a2 = angle * angle;
            angle * a2 / 6.0 * (1.0 - a2 / 20.0 * (1.0 - a2 / 42.0 * (1.0 - a2 / 72.0)))
        } else {
            angle - libm::sin(angle)
        };
        (0.5 * self.radius * self.radius * excess).copysign(self.sweep)
    }

    /// The smallest axis-aligned box containing the arc: its endpoints and
    /// any axis extremes it sweeps through.
    #[must_use]
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        let mut min = [self.from[0].min(self.to[0]), self.from[1].min(self.to[1])];
        let mut max = [self.from[0].max(self.to[0]), self.from[1].max(self.to[1])];
        for (axis, sign) in [(0, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)] {
            let mut direction = [0.0; 2];
            direction[axis] = sign;
            if self.spans(direction) {
                let extreme = self.center[axis] + sign * self.radius;
                min[axis] = min[axis].min(extreme);
                max[axis] = max[axis].max(extreme);
            }
        }
        [min, max]
    }

    /// Whether the ray from the center along `direction` meets the arc.
    pub(crate) fn spans(&self, direction: [f64; 2]) -> bool {
        let start = [self.from[0] - self.center[0], self.from[1] - self.center[1]];
        let mut turn = libm::atan2(cross(start, direction), dot(start, direction));
        if self.sweep > 0.0 {
            if turn < 0.0 {
                turn += TAU;
            }
            turn <= self.sweep
        } else {
            if turn > 0.0 {
                turn -= TAU;
            }
            turn >= self.sweep
        }
    }

    fn radial(&self, point: [f64; 2]) -> [f64; 2] {
        sub(point, self.center)
    }

    /// Distance from `point` to the arc.
    pub(crate) fn distance_to(&self, point: [f64; 2]) -> f64 {
        let radial = self.radial(point);
        let distance = libm::hypot(radial[0], radial[1]);
        if distance > 0.0 && self.spans(radial) {
            (distance - self.radius).abs()
        } else if distance == 0.0 {
            self.radius
        } else {
            length(sub(point, self.from)).min(length(sub(point, self.to)))
        }
    }

    /// Whether `point`, known to lie on the circle, lies on the arc.
    fn holds(&self, point: [f64; 2]) -> bool {
        self.spans(self.radial(point))
    }
}

/// One profile segment as a straight piece or an arc.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    Line([f64; 2], [f64; 2]),
    Arc(Arc),
}

impl Piece {
    /// Conservative axis-aligned support bounds including authored endpoints.
    pub(crate) fn bounds(&self) -> [[f64; 2]; 2] {
        match self {
            Self::Line(from, to) => [
                [from[0].min(to[0]), from[1].min(to[1])],
                [from[0].max(to[0]), from[1].max(to[1])],
            ],
            Self::Arc(arc) => arc.bounds(),
        }
    }
    pub(crate) fn from(&self) -> [f64; 2] {
        match self {
            Self::Line(from, _) => *from,
            Self::Arc(arc) => arc.from,
        }
    }

    pub(crate) fn to(&self) -> [f64; 2] {
        match self {
            Self::Line(_, to) => *to,
            Self::Arc(arc) => arc.to,
        }
    }

    pub(crate) fn midpoint(&self) -> [f64; 2] {
        match self {
            Self::Line(from, to) => [0.5 * (from[0] + to[0]), 0.5 * (from[1] + to[1])],
            Self::Arc(arc) => arc.point_at(0.5),
        }
    }

    pub(crate) fn distance_to(&self, point: [f64; 2]) -> f64 {
        match self {
            Self::Line(from, to) => segment_point_distance(*from, *to, point),
            Self::Arc(arc) => arc.distance_to(point),
        }
    }

    /// Smallest distance between the two pieces; zero where they meet.
    pub(crate) fn distance(&self, other: &Self) -> f64 {
        if !self.crossings(other).is_empty() {
            return 0.0;
        }
        let mut best = self
            .distance_to(other.from())
            .min(self.distance_to(other.to()))
            .min(other.distance_to(self.from()))
            .min(other.distance_to(self.to()));
        // Interior critical points: where a circle's radius is normal to a
        // line, or both circles meet the line of centers.
        match (self, other) {
            (Self::Line(..), Self::Line(..)) => {}
            (Self::Line(from, to), Self::Arc(arc)) | (Self::Arc(arc), Self::Line(from, to)) => {
                let direction = sub(*to, *from);
                let span = length(direction);
                let normal = [-direction[1] / span, direction[0] / span];
                for side in [1.0, -1.0] {
                    let radial = [side * normal[0], side * normal[1]];
                    if arc.spans(radial) {
                        let point = [
                            arc.center[0] + arc.radius * radial[0],
                            arc.center[1] + arc.radius * radial[1],
                        ];
                        best = best.min(segment_point_distance(*from, *to, point));
                    }
                }
            }
            (Self::Arc(first), Self::Arc(second)) => {
                let between = sub(second.center, first.center);
                let gap = length(between);
                if gap == 0.0 {
                    let overlap = first.spans(second.radial(second.from))
                        || first.spans(second.radial(second.to))
                        || second.spans(first.radial(first.from));
                    if overlap {
                        best = best.min((first.radius - second.radius).abs());
                    }
                } else {
                    let axis = [between[0] / gap, between[1] / gap];
                    for a in [1.0, -1.0] {
                        for b in [1.0, -1.0] {
                            let (ra, rb) = ([a * axis[0], a * axis[1]], [b * axis[0], b * axis[1]]);
                            if first.spans(ra) && second.spans(rb) {
                                let p = [
                                    first.center[0] + first.radius * ra[0],
                                    first.center[1] + first.radius * ra[1],
                                ];
                                let q = [
                                    second.center[0] + second.radius * rb[0],
                                    second.center[1] + second.radius * rb[1],
                                ];
                                best = best.min(length(sub(p, q)));
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// Points where the two pieces cross or touch, excluding overlapping
    /// collinear or concentric runs, which [`Self::distance`] reports through
    /// endpoint distances.
    pub(crate) fn crossings(&self, other: &Self) -> Few<[f64; 2]> {
        let mut points = Few::default();
        match (self, other) {
            (Self::Line(a, b), Self::Line(c, d)) => {
                let (o1, o2) = (orient(*a, *b, *c), orient(*a, *b, *d));
                let (o3, o4) = (orient(*c, *d, *a), orient(*c, *d, *b));
                // Compare signs directly: multiplying tiny nonzero
                // orientations can underflow and hide a proper crossing.
                let opposite = |a: f64, b: f64| (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0);
                if opposite(o1, o2) && opposite(o3, o4) {
                    let t = o3 / (o3 - o4);
                    points.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
                }
            }
            _ => {
                for point in self.meetings(other).iter() {
                    if self.holds(point) && other.holds(point) {
                        points.push(point);
                    }
                }
            }
        }
        points
    }
}

impl Piece {
    /// Points where the pieces' whole curves meet, the supporting line of a
    /// line piece and the circle of an arc, whether or not they lie on the
    /// pieces. Lines meet lines nowhere here: adjacent lines share only
    /// their vertex, and collinear runs are distance cases.
    pub(crate) fn meetings(&self, other: &Self) -> Few<[f64; 2]> {
        let mut points = Few::default();
        match (self, other) {
            (Self::Line(..), Self::Line(..)) => {}
            (Self::Line(from, to), Self::Arc(arc)) | (Self::Arc(arc), Self::Line(from, to)) => {
                for t in line_circle(*from, *to, arc.center, arc.radius).iter() {
                    points.push([
                        from[0] + t * (to[0] - from[0]),
                        from[1] + t * (to[1] - from[1]),
                    ]);
                }
            }
            (Self::Arc(first), Self::Arc(second)) => {
                for point in circle_circle(first, second).iter() {
                    points.push(point);
                }
            }
        }
        points
    }

    /// Whether `point`, on this piece's curve, lies within the piece.
    pub(crate) fn holds(&self, point: [f64; 2]) -> bool {
        match self {
            Self::Line(from, to) => {
                let direction = sub(*to, *from);
                let t = dot(sub(point, *from), direction) / dot(direction, direction);
                (0.0..=1.0).contains(&t)
            }
            Self::Arc(arc) => arc.holds(point),
        }
    }

    /// The unit direction of travel at the piece's end, or at its start.
    pub(crate) fn tangent(&self, at_end: bool) -> [f64; 2] {
        let direction = match self {
            Self::Line(from, to) => sub(*to, *from),
            Self::Arc(arc) => {
                let point = if at_end { arc.to } else { arc.from };
                let radial = arc.radial(point);
                // Counter-clockwise travel turns the radius a quarter left.
                let sense = arc.sweep.signum();
                [-radial[1] * sense, radial[0] * sense]
            }
        };
        let span = length(direction);
        [direction[0] / span, direction[1] / span]
    }
}

/// At most two values.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Few<T> {
    items: [T; 2],
    len: usize,
}

impl<T: Copy> Few<T> {
    fn push(&mut self, item: T) {
        self.items[self.len] = item;
        self.len += 1;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.items[..self.len].iter().copied()
    }
}

/// Parameters along `from -> to` where the line meets the circle.
fn line_circle(from: [f64; 2], to: [f64; 2], center: [f64; 2], radius: f64) -> Few<f64> {
    let direction = sub(to, from);
    let span = length(direction);
    let unit = [direction[0] / span, direction[1] / span];
    let offset = sub(center, from);
    // Foot of the perpendicular from the center, then half the chord.
    let along = dot(offset, unit);
    let across = cross(unit, offset);
    let mut list = Few::default();
    let reach = radius * radius - across * across;
    if reach < 0.0 {
        return list;
    }
    let half = libm::sqrt(reach);
    list.push((along - half) / span);
    if half > 0.0 {
        list.push((along + half) / span);
    }
    list
}

/// Points where the two arcs' circles meet.
fn circle_circle(first: &Arc, second: &Arc) -> Few<[f64; 2]> {
    let between = sub(second.center, first.center);
    let gap = length(between);
    let mut list = Few::default();
    if gap == 0.0
        || gap > first.radius + second.radius
        || gap < (first.radius - second.radius).abs()
    {
        return list;
    }
    let along =
        (first.radius * first.radius - second.radius * second.radius + gap * gap) / (2.0 * gap);
    let across = libm::sqrt((first.radius * first.radius - along * along).max(0.0));
    let axis = [between[0] / gap, between[1] / gap];
    let foot = [
        first.center[0] + along * axis[0],
        first.center[1] + along * axis[1],
    ];
    list.push([foot[0] - across * axis[1], foot[1] + across * axis[0]]);
    if across > 0.0 {
        list.push([foot[0] + across * axis[1], foot[1] - across * axis[0]]);
    }
    list
}

/// Winding contribution of the piece around `point`, in turns times `2 pi`.
pub(crate) fn winding_angle(piece: &Piece, point: [f64; 2]) -> f64 {
    let (from, to) = (sub(piece.from(), point), sub(piece.to(), point));
    // One cross product decides both the chord angle and the side of the
    // chord, so the two cannot disagree near zero.
    let (side, along) = (cross(from, to) + 0.0, dot(from, to));
    match piece {
        Piece::Line(..) => libm::atan2(side, along),
        // A point on the chord between the endpoints sees the arc turn half
        // way around it, in the sweep's direction.
        Piece::Arc(arc) if side == 0.0 && along < 0.0 => PI.copysign(arc.sweep),
        Piece::Arc(arc) => {
            // The arc and the reversed chord bound the circular segment; a
            // point inside it is wound once more in the sweep's direction.
            // Counter-clockwise arcs lie right of the chord, where `side < 0`.
            let bulge_side = if arc.sweep > 0.0 {
                side < 0.0
            } else {
                side > 0.0
            };
            let chord = libm::atan2(side, along);
            if bulge_side && length(arc.radial(point)) < arc.radius {
                chord + TAU.copysign(arc.sweep)
            } else {
                chord
            }
        }
    }
}

/// Twice the signed area of `a, b, c`: positive when counter-clockwise.
fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    cross(sub(b, a), sub(c, a))
}

fn segment_point_distance(from: [f64; 2], to: [f64; 2], point: [f64; 2]) -> f64 {
    let direction = sub(to, from);
    let offset = sub(point, from);
    let t = (dot(offset, direction) / dot(direction, direction)).clamp(0.0, 1.0);
    length(sub(offset, [t * direction[0], t * direction[1]]))
}

pub(crate) fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

pub(crate) fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub(crate) fn length(a: [f64; 2]) -> f64 {
    libm::hypot(a[0], a[1])
}

/// Broad-phase rejection only. Expand by the requested separation and a
/// roundoff guard; uncertain or overflowing bounds always reach the exact
/// piece predicate instead of being classified as separate.
pub(crate) fn bounds_may_contact(a: [[f64; 2]; 2], b: [[f64; 2]; 2], separation: f64) -> bool {
    let scale = a
        .into_iter()
        .chain(b)
        .flatten()
        .map(f64::abs)
        .fold(f64::MIN_POSITIVE, f64::max);
    let padding = separation + 64.0 * f64::EPSILON * scale;
    (0..2).all(|axis| !(a[1][axis] + padding < b[0][axis] || b[1][axis] + padding < a[0][axis]))
}

/// Normalizes a winding angle sum to whole turns.
pub(crate) fn turns(angle: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "winding sums of validated loops are small multiples of tau"
    )]
    {
        libm::round(angle / TAU) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic sequence in `[0, 1)`.
    struct Sequence(u64);

    impl Sequence {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[expect(clippy::cast_precision_loss, reason = "53 high bits")]
            {
                (self.0 >> 11) as f64 / (1_u64 << 53) as f64
            }
        }

        fn point(&mut self) -> [f64; 2] {
            [4.0 * self.next() - 2.0, 4.0 * self.next() - 2.0]
        }

        fn piece(&mut self) -> Piece {
            let (from, to) = (self.point(), self.point());
            if self.next() < 0.3 {
                Piece::Line(from, to)
            } else {
                let magnitude = 0.05 + 3.0 * self.next();
                let bulge = if self.next() < 0.5 {
                    magnitude
                } else {
                    -magnitude
                };
                Piece::Arc(Arc::from_bulge(from, to, bulge).unwrap())
            }
        }
    }

    fn samples(piece: &Piece, count: u32) -> (alloc::vec::Vec<[f64; 2]>, f64) {
        let points: alloc::vec::Vec<_> = (0..=count)
            .map(|i| {
                let t = f64::from(i) / f64::from(count);
                match piece {
                    Piece::Line(from, to) => [
                        from[0] + t * (to[0] - from[0]),
                        from[1] + t * (to[1] - from[1]),
                    ],
                    Piece::Arc(arc) => arc.point_at(t),
                }
            })
            .collect();
        let spacing = points
            .windows(2)
            .map(|pair| length(sub(pair[1], pair[0])))
            .fold(0.0, f64::max);
        (points, spacing)
    }

    #[test]
    fn piece_distances_agree_with_dense_sampling() {
        let mut sequence = Sequence(7);
        for _ in 0..300 {
            let (first, second) = (sequence.piece(), sequence.piece());
            let (a, spacing_a) = samples(&first, 400);
            let (b, spacing_b) = samples(&second, 400);
            let sampled = a
                .iter()
                .flat_map(|p| b.iter().map(move |q| length(sub(*p, *q))))
                .fold(f64::INFINITY, f64::min);
            let exact = first.distance(&second);
            for separation in [1e-9, 0.001, 0.1, 0.5, 2.0] {
                if !bounds_may_contact(first.bounds(), second.bounds(), separation) {
                    assert!(
                        exact >= separation,
                        "bounds rejected a contact: {first:?} {second:?}"
                    );
                }
            }
            assert!(
                exact <= sampled + 1e-12,
                "{first:?} {second:?}: {exact} > {sampled}"
            );
            assert!(
                exact >= sampled - 0.5 * (spacing_a + spacing_b) - 1e-12,
                "{first:?} {second:?}: {exact} << {sampled}"
            );
            assert_eq!(
                first.distance(&second).to_bits(),
                second.distance(&first).to_bits()
            );
        }
    }

    #[test]
    fn winding_counts_turns_around_points_on_and_off_chords() {
        // A counter-clockwise unit circle of two half arcs.
        let upper = Piece::Arc(Arc::from_bulge([1.0, 0.0], [-1.0, 0.0], 1.0).unwrap());
        let lower = Piece::Arc(Arc::from_bulge([-1.0, 0.0], [1.0, 0.0], 1.0).unwrap());
        let wound = |point| turns(winding_angle(&upper, point) + winding_angle(&lower, point));
        for point in [[0.0, 0.0], [0.5, 0.0], [0.0, 0.9], [-0.3, -0.6]] {
            assert_eq!(wound(point), 1, "{point:?}");
        }
        for point in [[2.0, 0.0], [0.0, 1.1], [-1.5, 0.0], [0.9, 0.9]] {
            assert_eq!(wound(point), 0, "{point:?}");
        }
    }
}
