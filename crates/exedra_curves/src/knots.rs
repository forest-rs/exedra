// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact edits of a NURBS curve: knot insertion, splitting, clamping,
//! reversal, reparameterization and degree elevation.
//!
//! Each gives a new curve that traces the same points as the old, to
//! rounding: the new controls are convex combinations of the old,
//! computed on [weighted controls](WeightedPoint). A curve
//! without weights stays without weights, its controls combined directly.

use alloc::vec::Vec;

use crate::NurbsCurve;
use crate::vector::WeightedPoint;

impl<const N: usize> NurbsCurve<N> {
    /// The curve with the knot `t` inserted until it occurs `multiplicity`
    /// times, or as often as it already does if that is more; the same
    /// curve with more controls (Boehm's algorithm; Piegl and Tiller, *The
    /// NURBS Book*, 2nd ed., algorithm A5.1). Multiplicity is capped at
    /// the degree, where the curve is still continuous.
    ///
    /// `None` when `t` is not inside the domain, ends excluded.
    #[must_use]
    pub fn insert_knot(&self, t: f64, multiplicity: u32) -> Option<Self> {
        let [start, end] = self.domain();
        if !(start < t && t < end) {
            return None;
        }
        let degree = self.order - 1;
        Some(self.inserted(t, multiplicity.min(degree)))
    }

    /// The curve cut at `t` into the pieces before and after it, each
    /// clamped at the cut: the knot is inserted to the degree's
    /// multiplicity, where the curve passes through a control, which both
    /// pieces share. A curve that jumps at `t` keeps each side's own end.
    ///
    /// `None` when `t` is not inside the domain, ends excluded.
    #[must_use]
    pub fn split(&self, t: f64) -> Option<(Self, Self)> {
        let [start, end] = self.domain();
        if !(start < t && t < end) {
            return None;
        }
        let full = self.inserted(t, self.order - 1);
        Some((full.before(t), full.after(t)))
    }

    /// The piece of the curve over `range`, clamped at both ends, or `None`
    /// unless `range` is an interval inside the domain.
    #[must_use]
    pub fn segment(&self, range: [f64; 2]) -> Option<Self> {
        let [start, end] = self.domain();
        let [a, b] = range;
        if !(start <= a && a < b && b <= end) {
            return None;
        }
        let mut curve = self.clamped();
        if a > start {
            curve = curve.split(a)?.1;
        }
        if b < end {
            curve = curve.split(b)?.0;
        }
        Some(curve)
    }

    /// The curve with its knot vector clamped: the domain's ends repeated
    /// `order` times, so the curve starts at its first control and ends at
    /// its last. Controls that only shaped the curve outside the domain
    /// are dropped. Already clamped, the curve is returned unchanged.
    #[must_use]
    pub fn clamped(&self) -> Self {
        let [start, end] = self.domain();
        let degree = self.order - 1;
        let mut curve = self.inserted(start, degree).inserted(end, degree);
        if curve.multiplicity(start) < self.order as usize {
            curve = curve.after(start);
        }
        if curve.multiplicity(end) < self.order as usize {
            curve = curve.before(end);
        }
        curve
    }

    /// The curve traced backwards over the same domain: the point at `t`
    /// is the old curve's at `start + end - t`. A curve whose knots outside
    /// the domain would reflect past what an `f64` holds is clamped first,
    /// which drops them.
    #[must_use]
    pub fn reversed(&self) -> Self {
        let [start, end] = self.domain();
        if self.knots.iter().any(|&u| !(start + (end - u)).is_finite()) {
            return self.clamped().reversed();
        }
        let map = |u: f64| {
            if u == start {
                end
            } else if u == end {
                start
            } else if start < u && u < end {
                (start + (end - u)).clamp(start, end)
            } else {
                start + (end - u)
            }
        };
        let knots = self.knots.iter().rev().map(|&u| map(u)).collect();
        let points = self.points.iter().rev().copied().collect();
        let weights = self
            .weights
            .as_ref()
            .map(|w| w.iter().rev().copied().collect());
        Self {
            order: self.order,
            knots,
            points,
            weights,
        }
    }

