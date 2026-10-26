// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fields from caller functions.
//!
//! [`FormulaField`] and [`FnField`] turn a distance function into a
//! [`ScalarField`] without writing interval or gradient code. A
//! [`Formula`] is written once over any [`Number`], so one definition yields
//! values, exact gradients by evaluating it on [`Dual`] numbers, and bounds
//! over a box by evaluating it on [`Interval`]s. A closure is quicker to
//! write; [`FnField`] takes its gradients as central differences, seven
//! evaluations for each, and its bounds from the Lipschitz claim alone.
//!
//! The Lipschitz claim, which the caller vouches for, says no value in a box
//! differs from the value at its center by more than the factor times the
//! half diagonal. A formula's own bounds tighten as boxes shrink, where the
//! claim's stay proportional to the box; on a gyroid they halve the cells an
//! extraction visits.
//!
//! ```
//! use exedra_isosurface::function::{Formula, FormulaField};
//! use exedra_isosurface::number::Number;
//! use exedra_isosurface::{Aabb, Metric, ScalarField};
//!
//! // A gyroid sheet: each gradient component is at most sqrt(2), and
//! // together at most sqrt(3).
//! struct Gyroid;
//!
//! impl Formula for Gyroid {
//!     fn eval<N: Number>(&self, [x, y, z]: [N; 3]) -> N {
//!         x.sin() * y.cos() + y.sin() * z.cos() + z.sin() * x.cos()
//!     }
//! }
//!
//! let field = FormulaField::new(Gyroid, Metric::Lipschitz(1.732_051), None).unwrap();
//! let mut row = [[0.0; 4]];
//! field.eval_gradients(&[[0.0; 3]], &mut row);
//! assert_eq!(row[0], [0.0, 1.0, 1.0, 1.0]);
//! ```
//!
//! ```
//! use exedra_isosurface::function::FnField;
//! use exedra_isosurface::{Aabb, Metric, ScalarField};
//!
//! // A sphere with a rippled skin: the ripple's slope is at most
//! // 0.05 * 12, so values change at most 1.6 times as fast as position.
//! let ripples = FnField::new(
//!     |[x, y, z]: [f64; 3]| {
//!         (x * x + y * y + z * z).sqrt() - 1.0 + 0.05 * (12.0 * x).sin()
//!     },
//!     Metric::Lipschitz(1.6),
//!     Some(Aabb::new([-1.1; 3], [1.1; 3]).unwrap()),
//! )
//! .unwrap();
//! let mut value = [0.0];
//! ripples.eval_points(&[[0.0; 3]], &mut value);
//! assert!(value[0] < 0.0);
//! ```

use exedra_math::Real;

use crate::capability::{Capabilities, FieldCapabilities, Metric, Support};
use crate::number::{Dual, Interval, Number};
use crate::spatial::Aabb;
use crate::{FieldError, ScalarField};

/// A distance function written once over any [`Number`].
///
/// See [`crate::number`] for what a formula may use: arithmetic,
/// `min`, `max`, `abs`, `sqrt`, `sin` and `cos`, and no branches on values.
pub trait Formula {
    /// The function's value at `p`, negative inside.
    fn eval<N: Number>(&self, p: [N; 3]) -> N;
}

/// A field evaluating a caller's [`Formula`] in `f64`, with exact gradients
/// and interval bounds from the formula itself.
#[derive(Copy, Clone, Debug)]
pub struct FormulaField<F> {
    formula: F,
    metric: Metric,
    support: Option<Aabb>,
}

impl<F: Formula> FormulaField<F> {
    /// Creates a field from `formula`, negative inside, with the caller's
    /// `metric` claim and, if known, a box containing the interior.
    ///
    /// The claims are trusted as for [`FnField::new`].
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a Lipschitz factor is not finite and
    /// positive.
    pub fn new(formula: F, metric: Metric, support: Option<Aabb>) -> Result<Self, FieldError> {
        if let Metric::Lipschitz(factor) = metric {
            crate::check_positive("lipschitz", factor)?;
        }
        Ok(Self {
            formula,
            metric,
            support,
        })
    }

    /// The formula this field evaluates.
    pub const fn formula(&self) -> &F {
        &self.formula
    }

