// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Operators that reshape space before a field sees it, after Íñigo
//! Quilez's [distance functions](https://iquilezles.org/articles/distfunctions/).
//!
//! - [`Elongate`] pulls a shape apart about a point, inserting straight
//!   runs. Distances stay bounded: the result is 1-Lipschitz wherever the
//!   field was.
//! - [`Twist`] turns each slice along `z` in proportion to its height. It
//!   stretches space more the farther from the axis, without bound, so it
//!   claims no metric.
//! - [`Repeat`] lays out copies on a grid. When the original lies within
//!   half a spacing of the origin, so each copy fits within its own grid
//!   cell, the nearest copy is among the two nearest cells on each axis, and
//!   checking those keeps an exact distance exact.

use exedra_math::Real;

use crate::capability::{Capabilities, FieldCapabilities, Metric, Support, outward};
use crate::spatial::Aabb;
use crate::{
    FieldError, ProvenanceField, ScalarField, check_finite, check_nonnegative, check_positive,
};

/// Wrapper elongating a field about `center` by `half_extents` per axis.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Elongate<F> {
    field: F,
    center: [f32; 3],
    half_extents: [f32; 3],
}

/// Wrapper twisting a field about the `z` axis through `center`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Twist<F> {
    field: F,
    center: [f32; 3],
    rate: f32,
}

/// Wrapper repeating a field on a grid of `counts` copies, `spacing` apart.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Repeat<F> {
    field: F,
    spacing: [f32; 3],
    counts: [u32; 3],
}

impl<F> Elongate<F> {
    /// Creates an elongation: the half space beyond `center` on each axis
    /// moves out by that axis's half extent, and the gap fills with the
    /// shape's cross-section there.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite and the half
    /// extents are not negative.
    pub fn new(field: F, center: [f32; 3], half_extents: [f32; 3]) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        for extent in half_extents {
            check_nonnegative("half_extents", extent)?;
        }
        Ok(Self {
            field,
            center,
            half_extents,
        })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    fn fold(&self, p: [f32; 3]) -> [f32; 3] {
        core::array::from_fn(|i| {
            let (c, h) = (self.center[i], self.half_extents[i]);
            let offset = p[i] - c;
            c + offset - offset.clamp(-h, h)
        })
    }
}

impl<F> Twist<F> {
    /// Creates a twist turning each slice by `rate` radians per unit of
    /// height above `center`, counterclockwise looking down `z`.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every value is finite.
    pub fn new(field: F, center: [f32; 3], rate: f32) -> Result<Self, FieldError> {
        check_finite("center", center)?;
        check_finite("rate", [rate])?;
        Ok(Self {
            field,
            center,
            rate,
        })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// The untwisted point, in `f64`, and its slice's turn.
    fn untwist(&self, p: [f32; 3]) -> ([f64; 3], f64) {
        let c = self.center.map(f64::from);
        let p = p.map(f64::from);
        let angle = f64::from(self.rate) * (p[2] - c[2]);
        let (sin, cos) = Real::sin_cos(angle);
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        (
            [c[0] + cos * x + sin * y, c[1] - sin * x + cos * y, p[2]],
            angle,
        )
    }
}

impl<F> Repeat<F> {
    /// Creates `counts[i]` copies along each axis, the `n`th offset by `n`
    /// times `spacing[i]`; the original is copy zero.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] unless every count is positive and the spacing
    /// is finite and, on each axis with more than one copy, positive.
    pub fn new(field: F, spacing: [f32; 3], counts: [u32; 3]) -> Result<Self, FieldError> {
        check_finite("spacing", spacing)?;
        for axis in 0..3 {
            if counts[axis] == 0 {
                return Err(FieldError::NonPositive {
                    parameter: "counts",
                });
            }
            if counts[axis] > 1 {
                check_positive("spacing", spacing[axis])?;
            }
        }
        Ok(Self {
            field,
            spacing,
            counts,
        })
    }

    /// Returns the wrapped field.
    #[must_use]
    pub const fn field(&self) -> &F {
        &self.field
    }

    /// The copy index nearest `p` on `axis`, and its neighbor on `p`'s side.
    fn nearest(&self, p: f32, axis: usize) -> [u32; 2] {
        let last = self.counts[axis] - 1;
        if last == 0 {
            return [0, 0];
        }
        let s = self.spacing[axis];
        // Rounding to the nearest copy: past the clamp the value is
        // nonnegative, where truncation floors.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to the copy range"
        )]
        let index = ((p / s + 0.5).clamp(0.0, last as f32) as u32).min(last);
        let position = index as f32;
        let toward = if p > s * position {
            index + 1
        } else {
            index.saturating_sub(1)
        };
        [index, toward.min(last)]
    }

    /// The local points of the copies that can be nearest `p`.
    fn candidates(&self, p: [f32; 3]) -> [[f32; 3]; 8] {
        let ids = [0, 1, 2].map(|axis| self.nearest(p[axis], axis));
        core::array::from_fn(|corner| {
            core::array::from_fn(|axis| {
                let index = ids[axis][(corner >> axis) & 1];
                p[axis] - self.spacing[axis] * index as f32
            })
        })
    }

    /// The copy index range that can be nearest any point of `bounds`.
    fn index_range(&self, bounds: &Aabb, axis: usize) -> [u32; 2] {
        let low = self.nearest(bounds.min[axis], axis);
        let high = self.nearest(bounds.max[axis], axis);
        [low[0].min(low[1]), high[0].max(high[1])]
    }
}