    /// The curve over a new domain `[a, b]`, its knots mapped affinely:
    /// the point at `a + (b - a) s` is the old curve's at
    /// `start + (end - start) s`. `None` unless `a < b`, both finite, every
    /// mapped knot finite, and distinct knots still distinct, which a
    /// domain too narrow for their spacing would merge.
    #[must_use]
    pub fn with_domain(&self, domain: [f64; 2]) -> Option<Self> {
        let [a, b] = domain;
        if !(a.is_finite() && b.is_finite() && a < b) {
            return None;
        }
        let [start, end] = self.domain();
        let (from, to) = (end - start, b - a);
        // Through the knot's place in the old domain, so neither a ratio
        // of the widths nor a knot's offset underflows or overflows.
        let map = |u: f64| {
            if u == start {
                a
            } else if u == end {
                b
            } else {
                a + (u - start) / from * to
            }
        };
        let mut knots: Vec<f64> = self.knots.iter().map(|&u| map(u)).collect();
        // Rounding cannot reorder knots mapped by one increasing affine
        // map, but can push an interior knot past a domain end.
        let degree = self.order as usize - 1;
        let n = self.points.len();
        for u in &mut knots[degree + 1..n] {
            *u = u.clamp(a, b);
        }
        let merged = self
            .knots
            .windows(2)
            .zip(knots.windows(2))
            .any(|(old, new)| old[0] < old[1] && new[0] >= new[1]);
        if merged {
            return None;
        }
        let curve = Self {
            order: self.order,
            knots,
            points: self.points.clone(),
            weights: self.weights.clone(),
        };
        curve.view().curve().validate().ok()?;
        Some(curve)
    }

    /// The curve raised `by` degrees: the same curve, as one of higher
    /// order whose knots each occur `by` more times, so it is exactly as
    /// smooth at each. Curves of different degrees made one degree can
    /// share a knot vector, as a surface through them needs.
    ///
    /// Each new control is the curve's blossom at `degree + by`
    /// consecutive knots, which for the raised curve is the average of the
    /// old blossom over every choice of `degree` of them (Ramshaw,
    /// "Blossoms are polar forms", 1989; Piegl and Tiller, *The NURBS
    /// Book*, 2nd ed., section 5.5). The cost grows with that binomial
    /// count, modest at the degrees curves take.
    #[must_use]
    pub fn elevated(&self, by: u32) -> Self {
        if by == 0 {
            return self.clone();
        }
        let curve = self.clamped();
        let p = curve.order as usize - 1;
        let q = p + by as usize;
        let mut knots = Vec::with_capacity(curve.knots.len() * 2);
        for (i, &u) in curve.knots.iter().enumerate() {
            knots.push(u);
            if curve.knots.get(i + 1) != Some(&u) {
                knots.extend(core::iter::repeat_n(u, by as usize));
            }
        }
        let n = knots.len() - q - 1;
        let view = curve.view();
        let choices = choices(q, p);
        let mut scratch = alloc::vec![WeightedPoint::ZERO; p + 1];
        let mut args = alloc::vec![0.0; p];
        let count = choices.len() as f64;
        let homogeneous: Vec<WeightedPoint<N>> = (0..n)
            .map(|i| {
                // The widest span the control shapes, inside the domain;
                // the clamped ends, and no knot repeated past the order,
                // make one nonempty. Every de Boor ratio's denominator
                // spans it, and its argument lies in the control's support
                // of at most `q + 1` spans, so no ratio exceeds `q + 2` in
                // size: no extrapolation from a narrow span amplifies
                // rounding.
                let k = (i.max(q)..=(i + q).min(n - 1))
                    .max_by(|&j, &k| {
                        (knots[j + 1] - knots[j])
                            .total_cmp(&(knots[k + 1] - knots[k]))
                            .then(k.cmp(&j))
                    })
                    .filter(|&k| knots[k] < knots[k + 1])
                    .expect("a control shapes a nonempty span");
                let old = curve.knots.partition_point(|&u| u <= knots[k]) - 1;
                let window = &knots[i + 1..=i + q];
                // Each term divided before summing, so the average of
                // controls near the largest `f64` does not overflow.
                let mut sum = WeightedPoint::ZERO;
                for choice in &choices {
                    for (slot, &j) in args.iter_mut().zip(choice) {
                        *slot = window[j];
                    }
                    view.load(old, &mut scratch);
                    let h = view.de_boor_levels(old, &mut scratch, |r| args[r - 1]);
                    sum = WeightedPoint::plus(sum, h.divided(count));
                }
                sum
            })
            .collect();
        Self::from_homogeneous(self.order + by, knots, &homogeneous, self.weights.is_some())
    }

