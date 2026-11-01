// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Control-hull proofs which retain subdivision roundoff and shared endpoints.

use super::{IntersectError, MAX_ORDER, interval::I};
use alloc::{sync::Arc, vec::Vec};
use exedra_math::predicates::{MAX_COORDINATE, Orientation, orient2d};

#[derive(Clone, Debug)]
struct Control {
    homogeneous: [I; 3],
    exact: Option<[f64; 2]>,
}
impl Control {
    fn bounds(&self) -> Option<[I; 2]> {
        if let Some(p) = self.exact {
            return Some(p.map(I::p));
        }
        let [x, y, w] = self.homogeneous;
        if w.lo <= 0.0 {
            return None;
        }
        let p = [x * w.recip(), y * w.recip()];
        p.iter()
            .all(|v| v.valid() && v.lo.abs().max(v.hi.abs()) <= MAX_COORDINATE)
            .then_some(p)
    }
}
fn same(a: &Arc<Control>, b: &Arc<Control>) -> bool {
    Arc::ptr_eq(a, b) || a.exact.zip(b.exact).is_some_and(|(a, b)| a == b)
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
    /// Returns [`IntersectError::Unresolved`] for invalid inputs, arithmetic
    /// overflow/underflow that loses positive weight, or out-of-envelope controls.
    pub fn new(points: &[[f64; 2]], weights: Option<&[f64]>) -> Result<Self, IntersectError> {
        if !(2..=MAX_ORDER).contains(&points.len())
            || weights.is_some_and(|w| w.len() != points.len())
        {
            return Err(IntersectError::Unresolved);
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
                    return Err(IntersectError::Unresolved);
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
                    return Err(IntersectError::Unresolved);
                }
                Ok(Arc::new(Control {
                    homogeneous,
                    exact: Some(p),
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { controls })
    }
    /// Bisects at parameter `1/2`, including all homogeneous arithmetic roundoff.
    /// The returned curves retain the same parameter direction.
    ///
    /// # Errors
    /// Returns [`IntersectError::Unresolved`] if finite positive enclosures cannot
    /// be maintained. The input remains available unchanged.
    pub fn bisect(&self) -> Result<(Self, Self), IntersectError> {
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
                let control = Arc::new(Control { homogeneous, exact });
                if control.bounds().is_none() {
                    return Err(IntersectError::Unresolved);
                }
                row[i] = control;
            }
            left.push(row[0].clone());
            right.push(row[width - 1].clone());
        }
        right.reverse();
        Ok((Self { controls: left }, Self { controls: right }))
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
        .map(|p| {
            p.exact
                .unwrap_or_else(|| p.bounds().expect("checked controls").map(I::mid))
        })
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
                if separates(a, b, shared, side).is_some() {
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
                        if let (Some(a), Some(b), Some(c)) = (from.exact, to.exact, p.exact) {
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
                    if let Some(result) = separates(a, b, shared, side) {
                        return Some(result);
                    }
                }
            }
        }
        None
    }
}
fn separates(
    a: &[Arc<Control>],
    b: &[Arc<Control>],
    shared: Option<&Arc<Control>>,
    side: impl Fn(&Arc<Control>) -> I,
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
        if (0..a.len()).all(|i| left(i).hi < 0.0) || (0..b.len()).all(|i| right(i).lo > 0.0) {
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
}
