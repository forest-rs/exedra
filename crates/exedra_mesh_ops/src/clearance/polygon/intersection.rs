// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Resolve a known transverse crossing only when its witness is well-conditioned.

use super::PolygonClearanceError;
use core::ops::{Add, Mul, Sub};

#[derive(Clone, Copy)]
struct Interval {
    lo: f64,
    hi: f64,
}
impl Interval {
    fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }
    fn valid(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    fn divide(self, divisor: Self) -> Option<Self> {
        if !divisor.valid() || (divisor.lo <= 0.0 && divisor.hi >= 0.0) {
            return None;
        }
        let inverse = Self {
            lo: (1.0 / divisor.hi).next_down(),
            hi: (1.0 / divisor.lo).next_up(),
        };
        if !inverse.valid() {
            return None;
        }
        let result = self * inverse;
        result.valid().then_some(result)
    }
}
impl Add for Interval {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self {
            lo: (self.lo + b.lo).next_down(),
            hi: (self.hi + b.hi).next_up(),
        }
    }
}
impl Sub for Interval {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self {
            lo: (self.lo - b.hi).next_down(),
            hi: (self.hi - b.lo).next_up(),
        }
    }
}
impl Mul for Interval {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let p = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        if p.iter().any(|v| !v.is_finite()) {
            return Self {
                lo: f64::NAN,
                hi: f64::NAN,
            };
        }
        Self {
            lo: p.into_iter().fold(f64::INFINITY, f64::min).next_down(),
            hi: p.into_iter().fold(f64::NEG_INFINITY, f64::max).next_up(),
        }
    }
}
fn cross(a: [Interval; 2], b: [Interval; 2]) -> Interval {
    a[0] * b[1] - a[1] * b[0]
}

pub(super) fn witness(
    a: [f64; 2],
    b: [f64; 2],
    c: [f64; 2],
    d: [f64; 2],
) -> Result<[f64; 2], PolygonClearanceError> {
    let difference = |p: [f64; 2], q: [f64; 2]| {
        core::array::from_fn(|i| Interval::point(p[i]) - Interval::point(q[i]))
    };
    let ab = difference(b, a);
    let cd = difference(d, c);
    let ac = difference(c, a);
    let t = cross(ac, cd)
        .divide(cross(ab, cd))
        .ok_or(PolygonClearanceError::NumericLimit)?;
    if t.lo <= 0.0 || t.hi >= 1.0 {
        return Err(PolygonClearanceError::NumericLimit);
    }
    let scale = [a, b, c, d]
        .into_iter()
        .flatten()
        .map(f64::abs)
        .fold(0.0, f64::max);
    let allowance = 64.0 * f64::EPSILON * scale;
    let mut point = [0.0; 2];
    for i in 0..2 {
        let enclosure = Interval::point(a[i]) + t * ab[i];
        if !enclosure.valid() || (enclosure.hi - enclosure.lo).next_up() > allowance {
            return Err(PolygonClearanceError::NumericLimit);
        }
        point[i] = enclosure.lo * 0.5 + enclosure.hi * 0.5;
    }
    Ok(point)
}