impl<F: ScalarField> ScalarField for Elongate<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        // Folding is 1-Lipschitz, so bounds survive; inside the inserted run
        // the nearest surface may lie across it, so exactness does not.
        let support = inner.support.map(|b| {
            outward(
                core::array::from_fn(|i| b.min[i] - self.half_extents[i]),
                core::array::from_fn(|i| b.max[i] + self.half_extents[i]),
            )
        });
        Capabilities {
            metric: inner
                .metric
                .lipschitz()
                .map_or(Metric::Unknown, Metric::Lipschitz),
            support,
        }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        // Folding is nondecreasing on each axis, so a box folds to a box.
        let folded = Aabb::new(self.fold(bounds.min), self.fold(bounds.max))?;
        self.field.eval_interval(&folded)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (point, out) in points.iter().zip(out.chunks_mut(1)) {
            self.field.eval_points(&[self.fold(*point)], out);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (point, row) in points.iter().zip(out.chunks_mut(1)) {
            self.field.eval_gradients(&[self.fold(*point)], row);
            // Within the run the fold is flat, so the value cannot change
            // along that axis.
            for axis in 0..3 {
                if (point[axis] - self.center[axis]).abs() < self.half_extents[axis] {
                    row[0][axis + 1] = 0.0;
                }
            }
        }
    }
}

impl<F: ScalarField> ScalarField for Twist<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        // Turning keeps each point's height and distance from the axis.
        let support = inner.support.map(|b| {
            let reach = radial_reach(&b, self.center);
            outward(
                [self.center[0] - reach, self.center[1] - reach, b.min[2]],
                [self.center[0] + reach, self.center[1] + reach, b.max[2]],
            )
        });
        Capabilities {
            metric: Metric::Unknown,
            support,
        }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        // Every slice turns by an angle within the box's range; turning the
        // box by the middle angle leaves each point within `reach * half`
        // of where any other angle in range would put it.
        let rate = f64::from(self.rate);
        let cz = f64::from(self.center[2]);
        let low = rate * (f64::from(bounds.min[2]) - cz);
        let high = rate * (f64::from(bounds.max[2]) - cz);
        let half = 0.5 * (high - low).abs();
        let reach = f64::from(radial_reach(bounds, self.center));
        let c = self.center.map(f64::from);
        let (mut minimum, mut maximum) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        if half >= core::f64::consts::FRAC_PI_2 {
            minimum = [c[0] - reach, c[1] - reach];
            maximum = [c[0] + reach, c[1] + reach];
        } else {
            let middle = 0.5 * (low + high);
            let (sin, cos) = Real::sin_cos(middle);
            for x in [bounds.min[0], bounds.max[0]] {
                for y in [bounds.min[1], bounds.max[1]] {
                    let (x, y) = (f64::from(x) - c[0], f64::from(y) - c[1]);
                    let turned = [c[0] + cos * x + sin * y, c[1] - sin * x + cos * y];
                    for axis in 0..2 {
                        minimum[axis] = minimum[axis].min(turned[axis] - reach * half);
                        maximum[axis] = maximum[axis].max(turned[axis] + reach * half);
                    }
                }
            }
        }
        let margin = 16.0 * f64::EPSILON * (reach + c[0].abs() + c[1].abs());
        #[expect(clippy::cast_possible_truncation, reason = "outward rounding follows")]
        let narrow = |v: f64| v as f32;
        let local = outward(
            [
                narrow(minimum[0] - margin),
                narrow(minimum[1] - margin),
                bounds.min[2],
            ],
            [
                narrow(maximum[0] + margin),
                narrow(maximum[1] + margin),
                bounds.max[2],
            ],
        )?;
        self.field.eval_interval(&local)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (point, out) in points.iter().zip(out.chunks_mut(1)) {
            let (local, _) = self.untwist(*point);
            self.field.eval_points(&[narrow3(local)], out);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (point, row) in points.iter().zip(out.chunks_mut(1)) {
            let (local, angle) = self.untwist(*point);
            self.field.eval_gradients(&[narrow3(local)], row);
            let [value, gx, gy, gz] = row[0].map(f64::from);
            // The chain rule through the untwist: the slice turns back by
            // the angle, and moving up turns it further by `rate`.
            let (sin, cos) = Real::sin_cos(angle);
            let rate = f64::from(self.rate);
            let (lx, ly) = (
                local[0] - f64::from(self.center[0]),
                local[1] - f64::from(self.center[1]),
            );
            row[0] = narrow4([
                value,
                cos * gx - sin * gy,
                sin * gx + cos * gy,
                gz + rate * (gx * ly - gy * lx),
            ]);
        }
    }
}

