// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Axis-aligned boxes: regions fields are bounded over, and where their
//! interiors lie.

use exedra_math::Real;

use crate::number::Interval;

/// A closed axis-aligned box with finite corners.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb {
    /// Least corner.
    pub min: [f64; 3],
    /// Greatest corner.
    pub max: [f64; 3],
}

impl Aabb {
    /// The box from `min` to `max`, when both are finite and `min <= max`
    /// on every axis.
    #[must_use]
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Option<Self> {
        let valid = (0..3).all(|i| min[i].is_finite() && max[i].is_finite() && min[i] <= max[i]);
        valid.then_some(Self { min, max })
    }

    /// The box holding exactly `point`.
    ///
    /// # Panics
    ///
    /// Panics when `point` is not finite.
    #[must_use]
    pub fn point(point: [f64; 3]) -> Self {
        Self::new(point, point).expect("a finite point")
    }

    /// The midpoint.
    #[must_use]
    pub fn center(&self) -> [f64; 3] {
        core::array::from_fn(|i| 0.5 * (self.min[i] + self.max[i]))
    }

    /// The length along each axis.
    #[must_use]
    pub fn extent(&self) -> [f64; 3] {
        core::array::from_fn(|i| self.max[i] - self.min[i])
    }

    /// Half the length of the diagonal.
    #[must_use]
    pub fn half_diagonal(&self) -> f64 {
        let [x, y, z] = self.extent();
        0.5 * Real::sqrt(x * x + y * y + z * z)
    }

    /// Whether `point` lies in the box, boundary included.
    #[must_use]
    pub fn contains(&self, point: [f64; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= point[i] && point[i] <= self.max[i])
    }

    /// The smallest box holding both.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            min: core::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: core::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }

    /// The box both hold, or `None` when they do not meet.
    #[must_use]
    pub fn overlap(&self, other: &Self) -> Option<Self> {
        Self::new(
            core::array::from_fn(|i| self.min[i].max(other.min[i])),
            core::array::from_fn(|i| self.max[i].min(other.max[i])),
        )
    }

    /// The box grown by `margin` on every side, rounded outward.
    #[must_use]
    pub fn grown(&self, margin: f64) -> Option<Self> {
        outward(self.min.map(|v| v - margin), self.max.map(|v| v + margin))
    }

    /// The coordinates the box spans, as intervals.
    #[must_use]
    pub fn intervals(&self) -> [Interval; 3] {
        core::array::from_fn(|i| Interval::new(self.min[i], self.max[i]))
    }

    /// The eight corners, `x` varying fastest.
    #[must_use]
    pub fn corners(&self) -> [[f64; 3]; 8] {
        core::array::from_fn(|index| {
            core::array::from_fn(|axis| {
                if index >> axis & 1 == 0 {
                    self.min[axis]
                } else {
                    self.max[axis]
                }
            })
        })
    }
}

/// A closed axis-aligned rectangle with finite corners, for profiles.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Aabb2 {
    /// Least corner.
    pub min: [f64; 2],
    /// Greatest corner.
    pub max: [f64; 2],
}

impl Aabb2 {
    /// The rectangle from `min` to `max`, when both are finite and
    /// `min <= max` on both axes.
    #[must_use]
    pub fn new(min: [f64; 2], max: [f64; 2]) -> Option<Self> {
        let valid = (0..2).all(|i| min[i].is_finite() && max[i].is_finite() && min[i] <= max[i]);
        valid.then_some(Self { min, max })
    }

    /// The smallest rectangle holding both.
    #[must_use]
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            min: core::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: core::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }

    /// The rectangle both hold, or `None` when they do not meet.
    #[must_use]
    pub fn overlap(&self, other: &Self) -> Option<Self> {
        Self::new(
            core::array::from_fn(|i| self.min[i].max(other.min[i])),
            core::array::from_fn(|i| self.max[i].min(other.max[i])),
        )
    }
}

/// The length of `v`, scaled so no square underflows or overflows, and
/// rounded up past its rounding error.
pub(crate) fn length_up(v: [f64; 3]) -> f64 {
    scaled_length(v) * (1.0 + 8.0 * f64::EPSILON)
}

/// The length of `v`, scaled so no square underflows or overflows, and
/// rounded down past its rounding error.
pub(crate) fn length_down(v: [f64; 3]) -> f64 {
    scaled_length(v) * (1.0 - 8.0 * f64::EPSILON)
}

fn scaled_length(v: [f64; 3]) -> f64 {
    let largest = v.iter().fold(0.0_f64, |m, c| m.max(c.abs()));
    if largest == 0.0 || !largest.is_finite() {
        return largest;
    }
    let [x, y, z] = v.map(|c| c / largest);
    largest * Real::sqrt(x * x + y * y + z * z)
}

/// The box from corners rounded outward by one ulp, so arithmetic that
/// formed them cannot shrink it; `None` unless finite and ordered.
pub(crate) fn outward(min: [f64; 3], max: [f64; 3]) -> Option<Aabb> {
    Aabb::new(min.map(f64::next_down), max.map(f64::next_up))
}

/// The 2D counterpart of [`outward`].
pub(crate) fn outward2(min: [f64; 2], max: [f64; 2]) -> Option<Aabb2> {
    Aabb2::new(min.map(f64::next_down), max.map(f64::next_up))
}