    /// This curve and `other` as one pair of the same order over the same
    /// knot vector, each the same curve as before: the lower degree is
    /// raised to the higher, and every knot is inserted into each curve as
    /// often as the other has it. Where one curve jumps and the other is
    /// continuous, the continuous one repeats the control it passes
    /// through, so both break there. Surfaces through several curves need
    /// them so. `None` unless both have the same domain.
    #[must_use]
    pub fn compatible(&self, other: &Self) -> Option<(Self, Self)> {
        if self.domain() != other.domain() {
            return None;
        }
        let order = self.order.max(other.order);
        let mut a = self.clamped().elevated(order - self.order);
        let mut b = other.clamped().elevated(order - other.order);
        let [start, end] = a.domain();
        // Every interior knot, with the larger multiplicity of the two.
        let mut knots: Vec<f64> = a
            .knots
            .iter()
            .chain(&b.knots)
            .copied()
            .filter(|&u| start < u && u < end)
            .collect();
        knots.sort_by(f64::total_cmp);
        knots.dedup();
        for u in knots {
            let want = a.multiplicity(u).max(b.multiplicity(u));
            for curve in [&mut a, &mut b] {
                if curve.multiplicity(u) < want {
                    let at = u32::try_from(want).unwrap_or(u32::MAX).min(order - 1);
                    *curve = curve.inserted(u, at);
                    // A knot repeats `order` times only where the other
                    // curve jumps; this one passes through a control there.
                    if curve.multiplicity(u) < want {
                        *curve = curve.broken(u);
                    }
                }
            }
        }
        (a.knots == b.knots).then_some((a, b))
    }

    /// The curve with `t`, an interior knot occurring `degree` times,
    /// occurring `order` times: the control the curve passes through at
    /// `t` is repeated, ending the piece before and starting the piece
    /// after, so the curve breaks there without moving.
    fn broken(&self, t: f64) -> Self {
        let first = self.knots.partition_point(|&u| u < t);
        let mut knots = self.knots.clone();
        knots.insert(first, t);
        let mut points = self.points.clone();
        points.insert(first - 1, points[first - 1]);
        let weights = self.weights.as_ref().map(|weights| {
            let mut weights = weights.clone();
            weights.insert(first - 1, weights[first - 1]);
            weights
        });
        Self {
            order: self.order,
            knots,
            points,
            weights,
        }
    }

    /// How many times `t` occurs in the knot vector.
    fn multiplicity(&self, t: f64) -> usize {
        self.knots.iter().filter(|&&u| u == t).count()
    }

