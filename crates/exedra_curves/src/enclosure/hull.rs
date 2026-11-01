// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Control-hull proofs which retain subdivision roundoff and shared endpoints.

use super::EnclosureError;
use crate::{intersect::MAX_ORDER, interval::I};
use alloc::{sync::Arc, vec::Vec};
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};

#[derive(Clone, Debug)]
struct Control {
    homogeneous: [I; 3],
    exact: Option<[f64; 2]>,
    /// The point's identity and its one representative, kept when the
    /// control is only reweighted, so pieces sharing it agree on it.
    point: Arc<[f64; 2]>,
}
impl Control {
    /// A control with a fresh identity, represented by its exact point or
    /// the middle of its enclosure; `None` where that cannot be formed.
    fn fresh(homogeneous: [I; 3], exact: Option<[f64; 2]>) -> Option<Self> {
        let mut control = Self {
            homogeneous,
            exact,
            point: Arc::new([0.0; 2]),
        };
        let representative = match exact {
            Some(p) => p,
            None => control.bounds()?.map(I::mid),
        };
        control.point = Arc::new(representative);
        Some(control)
    }

    fn bounds(&self) -> Option<[I; 2]> {
        if let Some(p) = self.exact {
            return Some(p.map(I::p));
        }
        let [x, y, w] = self.homogeneous;
        if w.lo <= 0.0 {
            return None;
        }
        let p = [x / w, y / w];
        p.iter()
            .all(|v| v.valid() && v.lo.abs().max(v.hi.abs()) <= MAX_COORDINATE)
            .then_some(p)
    }
}
fn same(a: &Arc<Control>, b: &Arc<Control>) -> bool {
    Arc::ptr_eq(&a.point, &b.point) || a.exact.zip(b.exact).is_some_and(|(a, b)| a == b)
}

/// A proved relation between two complete rational Bezier control hulls.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HullSeparation {
    /// The hulls are disjoint, hence so are their curves.
    Disjoint,
    /// The hulls intersect only at one identical endpoint. The caller must
    /// independently establish that its topology permits this adjacency.
    SharedEndpoint,
}