    /// Interval bounds of the formula over `bounds`, natural and mean-value
    /// forms intersected, and of its gradient.
    fn bounds(&self, bounds: &Aabb) -> (Interval, [Interval; 3]) {
        let Dual { value, gradient } = self.formula.eval(Dual::point(boxed(bounds)));
        let center = widen(bounds.center());
        let at_center = self.formula.eval(center.map(Interval::constant));
        let mean_value = (0..3).fold(at_center, |sum, axis| {
            let reach = f64::from(bounds.max[axis]) - center[axis];
            let lag = center[axis] - f64::from(bounds.min[axis]);
            sum + gradient[axis] * Interval::new(-lag, reach)
        });
        (value.intersect(mean_value).unwrap_or(value), gradient)
    }
}

impl<F: Formula> ScalarField for FormulaField<F> {
    fn capabilities(&self) -> FieldCapabilities {
        Capabilities {
            metric: self.metric,
            support: self.support.map_or(Support::Unknown, Support::Asserted),
        }
    }

    /// The tightest of three sound bounds: the formula evaluated on
    /// intervals, the mean-value form `F(c) + ∇F(box)·(box - c)` about the
    /// center `c`, and the Lipschitz claim.
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        self.value_bounds(bounds, self.bounds(bounds).0)
    }

    fn eval_gradient_interval(&self, bounds: &Aabb) -> Option<[[f32; 2]; 4]> {
        let (value, gradient) = self.bounds(bounds);
        let rows = [
            self.value_bounds(bounds, value)?,
            outward(gradient[0]),
            outward(gradient[1]),
            outward(gradient[2]),
        ];
        rows.iter().flatten().all(|v| !v.is_nan()).then_some(rows)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        eval_points(|p| self.formula.eval(p), [0.0; 3], points, out);
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_eq!(
            points.len(),
            out.len(),
            "point/gradient slice lengths must match"
        );
        for (point, row) in points.iter().zip(out) {
            let Dual { value, gradient } = self.formula.eval(Dual::point(widen(*point)));
            *row = [
                narrow(value),
                narrow(gradient[0]),
                narrow(gradient[1]),
                narrow(gradient[2]),
            ];
        }
    }
}

impl<F: Formula> FormulaField<F> {
    /// `interval`, from the formula, narrowed by the Lipschitz claim and
    /// rounded outward to `f32`.
    fn value_bounds(&self, bounds: &Aabb, interval: Interval) -> Option<[f32; 2]> {
        let lipschitz = self
            .metric
            .lipschitz()
            .and_then(|lipschitz| {
                lipschitz_interval(|p| self.formula.eval(p), [0.0; 3], lipschitz, bounds)
            })
            .map(|[low, high]| Interval::new(f64::from(low), f64::from(high)));
        let interval =
            lipschitz.map_or(Some(interval), |lipschitz| interval.intersect(lipschitz))?;
        let interval = [round_down(interval.low()), round_up(interval.high())];
        interval.iter().all(|v| !v.is_nan()).then_some(interval)
    }
}

/// A field evaluating a caller's distance function in `f64`, with gradients
/// by central differences.
#[derive(Copy, Clone, Debug)]
pub struct FnField<D> {
    distance: D,
    metric: Metric,
    support: Option<Aabb>,
}

impl<D: Fn([f64; 3]) -> f64> FnField<D> {
    /// Creates a field from `distance`, negative inside, with the caller's
    /// `metric` claim and, if known, a box containing the interior.
    ///
    /// Nothing checks the claims. A Lipschitz factor that is too small makes
    /// interval culling skip surface; a box that is too small makes
    /// extraction sized from it miss surface. [`Metric::Unknown`] is always
    /// truthful, but leaves intervals unbounded, so extraction cannot cull.
    ///
    /// # Errors
    ///
    /// Returns [`FieldError`] when a Lipschitz factor is not finite and
    /// positive.
    pub fn new(distance: D, metric: Metric, support: Option<Aabb>) -> Result<Self, FieldError> {
        if let Metric::Lipschitz(factor) = metric {
            crate::check_positive("lipschitz", factor)?;
        }
        Ok(Self {
            distance,
            metric,
            support,
        })
    }
}

