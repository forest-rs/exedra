// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tiny analytic reference fields used to exercise the evaluation seam.

use exedra_spatial::Aabb;

use crate::ScalarField;

/// Analytic sphere signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SphereField {
    /// Sphere center.
    pub center: [f32; 3],
    /// Sphere radius.
    pub radius: f32,
}

impl ScalarField for SphereField {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let nearest = [
            clamp(self.center[0], bounds.min[0], bounds.max[0]),
            clamp(self.center[1], bounds.min[1], bounds.max[1]),
            clamp(self.center[2], bounds.min[2], bounds.max[2]),
        ];
        let furthest = [
            farthest_axis(self.center[0], bounds.min[0], bounds.max[0]),
            farthest_axis(self.center[1], bounds.min[1], bounds.max[1]),
            farthest_axis(self.center[2], bounds.min[2], bounds.max[2]),
        ];
        Some([
            length(sub(nearest, self.center)) - self.radius,
            length(sub(furthest, self.center)) - self.radius,
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_eq!(
            points.len(),
            out.len(),
            "point/value slice lengths must match"
        );
        for (index, point) in points.iter().enumerate() {
            out[index] = length(sub(*point, self.center)) - self.radius;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_eq!(
            points.len(),
            out.len(),
            "point/gradient slice lengths must match"
        );
        for (index, point) in points.iter().enumerate() {
            let delta = sub(*point, self.center);
            let dist = length(delta);
            out[index][0] = dist - self.radius;
            if dist == 0.0 {
                out[index][1] = f32::NAN;
                out[index][2] = f32::NAN;
                out[index][3] = f32::NAN;
            } else {
                out[index][1] = delta[0] / dist;
                out[index][2] = delta[1] / dist;
                out[index][3] = delta[2] / dist;
            }
        }
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    value.max(min).min(max)
}

fn farthest_axis(center: f32, min: f32, max: f32) -> f32 {
    if (center - min).abs() >= (max - center).abs() {
        min
    } else {
        max
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(vector: [f32; 3]) -> f32 {
    sqrt(vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2])
}

#[cfg(feature = "std")]
fn sqrt(value: f32) -> f32 {
    value.sqrt()
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}