/// Outward-rounded rational Bezier controls for bounded outline checks.
///
/// This is proof scratch, not a replacement defining curve. Bisection propagates
/// homogeneous interval controls and shares the new endpoint's identity between
/// children. Separation therefore accounts for rounding without ignoring any
/// neighborhood of a shared endpoint. It can refuse geometrically separated
/// curves whose control hulls or interval uncertainty cannot be separated.
#[derive(Clone, Debug)]
pub struct BezierEnclosure {
    controls: Vec<Arc<Control>>,
}
impl BezierEnclosure {
    /// Encloses the exact polynomial or positive-weight rational Bezier curve
    /// defined by these finite f64 controls. Order is in `2..=MAX_ORDER`.
    ///
    /// # Errors
    /// Returns [`EnclosureError::InvalidInput`] for too few or too many
    /// controls, mismatched or non-positive weights, or controls outside the
    /// predicates' envelope, and [`EnclosureError::NumericLimit`] where
    /// rounding loses a positive weight or a finite control.
    pub fn new(points: &[[f64; 2]], weights: Option<&[f64]>) -> Result<Self, EnclosureError> {
        if !(2..=MAX_ORDER).contains(&points.len())
            || weights.is_some_and(|w| w.len() != points.len())
        {
            return Err(EnclosureError::InvalidInput);
        }
        let controls = points
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let w = weights.map_or(1.0, |weights| weights[i]);
                if !w.is_finite()
                    || w <= 0.0
                    || p.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE)
                {
                    return Err(EnclosureError::InvalidInput);
                }
                let multiply = |x| {
                    if x == 0.0 {
                        I::ZERO
                    } else if w == 1.0 {
                        I::p(x)
                    } else {
                        I::p(x) * I::p(w)
                    }
                };
                let homogeneous = [multiply(p[0]), multiply(p[1]), I::p(w)];
                if homogeneous.iter().any(|v| !v.valid()) {
                    return Err(EnclosureError::NumericLimit);
                }
                Control::fresh(homogeneous, Some(p))
                    .map(Arc::new)
                    .ok_or(EnclosureError::NumericLimit)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { controls }.balanced())
    }

    /// The same enclosure reweighted towards standard form, its end weights
    /// near one another, where they lie more than a factor of four apart:
    /// control `i` scaled by `c^i` for one positive `c`, which moves no
    /// control's point, so no hull, while making halving split the curve
    /// where it travels rather than where a heavy weight rushes it.
    fn balanced(self) -> Self {
        let n = self.controls.len() - 1;
        let (first, last) = (
            self.controls[0].homogeneous[2],
            self.controls[n].homogeneous[2],
        );
        if first.mid() <= 4.0 * last.mid() && last.mid() <= 4.0 * first.mid() {
            return self;
        }
        let c = libm::pow(first.mid() / last.mid(), 1.0 / n as f64);
        if !(c.is_finite() && c > 0.0) {
            return self;
        }
        // All or nothing: a control left at its old weight would describe
        // another curve.
        let mut power = I::ONE;
        let mut controls = Vec::with_capacity(self.controls.len());
        for control in &self.controls {
            let scaled = control.homogeneous.map(|v| v * power);
            power = power * I::p(c);
            let reweighted = Control {
                homogeneous: scaled,
                exact: control.exact,
                point: control.point.clone(),
            };
            if !scaled.iter().all(|v| v.valid())
                || scaled[2].lo <= 0.0
                || reweighted.bounds().is_none()
            {
                return self;
            }
            controls.push(Arc::new(reweighted));
        }
        Self { controls }
    }
    /// Bisects at parameter `1/2`, including all homogeneous arithmetic roundoff.
    /// The returned curves retain the same parameter direction.
    ///
    /// # Errors
    /// Returns [`EnclosureError::NumericLimit`] if finite positive enclosures cannot
    /// be maintained. The input remains available unchanged.
    pub fn bisect(&self) -> Result<(Self, Self), EnclosureError> {
        let n = self.controls.len();
        let mut row = self.controls.clone();
        let mut left = Vec::with_capacity(n);
        let mut right = Vec::with_capacity(n);
        left.push(row[0].clone());
        right.push(row[n - 1].clone());
        for width in (1..n).rev() {
            for i in 0..width {
                let homogeneous = core::array::from_fn(|k| {
                    (row[i].homogeneous[k] + row[i + 1].homogeneous[k]) * I::p(0.5)
                });
                let exact = row[i]
                    .exact
                    .zip(row[i + 1].exact)
                    .and_then(|(a, b)| (a == b).then_some(a));
                let control = Arc::new(
                    Control::fresh(homogeneous, exact).ok_or(EnclosureError::NumericLimit)?,
                );
                row[i] = control;
            }
            left.push(row[0].clone());
            right.push(row[width - 1].clone());
        }
        right.reverse();
        Ok((
            Self { controls: left }.balanced(),
            Self { controls: right }.balanced(),
        ))
    }
    /// Encloses the circular arc from `from` to `to` whose bulge,
    /// `tan(sweep / 4)`, lies in `[bulge.lo, bulge.hi]`, as rational
    /// quadratic pieces from `from` to `to` in order: one for a bulge below
    /// `1/2` in magnitude, otherwise two sharing the arc's midpoint. The
    /// ends keep their exact identities.
    pub(crate) fn arc(
        from: [f64; 2],
        to: [f64; 2],
        bulge: [f64; 2],
    ) -> Result<Vec<Self>, EnclosureError> {
        let b = I {
            lo: bulge[0],
            hi: bulge[1],
        };
        if !b.valid() || !b.excludes_zero() || from == to {
            return Err(EnclosureError::InvalidInput);
        }
        let exact = |p: [f64; 2]| {
            Control::fresh([I::p(p[0]), I::p(p[1]), I::ONE], Some(p))
                .map(Arc::new)
                .ok_or(EnclosureError::NumericLimit)
        };
        let (start, end) = (exact(from)?, exact(to)?);
        let ends = |c: &Arc<Control>| c.bounds().ok_or(EnclosureError::NumericLimit);
        if b.lo.abs().max(b.hi.abs()) < 0.5 {
            let middle = quadratic_middle(ends(&start)?, ends(&end)?, b)?;
            return Ok(alloc::vec![Self {
                controls: alloc::vec![start, middle, end],
            }]);
        }
        // The midpoint is the sagitta, `bulge` half chords, to the chord's
        // right; each half has bulge `tan(sweep / 8) = b / (1 + sqrt(1 + b^2))`.
        let [p, q] = [ends(&start)?, ends(&end)?];
        let d = [q[0] - p[0], q[1] - p[1]];
        let half = I::p(0.5);
        let midpoint = [
            (p[0] + q[0]) * half + d[1] * b * half,
            (p[1] + q[1]) * half - d[0] * b * half,
        ];
        let mid = Arc::new(
            Control::fresh([midpoint[0], midpoint[1], I::ONE], None)
                .ok_or(EnclosureError::NumericLimit)?,
        );
        let m = mid.bounds().ok_or(EnclosureError::NumericLimit)?;
        let halves = b / (I::ONE + (I::ONE + b.square()).sqrt());
        Ok(alloc::vec![
            Self {
                controls: alloc::vec![start, quadratic_middle(p, m, halves)?, mid.clone()],
            },
            Self {
                controls: alloc::vec![mid, quadratic_middle(m, q, halves)?, end],
            },
        ])
    }

    /// Encloses every circular arc about `center` from the direction of
    /// `from` to that of `to` whose radius lies between their distances
    /// from it, and so every curve between those radii turning steadily
    /// between those directions: rational quadratic pieces in order,
    /// turning `quarters` exact quarter turns, counter-clockwise unless
    /// `clockwise`, and then the rest of the way, which must certainly turn
    /// the same way by less than half a turn. The ends are boxes about the
    /// radii's disagreement, represented by `from` and `to`, which they hold.
    pub(crate) fn band(
        center: [f64; 2],
        from: [f64; 2],
        to: [f64; 2],
        clockwise: bool,
        quarters: usize,
    ) -> Result<Vec<Self>, EnclosureError> {
        let c = center.map(I::p);
        let relative = |p: [f64; 2]| [I::p(p[0]) - c[0], I::p(p[1]) - c[1]];
        let (a, b) = (relative(from), relative(to));
        let (ra, rb) = (I::norm(a), I::norm(b));
        if !(ra.valid() && rb.valid() && ra.lo > 0.0 && rb.lo > 0.0) {
            return Err(EnclosureError::NumericLimit);
        }
        let radius = ra.union(rb);
        let unit = |v: [I; 2], r: I| [v[0] / r, v[1] / r];
        // A quarter turn, exact in every interval.
        let turn = |d: [I; 2]| {
            if clockwise {
                [d[1], -d[0]]
            } else {
                [-d[1], d[0]]
            }
        };
        let mut directions = alloc::vec![unit(a, ra)];
        for _ in 0..quarters {
            directions.push(turn(directions[directions.len() - 1]));
        }
        let rest = directions[directions.len() - 1];
        directions.push(unit(b, rb));
        let end = directions[directions.len() - 1];
        // The rest's turn must be certain, and the travel's way.
        let sine = rest[0] * end[1] - rest[1] * end[0];
        let travel = if clockwise { -sine } else { sine };
        if !(travel.valid() && travel.lo > 0.0) {
            return Err(EnclosureError::NumericLimit);
        }
        let last = directions.len() - 1;
        let junctions = directions
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let homogeneous = [c[0] + radius * d[0], c[1] + radius * d[1], I::ONE];
                let control = if i == 0 || i == last {
                    let control = Control {
                        homogeneous,
                        exact: None,
                        point: Arc::new(if i == 0 { from } else { to }),
                    };
                    control.bounds().is_some().then_some(control)
                } else {
                    Control::fresh(homogeneous, None)
                };
                control.map(Arc::new).ok_or(EnclosureError::NumericLimit)
            })
            .collect::<Result<Vec<_>, _>>()?;
        directions
            .windows(2)
            .zip(junctions.windows(2))
            .map(|(d, ends)| {
                // The tangents meet over the middle direction `m`, at `R / cos`
                // from the center, `cos` the half turn's, also the weight.
                let s = [d[0][0] + d[1][0], d[0][1] + d[1][1]];
                let size = I::norm(s);
                if !(size.valid() && size.lo > 0.0) {
                    return Err(EnclosureError::NumericLimit);
                }
                let m = unit(s, size);
                let cos = d[0][0] * m[0] + d[0][1] * m[1];
                if !(cos.valid() && cos.lo > 0.0) {
                    return Err(EnclosureError::NumericLimit);
                }
                let homogeneous = [c[0] * cos + radius * m[0], c[1] * cos + radius * m[1], cos];
                let middle = Control::fresh(homogeneous, None)
                    .filter(|_| homogeneous.iter().all(|v| v.valid()))
                    .map(Arc::new)
                    .ok_or(EnclosureError::NumericLimit)?;
                Ok(Self {
                    controls: alloc::vec![ends[0].clone(), middle, ends[1].clone()],
                })
            })
            .collect()
    }

    /// How far the curve's ends may lie from their representatives: the
    /// farther corner of either end's box, in model units.
    pub(crate) fn end_spread(&self) -> f64 {
        [&self.controls[0], &self.controls[self.controls.len() - 1]]
            .into_iter()
            .map(|c| {
                let b = c.bounds().expect("checked controls");
                let p = *c.point;
                let off = |k: usize| (b[k].lo - p[k]).abs().max((b[k].hi - p[k]).abs());
                (off(0) + off(1)).next_up()
            })
            .fold(0.0, f64::max)
    }

    /// The box of the controls' enclosures, which holds the curve.
    pub(crate) fn bounds(&self) -> [[f64; 2]; 2] {
        self.controls.iter().fold(
            [[f64::INFINITY; 2], [f64::NEG_INFINITY; 2]],
            |[lo, hi], control| {
                let b = control.bounds().expect("checked controls");
                [
                    [lo[0].min(b[0].lo), lo[1].min(b[1].lo)],
                    [hi[0].max(b[0].hi), hi[1].max(b[1].hi)],
                ]
            },
        )
    }

    /// Whether halving cannot sharpen the proof: the curve is no larger
    /// than eight times the rounding its controls carry, or than the
    /// rounding of distances at its coordinates.
    pub(crate) fn is_unresolvable(&self) -> bool {
        let [lo, hi] = self.bounds();
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let scale = [lo, hi]
            .into_iter()
            .flatten()
            .fold(0.0_f64, |m, c| m.max(c.abs()));
        let rounding = self
            .controls
            .iter()
            .filter_map(|c| c.bounds())
            .flat_map(|b| b.map(I::width))
            .fold(0.0_f64, f64::max);
        size <= (8.0 * rounding).max(16.0 * f64::EPSILON * scale)
    }

    /// Representative endpoints for a chord profile. A derived endpoint is the
    /// midpoint of its enclosure, not an exact evaluation of the defining curve.
    /// Sibling pieces return identical representatives for their shared endpoint.
    #[must_use]
    pub fn endpoints(&self) -> [[f64; 2]; 2] {
        [
            self.controls.first().expect("nonempty"),
            self.controls.last().expect("nonempty"),
        ]
        .map(|p| *p.point)
    }
    /// Whether one coordinate is proved strictly monotone along the curve.
    /// Positive rational weights and ordered Bezier control projections establish
    /// injectivity; this says nothing about intersections with other pieces.
    #[must_use]
    pub fn is_monotone(&self) -> bool {
        let bounds: [[I; 2]; MAX_ORDER] = core::array::from_fn(|i| {
            self.controls
                .get(i)
                .map_or([I::ZERO; 2], |p| p.bounds().expect("checked controls"))
        });
        let bounds = &bounds[..self.controls.len()];
        (0..2).any(|k| {
            (bounds[0][k].hi < bounds[bounds.len() - 1][k].lo
                && bounds.windows(2).all(|p| p[0][k].hi <= p[1][k].lo))
                || (bounds[0][k].lo > bounds[bounds.len() - 1][k].hi
                    && bounds.windows(2).all(|p| p[0][k].lo >= p[1][k].hi))
        })
    }
    /// Proves disjoint hulls, or contact only at one identical endpoint.
    ///
    /// `None` means unresolved, including overlapping hulls, tangencies,
    /// coincident curves and insufficient interval precision. It never means a
    /// successful no-crossing result. Runtime is bounded by `MAX_ORDER` per pair.
    #[must_use]
    pub fn separation(&self, other: &Self) -> Option<HullSeparation> {
        self.separation_by(other, 0.0)
    }

    /// As [`Self::separation`], with [`HullSeparation::Disjoint`] proved
    /// only for hulls at least `margin` apart, so their curves are too. A
    /// shared endpoint is judged as there, without the margin.
    #[must_use]
    pub fn separation_by(&self, other: &Self, margin: f64) -> Option<HullSeparation> {
        if !(margin >= 0.0 && margin.is_finite()) {
            return None;
        }
        let a = &self.controls;
        let b = &other.controls;
        let mut common = 0;
        let mut shared = None;
        for i in [0, a.len() - 1] {
            for j in [0, b.len() - 1] {
                if same(&a[i], &b[j]) {
                    common += 1;
                    shared = Some(&a[i]);
                }
            }
        }
        if common != 1 {
            shared = None;
        }
        // Axis-aligned support lines through the common endpoint avoid losing
        // exact collinearity of a straight continuation to interval subtraction.
        if let Some(join) = shared {
            for k in 0..2 {
                let side = |p: &Arc<Control>| {
                    if same(p, join) {
                        I::ZERO
                    } else {
                        p.bounds().expect("checked")[k] - join.bounds().expect("checked")[k]
                    }
                };
                if separates(a, b, shared, side, f64::INFINITY).is_some() {
                    return Some(HullSeparation::SharedEndpoint);
                }
            }
        }
        for controls in [a, b] {
            for (i, from) in controls.iter().enumerate() {
                for to in &controls[i + 1..] {
                    if same(from, to) {
                        continue;
                    }
                    let side = |p: &Arc<Control>| {
                        if same(p, from) || same(p, to) {
                            return I::ZERO;
                        }
                        if let (0.0, Some(a), Some(b), Some(c)) =
                            (margin, from.exact, to.exact, p.exact)
                        {
                            return I::p(match orient2d(a, b, c) {
                                Orientation::Ccw => 1.0,
                                Orientation::Cw => -1.0,
                                Orientation::Collinear => 0.0,
                            });
                        }
                        let a = from.bounds().expect("checked");
                        let b = to.bounds().expect("checked");
                        let c = p.bounds().expect("checked");
                        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
                    };
                    // The sides are cross products with the edge, its length
                    // times the distance from its line.
                    let gap = if margin == 0.0 {
                        0.0
                    } else {
                        let ends = from.bounds().zip(to.bounds())?;
                        let edge = [ends.1[0] - ends.0[0], ends.1[1] - ends.0[1]];
                        let length = I::norm(edge);
                        (I::p(margin) * length).hi
                    };
                    if !gap.is_finite() {
                        continue;
                    }
                    if let Some(result) = separates(a, b, shared, side, gap) {
                        return Some(result);
                    }
                }
            }
        }
        // Lines across the curves rather than along their hulls' edges:
        // perpendicular to the axes and to each pair of controls, as
        // separate pieces lying end to end along one line need.
        if shared.is_none() && across(a, b, margin) {
            return Some(HullSeparation::Disjoint);
        }
        None
    }
}
/// Whether a line perpendicular to an axis, or to the direction between
/// two controls of either hull, has the hulls on its two sides at least
/// `margin` apart (strictly apart for no margin).
fn across(a: &[Arc<Control>], b: &[Arc<Control>], margin: f64) -> bool {
    let bounds = |controls: &[Arc<Control>]| -> Option<Vec<[I; 2]>> {
        controls.iter().map(|c| c.bounds()).collect()
    };
    let (Some(pa), Some(pb)) = (bounds(a), bounds(b)) else {
        return false;
    };
    let mut directions = alloc::vec![[I::ONE, I::ZERO], [I::ZERO, I::ONE]];
    for points in [&pa, &pb] {
        for (i, from) in points.iter().enumerate() {
            for to in &points[i + 1..] {
                directions.push([to[0] - from[0], to[1] - from[1]]);
            }
        }
    }
    directions.into_iter().any(|d| {
        let project = |p: &[I; 2]| p[0] * d[0] + p[1] * d[1];
        let span = |points: &[[I; 2]]| {
            points
                .iter()
                .map(project)
                .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], v| {
                    [lo.min(v.lo), hi.max(v.hi)]
                })
        };
        let ([a_lo, a_hi], [b_lo, b_hi]) = (span(&pa), span(&pb));
        if ![a_lo, a_hi, b_lo, b_hi].iter().all(|v| v.is_finite()) {
            return false;
        }
        let length = I::norm(d);
        if !length.valid() || length.lo <= 0.0 {
            return false;
        }
        let gap = (I::p(margin) * length).hi;
        let apart = |low: f64, high: f64| {
            let between = (I::p(high) - I::p(low)).lo;
            if margin == 0.0 {
                between > 0.0
            } else {
                between >= gap
            }
        };
        apart(a_hi, b_lo) || apart(b_hi, a_lo)
    })
}