    /// The weighted controls; weight 1 without weights.
    fn homogeneous(&self) -> Vec<WeightedPoint<N>> {
        self.points
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                WeightedPoint::new(p, self.weights.as_ref().map_or(1.0, |weights| weights[i]))
            })
            .collect()
    }

    /// A curve from weighted controls computed by convex combination from
    /// a valid curve's. Without weights the weight, which rounding may have
    /// moved off 1, is dropped and the coordinates taken as they are.
    fn from_homogeneous(
        order: u32,
        knots: Vec<f64>,
        homogeneous: &[WeightedPoint<N>],
        rational: bool,
    ) -> Self {
        let points = homogeneous
            .iter()
            .map(|h| if rational { h.point() } else { h.weighted })
            .collect();
        let weights = rational.then(|| homogeneous.iter().map(|h| h.weight).collect());
        let curve = Self {
            order,
            knots,
            points,
            weights,
        };
        debug_assert!(
            curve.view().curve().validate().is_ok(),
            "edits of a valid curve stay valid"
        );
        curve
    }

    /// The curve with `t`, a parameter in the domain, inserted until it
    /// occurs `multiplicity` times; `multiplicity` is at most the degree.
    fn inserted(&self, t: f64, multiplicity: u32) -> Self {
        let p = self.order as usize - 1;
        let have = self.multiplicity(t);
        let r = (multiplicity as usize).saturating_sub(have);
        if r == 0 {
            return self.clone();
        }
        // The span `[knots[k], knots[k + 1])` holding `t`. With fewer than
        // `degree` copies of `t`, some knot exceeds it: at the domain's
        // end, the clamped end repeats `order` times.
        let k = self.knots.partition_point(|&u| u <= t) - 1;
        let (knots, homogeneous) = insert(p, &self.knots, &self.homogeneous(), t, k, have, r);
        Self::from_homogeneous(self.order, knots, &homogeneous, self.weights.is_some())
    }

    /// The piece before `t`, a knot of multiplicity at least the degree.
    fn before(&self, t: f64) -> Self {
        let p = self.order as usize - 1;
        let first = self.knots.partition_point(|&u| u < t);
        let mut knots = self.knots[..first + p].to_vec();
        knots.push(t);
        self.piece(knots, 0..first)
    }

    /// The piece after `t`, a knot of multiplicity at least the degree.
    fn after(&self, t: f64) -> Self {
        let p = self.order as usize - 1;
        let last = self.knots.partition_point(|&u| u <= t) - 1;
        let mut knots = alloc::vec![t];
        knots.extend_from_slice(&self.knots[last + 1 - p..]);
        self.piece(knots, last - p..self.points.len())
    }

    fn piece(&self, knots: Vec<f64>, controls: core::ops::Range<usize>) -> Self {
        let curve = Self {
            order: self.order,
            knots,
            points: self.points[controls.clone()].to_vec(),
            weights: self.weights.as_ref().map(|w| w[controls].to_vec()),
        };
        debug_assert!(
            curve.view().curve().validate().is_ok(),
            "edits of a valid curve stay valid"
        );
        curve
    }
}

/// Inserts `t`, in span `k` and already of multiplicity `s`, `r` more
/// times into a curve of degree `p` (Piegl and Tiller, algorithm A5.1).
/// `r + s` is at most `p`.
fn insert<const N: usize>(
    p: usize,
    knots: &[f64],
    controls: &[WeightedPoint<N>],
    t: f64,
    k: usize,
    s: usize,
    r: usize,
) -> (Vec<f64>, Vec<WeightedPoint<N>>) {
    let n = controls.len();
    let mut new_knots = Vec::with_capacity(knots.len() + r);
    new_knots.extend_from_slice(&knots[..=k]);
    new_knots.extend(core::iter::repeat_n(t, r));
    new_knots.extend_from_slice(&knots[k + 1..]);
    let mut q = alloc::vec![WeightedPoint::ZERO; n + r];
    q[..=k - p].copy_from_slice(&controls[..=k - p]);
    q[k - s + r..].copy_from_slice(&controls[k - s..]);
    let mut work: Vec<WeightedPoint<N>> = controls[k - p..=k - s].to_vec();
    let mut l = k - p;
    for j in 1..=r {
        l = k - p + j;
        for i in 0..=p - j - s {
            let alpha = (t - knots[l + i]) / (knots[i + k + 1] - knots[l + i]);
            work[i] = WeightedPoint::lerp(work[i], work[i + 1], alpha);
        }
        q[l] = work[0];
        q[k + r - j - s] = work[p - j - s];
    }
    if l + 1 < k - s {
        q[l + 1..k - s].copy_from_slice(&work[1..k - s - l]);
    }
    (new_knots, q)
}