impl<F: ScalarField> ScalarField for Repeat<F> {
    fn capabilities(&self) -> FieldCapabilities {
        let inner = self.field.capabilities();
        let reach =
            core::array::from_fn::<f32, 3, _>(|i| self.spacing[i] * (self.counts[i] - 1) as f32);
        let support = inner.support.map(|b| {
            outward(
                core::array::from_fn(|i| b.min[i] + reach[i].min(0.0)),
                core::array::from_fn(|i| b.max[i] + reach[i].max(0.0)),
            )
        });
        // Checking two copies per axis finds the nearest only when every
        // copy keeps within its own cell.
        let in_cells = match inner.support {
            Support::Empty => true,
            Support::Bounded(b) | Support::Asserted(b) => (0..3).all(|i| {
                self.counts[i] == 1
                    || (-0.5 * self.spacing[i] < b.min[i] && b.max[i] < 0.5 * self.spacing[i])
            }),
            Support::Unbounded | Support::Unknown => false,
        };
        let metric = if in_cells {
            // Copies are disjoint, so even inside one the nearest surface
            // is its own.
            inner.metric
        } else {
            Metric::Unknown
        };
        Capabilities { metric, support }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let ranges = [0, 1, 2].map(|axis| self.index_range(bounds, axis));
        let mut interval = [f32::INFINITY, f32::NEG_INFINITY];
        for i in ranges[0][0]..=ranges[0][1] {
            for j in ranges[1][0]..=ranges[1][1] {
                for k in ranges[2][0]..=ranges[2][1] {
                    let offset = [i, j, k].map(|n| n as f32);
                    let offset: [f32; 3] =
                        core::array::from_fn(|axis| self.spacing[axis] * offset[axis]);
                    let shifted = outward(
                        core::array::from_fn(|axis| bounds.min[axis] - offset[axis]),
                        core::array::from_fn(|axis| bounds.max[axis] - offset[axis]),
                    )?;
                    let [low, high] = self.field.eval_interval(&shifted)?;
                    // Each point takes the least of some of these copies,
                    // so its value is at least the least lower bound and at
                    // most the greatest upper bound among them.
                    interval[0] = interval[0].min(low);
                    interval[1] = interval[1].max(high);
                }
            }
        }
        Some(interval)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (point, out) in points.iter().zip(out) {
            let candidates = self.candidates(*point);
            let mut values = [0.0; 8];
            self.field.eval_points(&candidates, &mut values);
            *out = values.into_iter().fold(f32::INFINITY, f32::min);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (point, row) in points.iter().zip(out) {
            let candidates = self.candidates(*point);
            let mut rows = [[0.0; 4]; 8];
            self.field.eval_gradients(&candidates, &mut rows);
            *row = rows
                .into_iter()
                .reduce(|best, row| if row[0] < best[0] { row } else { best })
                .unwrap_or([f32::NAN; 4]);
        }
    }
}

impl<F: ProvenanceField> ProvenanceField for Elongate<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let interval = self.eval_interval(bounds)?;
        Some((interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(self.fold(point))
    }
}

impl<F: ProvenanceField> ProvenanceField for Twist<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let interval = self.eval_interval(bounds)?;
        Some((interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        self.field.point_provenance(narrow3(self.untwist(point).0))
    }
}

impl<F: ProvenanceField> ProvenanceField for Repeat<F> {
    type Provenance = F::Provenance;

    fn eval_interval_with_provenance(&self, bounds: &Aabb) -> Option<([f32; 2], Self::Provenance)> {
        let interval = self.eval_interval(bounds)?;
        Some((interval, self.point_provenance(bounds.center())))
    }

    fn point_provenance(&self, point: [f32; 3]) -> Self::Provenance {
        let candidates = self.candidates(point);
        let mut values = [0.0; 8];
        self.field.eval_points(&candidates, &mut values);
        let mut nearest = 0;
        for (index, value) in values.iter().enumerate() {
            if *value < values[nearest] {
                nearest = index;
            }
        }
        self.field.point_provenance(candidates[nearest])
    }
}

/// The farthest any point of `bounds` lies from the `z` axis through
/// `center`, rounded up.
fn radial_reach(bounds: &Aabb, center: [f32; 3]) -> f32 {
    let dx = (f64::from(bounds.min[0]) - f64::from(center[0]))
        .abs()
        .max((f64::from(bounds.max[0]) - f64::from(center[0])).abs());
    let dy = (f64::from(bounds.min[1]) - f64::from(center[1]))
        .abs()
        .max((f64::from(bounds.max[1]) - f64::from(center[1])).abs());
    let reach = Real::sqrt(dx * dx + dy * dy);
    #[expect(clippy::cast_possible_truncation, reason = "rounded up below")]
    let rounded = reach as f32;
    if f64::from(rounded) < reach {
        rounded.next_up()
    } else {
        rounded
    }
}

#[expect(clippy::cast_possible_truncation, reason = "fields evaluate in f32")]
fn narrow3(p: [f64; 3]) -> [f32; 3] {
    p.map(|v| v as f32)
}

#[expect(clippy::cast_possible_truncation, reason = "fields evaluate in f32")]
fn narrow4(p: [f64; 4]) -> [f32; 4] {
    p.map(|v| v as f32)
}