/// The weighted middle control of the rational quadratic circular arc
/// from `p` to `q` with bulge `b`, `|b| < 1`: the tangents' meeting point
/// `tan(sweep / 2) = 2 b / (1 - b^2)` half chords to the chord's right,
/// with weight `cos(sweep / 2) = (1 - b^2) / (1 + b^2)`.
fn quadratic_middle(p: [I; 2], q: [I; 2], b: I) -> Result<Arc<Control>, EnclosureError> {
    let b2 = b.square();
    let (minus, plus) = (I::ONE - b2, I::ONE + b2);
    if minus.lo <= 0.0 {
        return Err(EnclosureError::NumericLimit);
    }
    let d = [q[0] - p[0], q[1] - p[1]];
    let reach = b / minus;
    let half = I::p(0.5);
    let point = [
        (p[0] + q[0]) * half + d[1] * reach,
        (p[1] + q[1]) * half - d[0] * reach,
    ];
    let weight = minus / plus;
    let homogeneous = [point[0] * weight, point[1] * weight, weight];
    if homogeneous.iter().any(|v| !v.valid()) {
        return Err(EnclosureError::NumericLimit);
    }
    Control::fresh(homogeneous, None)
        .map(Arc::new)
        .ok_or(EnclosureError::NumericLimit)
}