impl<D: Fn([f64; 3]) -> f64> ScalarField for FnField<D> {
    fn capabilities(&self) -> FieldCapabilities {
        Capabilities {
            metric: self.metric,
            support: self.support.map_or(Support::Unknown, Support::Asserted),
        }
    }

    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let lipschitz = self.metric.lipschitz()?;
        lipschitz_interval(&self.distance, [0.0; 3], lipschitz, bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        eval_points(&self.distance, [0.0; 3], points, out);
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        eval_gradients(&self.distance, [0.0; 3], points, out);
    }
}

/// Bounds over `bounds` of a function changing at most `lipschitz` per unit,
/// evaluated relative to `origin`.
pub(crate) fn lipschitz_interval(
    distance: impl Fn([f64; 3]) -> f64,
    origin: [f32; 3],
    lipschitz: f32,
    bounds: &Aabb,
) -> Option<[f32; 2]> {
    let center = local(origin, widen(bounds.center()));
    let half = 0.5
        * length(core::array::from_fn(|i| {
            f64::from(bounds.max[i]) - f64::from(bounds.min[i])
        }));
    let value = distance(center);
    let reach = f64::from(lipschitz) * half;
    // The function rounds by a few ulps of its inputs and result.
    let margin = 64.0 * f64::EPSILON * (1.0 + length(center) + value.abs());
    let interval = [
        round_down(value - reach - margin),
        round_up(value + reach + margin),
    ];
    interval.iter().all(|v| !v.is_nan()).then_some(interval)
}

pub(crate) fn eval_points(
    distance: impl Fn([f64; 3]) -> f64,
    origin: [f32; 3],
    points: &[[f32; 3]],
    out: &mut [f32],
) {
    assert_eq!(
        points.len(),
        out.len(),
        "point/value slice lengths must match"
    );
    for (point, out) in points.iter().zip(out) {
        *out = narrow(distance(local(origin, widen(*point))));
    }
}

pub(crate) fn eval_gradients(
    distance: impl Fn([f64; 3]) -> f64,
    origin: [f32; 3],
    points: &[[f32; 3]],
    out: &mut [[f32; 4]],
) {
    assert_eq!(
        points.len(),
        out.len(),
        "point/gradient slice lengths must match"
    );
    for (point, row) in points.iter().zip(out) {
        let p = local(origin, widen(*point));
        let gradient = differences(&distance, p);
        *row = [
            narrow(distance(p)),
            narrow(gradient[0]),
            narrow(gradient[1]),
            narrow(gradient[2]),
        ];
    }
}

/// Central differences of `distance` at `p`, with a step small against the
/// coordinates but well above `f64` rounding.
fn differences(distance: impl Fn([f64; 3]) -> f64, p: [f64; 3]) -> [f64; 3] {
    let step = 1.0e-7 * (1.0 + length(p));
    core::array::from_fn(|axis| {
        let mut ahead = p;
        let mut behind = p;
        ahead[axis] += step;
        behind[axis] -= step;
        (distance(ahead) - distance(behind)) / (2.0 * step)
    })
}

/// The coordinates of `bounds` as intervals.
fn boxed(bounds: &Aabb) -> [Interval; 3] {
    core::array::from_fn(|axis| {
        Interval::new(f64::from(bounds.min[axis]), f64::from(bounds.max[axis]))
    })
}

/// `interval` narrowed to `f32`, rounded outward.
fn outward(interval: Interval) -> [f32; 2] {
    [round_down(interval.low()), round_up(interval.high())]
}

fn local(origin: [f32; 3], p: [f64; 3]) -> [f64; 3] {
    core::array::from_fn(|i| p[i] - f64::from(origin[i]))
}

fn widen(point: [f32; 3]) -> [f64; 3] {
    point.map(f64::from)
}

fn length(v: [f64; 3]) -> f64 {
    Real::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "fields compute in f64 and return f32 by design"
)]
fn narrow(value: f64) -> f32 {
    value as f32
}

fn round_down(value: f64) -> f32 {
    let rounded = narrow(value);
    if f64::from(rounded) > value {
        rounded.next_down()
    } else {
        rounded
    }
}

fn round_up(value: f64) -> f32 {
    let rounded = narrow(value);
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}