/// Every choice of `k` of the indices `0..n`, increasing.
fn choices(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut choice: Vec<usize> = (0..k).collect();
    loop {
        out.push(choice.clone());
        // Advance the last index that can move, and reset those after it.
        let Some(i) = (0..k).rev().find(|&i| choice[i] < n - k + i) else {
            return out;
        };
        choice[i] += 1;
        for j in i + 1..k {
            choice[j] = choice[j - 1] + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use crate::NurbsCurve;

    /// A weighted cubic with uneven interior knots.
    fn weighted() -> NurbsCurve {
        NurbsCurve::new(
            4,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.5, 2.0, 2.0, 2.0, 2.0],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 2.0, 0.5],
                [3.0, 2.5, -1.0],
                [4.0, -1.0, 0.0],
                [6.0, 0.0, 2.0],
                [7.0, 1.0, 1.0],
            ],
            Some(vec![1.0, 0.5, 2.0, 1.5, 0.75, 1.0]),
        )
        .unwrap()
    }

    /// An unclamped uniform cubic B-spline over `[3, 6]`.
    fn unclamped() -> NurbsCurve {
        NurbsCurve::new(
            4,
            (0..10).map(f64::from).collect(),
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 3.0, 0.0],
                [2.0, -1.0, 1.0],
                [4.0, 2.0, 0.0],
                [5.0, 0.0, -2.0],
                [6.0, 1.0, 0.0],
            ],
            None,
        )
        .unwrap()
    }

    /// A quadratic with a double interior knot, a corner.
    fn cornered() -> NurbsCurve {
        NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 2.0, 1.0],
                [4.0, 0.0, 0.0],
                [5.0, 1.0, 0.0],
            ],
            Some(vec![1.0, 2.0, 1.0, 0.5, 1.0, 1.0]),
        )
        .unwrap()
    }

    /// A quintic, past the inline scratch when raised.
    fn quintic() -> NurbsCurve {
        let points: Vec<[f64; 3]> = (0..9)
            .map(|i| {
                let i = f64::from(i);
                [i, (i * 1.3).sin() * 2.0, (i * 0.7).cos()]
            })
            .collect();
        let knots = vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
        ];
        NurbsCurve::new(6, knots, points, None).unwrap()
    }

    fn curves() -> [NurbsCurve; 4] {
        [weighted(), unclamped(), cornered(), quintic()]
    }

    /// Parameters across the domain, both of its ends included.
    fn params(curve: &NurbsCurve) -> impl Iterator<Item = f64> {
        let [a, b] = curve.domain();
        (0..=97).map(move |i| a + (b - a) * f64::from(i) / 97.0)
    }

    fn assert_near(a: [f64; 3], b: [f64; 3], what: &str) {
        let d = exedra_math::norm(exedra_math::sub(a, b));
        assert!(d < 1e-11, "{what}: {a:?} against {b:?}");
    }

    /// Whether `a`'s point at each parameter is `b`'s at `map` of it.
    fn assert_traces(a: &NurbsCurve, b: &NurbsCurve, map: impl Fn(f64) -> f64, what: &str) {
        for t in params(a) {
            assert_near(a.view().evaluate(t), b.view().evaluate(map(t)), what);
        }
    }

    fn assert_clamped(curve: &NurbsCurve) {
        let order = curve.order() as usize;
        let knots = curve.knots();
        assert!(knots[..order].iter().all(|&u| u == knots[0]), "{knots:?}");
        assert!(
            knots[knots.len() - order..]
                .iter()
                .all(|&u| u == knots[knots.len() - 1])
        );
        let [a, b] = curve.domain();
        assert_near(curve.view().evaluate(a), weighted_point(curve, 0), "start");
        let last = curve.points().len() - 1;
        assert_near(curve.view().evaluate(b), weighted_point(curve, last), "end");
    }

    fn weighted_point(curve: &NurbsCurve, i: usize) -> [f64; 3] {
        curve.points()[i]
    }

    #[test]
    fn inserted_knots_keep_the_curve() {
        for curve in curves() {
            let [a, b] = curve.domain();
            for (t, times) in [
                (a + 0.31 * (b - a), 1),
                (a + 0.5 * (b - a), 3),
                (curve.knots()[curve.order() as usize], 2),
            ] {
                let Some(more) = curve.insert_knot(t, times) else {
                    continue;
                };
                assert_traces(&more, &curve, |t| t, "insertion");
                assert!(more.points().len() >= curve.points().len());
                assert_eq!(
                    more.knots().len() - more.points().len(),
                    curve.order() as usize
                );
            }
            assert!(curve.insert_knot(a, 1).is_none() && curve.insert_knot(f64::NAN, 1).is_none());
        }
        // Inserting a knot the curve has keeps it, raised to the degree.
        let corner = cornered().insert_knot(1.0, 5).unwrap();
        assert_eq!(corner.knots().iter().filter(|&&u| u == 1.0).count(), 2);
    }

    #[test]
    fn split_pieces_meet_and_trace_their_halves() {
        for curve in curves() {
            let [a, b] = curve.domain();
            for t in [a + 0.2 * (b - a), a + 0.5 * (b - a), a + 0.9 * (b - a)] {
                let (left, right) = curve.split(t).unwrap();
                assert_eq!(left.domain(), [a, t]);
                assert_eq!(right.domain(), [t, b]);
                assert_traces(&left, &curve, |t| t, "left");
                assert_traces(&right, &curve, |t| t, "right");
                assert_eq!(left.points().last(), right.points().first());
                // Clamped at the cut.
                let order = curve.order() as usize;
                assert!(
                    left.knots()[left.knots().len() - order..]
                        .iter()
                        .all(|&u| u == t)
                );
                assert!(right.knots()[..order].iter().all(|&u| u == t));
            }
        }
        // At the corner the pieces share the corner.
        let (left, right) = cornered().split(2.0).unwrap();
        assert_near(*left.points().last().unwrap(), [3.0, 2.0, 1.0], "corner");
        assert_eq!(left.points().last(), right.points().first());
    }

    #[test]
    fn segments_and_clamping_keep_the_curve() {
        for curve in curves() {
            let [a, b] = curve.domain();
            let clamped = curve.clamped();
            assert_eq!(clamped.domain(), [a, b]);
            assert_traces(&clamped, &curve, |t| t, "clamped");
            assert_clamped(&clamped);
            let range = [a + 0.15 * (b - a), a + 0.7 * (b - a)];
            let piece = curve.segment(range).unwrap();
            assert_eq!(piece.domain(), range);
            assert_traces(&piece, &curve, |t| t, "segment");
            assert_eq!(curve.segment([a, b]).unwrap(), clamped);
            assert!(curve.segment([range[1], range[0]]).is_none());
            assert!(curve.segment([a - 1.0, b]).is_none());
        }
        // A clamped curve is its own clamping.
        assert_eq!(weighted().clamped(), weighted());
    }

    #[test]
    fn reversal_and_reparameterization_keep_the_points() {
        for curve in curves() {
            let [a, b] = curve.domain();
            let back = curve.reversed();
            assert_eq!(back.domain(), [a, b]);
            assert_traces(&back, &curve, |t| a + (b - t), "reversed");
            let there = back.reversed();
            assert_traces(&there, &curve, |t| t, "reversed twice");
            let moved = curve.with_domain([-2.0, 5.0]).unwrap();
            assert_eq!(moved.domain(), [-2.0, 5.0]);
            assert_traces(&moved, &curve, |t| a + (t + 2.0) / 7.0 * (b - a), "moved");
            assert!(curve.with_domain([1.0, 1.0]).is_none());
            assert!(curve.with_domain([0.0, f64::INFINITY]).is_none());
        }
    }

    #[test]
    fn elevated_curves_trace_the_same_points() {
        for curve in curves() {
            for by in 1..=3 {
                let raised = curve.elevated(by);
                assert_eq!(raised.order(), curve.order() + by);
                assert_eq!(raised.domain(), curve.domain());
                assert_traces(&raised, &curve, |t| t, "elevated");
                // Each distinct knot gains `by` copies, and so does each end.
                let clamped = curve.clamped();
                let distinct = |k: &[f64]| k.windows(2).filter(|w| w[0] < w[1]).count() + 1;
                assert_eq!(distinct(raised.knots()), distinct(clamped.knots()));
                assert_eq!(
                    raised.knots().len(),
                    clamped.knots().len() + by as usize * distinct(clamped.knots())
                );
            }
        }
        assert_eq!(weighted().elevated(0), weighted());
        // Without weights the curve stays polynomial.
        assert!(unclamped().elevated(2).weights().is_none());
    }

    #[test]
    fn a_jump_is_kept_through_every_edit() {
        // A quadratic whose middle knot repeats three times: two pieces.
        let jump = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0],
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 3.0, 0.0],
                [3.0, 4.0, 0.0],
                [4.0, 3.0, 0.0],
            ],
            None,
        )
        .unwrap();
        let (left, right) = jump.split(1.0).unwrap();
        assert_near(left.view().evaluate(1.0), [2.0, 0.0, 0.0], "left end");
        assert_near(right.view().evaluate(1.0), [2.0, 3.0, 0.0], "right start");
        let raised = jump.elevated(1);
        assert_traces(&raised, &jump, |t| t, "elevated jump");
        let back = jump.reversed();
        assert_traces(&back, &jump, |t| 2.0 - t, "reversed jump");
    }

    #[test]
    fn compatible_pairs_share_order_and_knots() {
        let pairs = [
            (weighted(), weighted().reversed()),
            (unclamped(), unclamped().elevated(1)),
            (
                cornered().with_domain([0.0, 2.0]).unwrap(),
                weighted().clone(),
            ),
            (
                NurbsCurve::new(
                    2,
                    vec![0.0, 0.0, 2.0, 2.0],
                    vec![[0.0; 3], [1.0, 1.0, 1.0]],
                    None,
                )
                .unwrap(),
                weighted(),
            ),
        ];
        for (a, b) in pairs {
            let (x, y) = a.compatible(&b).unwrap();
            assert_eq!(x.order(), y.order());
            assert_eq!(x.knots(), y.knots());
            assert_traces(&x, &a, |t| t, "first");
            assert_traces(&y, &b, |t| t, "second");
        }
        assert!(weighted().compatible(&quintic()).is_none());
    }

    #[test]
    fn a_continuous_curve_breaks_to_match_a_jump() {
        let line = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 2.0, 2.0],
            vec![[0.0; 3], [2.0, 0.0, 0.0]],
            None,
        )
        .unwrap();
        let jump = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0],
            vec![
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [11.0, 0.0, 0.0],
            ],
            None,
        )
        .unwrap();
        for (a, b) in [(&line, &jump), (&jump, &line)] {
            let (x, y) = a.compatible(b).unwrap();
            assert_eq!(x.knots(), y.knots());
            assert_traces(&x, a, |t| t, "first");
            assert_traces(&y, b, |t| t, "second");
        }
    }

    #[test]
    fn elevation_stays_exact_beside_a_tiny_span_and_near_the_largest_f64() {
        let narrow = NurbsCurve::new(
            3,
            vec![0.0, 0.0, 0.0, 1e-20, 1.0, 1.0, 1.0],
            vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]],
            None,
        )
        .unwrap();
        assert_traces(&narrow.elevated(1), &narrow, |t| t, "beside a tiny span");
        assert_traces(&narrow.elevated(3), &narrow, |t| t, "raised three");
        let far = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![[1e308, 0.0, 0.0]; 2],
            None,
        )
        .unwrap();
        let raised = far.elevated(2);
        assert!(raised.points().iter().all(|p| p[0] == 1e308), "{raised:?}");
    }

    #[test]
    fn knots_repeated_past_the_order_are_refused() {
        // The middle control would shape no span.
        let points = vec![
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [99.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [11.0, 0.0, 0.0],
        ];
        assert_eq!(
            NurbsCurve::new(2, vec![0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0], points, None).err(),
            Some(crate::NurbsError::Knots { index: 4 })
        );
    }

    #[test]
    fn new_domains_neither_underflow_nor_merge_knots() {
        let wide = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 5e299, 1e300, 1e300],
            vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0; 3]],
            None,
        )
        .unwrap();
        let narrow = wide.with_domain([0.0, 1e-300]).unwrap();
        assert_traces(&narrow, &wide, |t| t / 1e-300 * 1e300, "narrowed");
        let tiny = f64::from_bits(1);
        assert!(weighted().with_domain([0.0, tiny]).is_none());
    }

    #[test]
    fn reversal_drops_knots_that_would_reflect_past_the_largest_f64() {
        let curve = NurbsCurve::new(
            2,
            vec![0.0, 1e308, 1.1e308, 1.2e308],
            vec![[0.0; 3], [1.0, 2.0, 3.0]],
            None,
        )
        .unwrap();
        let back = curve.reversed();
        assert!(back.knots().iter().all(|u| u.is_finite()));
        assert!(back.view().curve().validate().is_ok());
        let [a, b] = curve.domain();
        assert_eq!(back.domain(), [a, b]);
        assert_traces(&back, &curve, |t| a + (b - t), "reversed");
    }
}