/// Whether the line `side` measures separates the hulls: `Disjoint` when
/// they lie on its two sides at least `gap` apart in `side`'s units (any
/// strict separation when `gap` is zero), `SharedEndpoint` when they meet
/// only at `shared`. An infinite `gap` asks only for the latter.
fn separates(
    a: &[Arc<Control>],
    b: &[Arc<Control>],
    shared: Option<&Arc<Control>>,
    side: impl Fn(&Arc<Control>) -> I,
    gap: f64,
) -> Option<HullSeparation> {
    let a_sides: [I; MAX_ORDER] = core::array::from_fn(|i| a.get(i).map_or(I::ZERO, &side));
    let b_sides: [I; MAX_ORDER] = core::array::from_fn(|i| b.get(i).map_or(I::ZERO, &side));
    for reverse in [false, true] {
        let left = |i: usize| if reverse { -a_sides[i] } else { a_sides[i] };
        let right = |i: usize| if reverse { -b_sides[i] } else { b_sides[i] };
        if (0..a.len()).any(|i| !left(i).valid() || left(i).hi > 0.0)
            || (0..b.len()).any(|i| !right(i).valid() || right(i).lo < 0.0)
        {
            continue;
        }
        let disjoint = if gap == 0.0 {
            (0..a.len()).all(|i| left(i).hi < 0.0) || (0..b.len()).all(|i| right(i).lo > 0.0)
        } else {
            let highest = (0..a.len())
                .map(|i| left(i).hi)
                .fold(f64::NEG_INFINITY, f64::max);
            let lowest = (0..b.len())
                .map(|i| right(i).lo)
                .fold(f64::INFINITY, f64::min);
            (I::p(lowest) - I::p(highest)).lo >= gap
        };
        if disjoint {
            return Some(HullSeparation::Disjoint);
        }
        if let Some(join) = shared
            && ((0..a.len()).all(|i| left(i).hi < 0.0 || same(&a[i], join))
                || (0..b.len()).all(|i| right(i).lo > 0.0 || same(&b[i], join)))
        {
            return Some(HullSeparation::SharedEndpoint);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounded_splits_keep_shared_identity_and_refuse_near_crossings() {
        for scale in [1e-12, 1.0, 1e12] {
            let points = [
                [0.0, 0.0],
                [scale, 2.0 * scale],
                [3.0 * scale, 2.0 * scale],
                [4.0 * scale, 0.0],
            ];
            let curve = BezierEnclosure::new(&points, Some(&[1.0, 0.7, 1.4, 1.0])).unwrap();
            let (left, right) = curve.bisect().unwrap();
            assert_eq!(left.endpoints()[1], right.endpoints()[0]);
            assert!(left.is_monotone() && right.is_monotone());
            assert_eq!(
                left.separation(&right),
                Some(HullSeparation::SharedEndpoint)
            );
            let crossing = BezierEnclosure::new(
                &[
                    [0.0, 0.0],
                    [scale * 1e-10, -scale * 1e-10],
                    [3.0 * scale, 2.0 * scale],
                    [4.0 * scale, 0.0],
                ],
                Some(&[1.0, 0.7, 1.4, 1.0]),
            )
            .unwrap();
            let chord = BezierEnclosure::new(&[[0.0, 0.0], [4.0 * scale, 0.0]], None).unwrap();
            let (mut near, _) = crossing.bisect().unwrap();
            for _ in 0..8 {
                let (left, _) = near.bisect().unwrap();
                near = left;
            }
            assert_eq!(near.separation(&chord), None);
        }
    }
    #[test]
    fn reweighting_is_all_or_nothing() {
        // Reweighting would overflow one control; the curve still crosses
        // the horizontal at three quarters height, and no proof says not.
        let scale = 1e100;
        let mut points: Vec<[f64; 2]> = (0..8).map(|i| [f64::from(i) * scale / 8.0, 0.0]).collect();
        points[6][1] = scale;
        let mut weights = alloc::vec![2.0_f64.powi(256); 8];
        weights[7] = 2.0_f64.powi(-256);
        let curve = BezierEnclosure::new(&points, Some(&weights)).unwrap();
        let line =
            BezierEnclosure::new(&[[0.0, 0.75 * scale], [scale, 0.75 * scale]], None).unwrap();
        assert_ne!(curve.separation(&line), Some(HullSeparation::Disjoint));
        // Halving it may be refused, but never proves the halves apart.
        if let Ok((left, right)) = curve.bisect() {
            for piece in [&left, &right] {
                assert_ne!(piece.separation(&line), Some(HullSeparation::Disjoint));
            }
        }
        // Reweighting that would lose a control's point is not made.
        let light = BezierEnclosure::new(
            &[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]],
            Some(&[1e-310, 1e-10, 1e-10]),
        )
        .unwrap();
        if let Ok((left, right)) = light.bisect() {
            let _ = (left.endpoints(), right.endpoints());
        }
    }
    #[test]
    fn reweighted_halves_agree_on_their_shared_end() {
        let curve = BezierEnclosure::new(
            &[[0.0, 0.0], [1.0, 2.0], [3.0, 0.0]],
            Some(&[1.0, 8.0, 1.0]),
        )
        .unwrap();
        let (left, right) = curve.bisect().unwrap();
        assert_eq!(left.endpoints()[1], right.endpoints()[0]);
        let (a, b) = right.bisect().unwrap();
        assert_eq!(a.endpoints()[1], b.endpoints()[0]);
        assert_eq!(left.endpoints()[1], a.endpoints()[0]);
    }
}
