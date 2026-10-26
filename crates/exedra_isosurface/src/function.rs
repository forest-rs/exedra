// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fields from caller functions.
//!
//! [`FnField`] turns a distance function into a [`ScalarField`] without
//! writing interval or gradient code. Its Lipschitz claim, which the caller
//! vouches for, does the rest: no value in a box differs from the value at
//! its center by more than the factor times the half diagonal, which bounds
//! every interval, and gradients are central differences of the function.
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
use crate::spatial::Aabb;
use crate::{FieldError, ScalarField};

/// A field evaluating a caller's distance function in `f64`.
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
