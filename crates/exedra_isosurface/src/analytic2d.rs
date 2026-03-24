// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Analytic 2D profile fields used by lifting operators and tests.

use crate::{Aabb2, ScalarField2d};

/// Analytic circle signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CircleField2d {
    /// Circle center.
    pub center: [f32; 2],
    /// Circle radius.
    pub radius: f32,
}

/// Axis-aligned rectangle signed-distance field.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RectField2d {
    /// Rectangle center.
    pub center: [f32; 2],
    /// Half extents along x and y.
    pub half_extents: [f32; 2],
}

/// Plane half-space in 2D.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HalfPlaneField2d {
    /// One point on the boundary line.
    pub point: [f32; 2],
    /// Outward normal.
    pub normal: [f32; 2],
}

impl ScalarField2d for CircleField2d {
    fn eval_interval(&self, bounds: &Aabb2) -> Option<[f32; 2]> {
        let nearest = [
            clamp(self.center[0], bounds.min[0], bounds.max[0]),
            clamp(self.center[1], bounds.min[1], bounds.max[1]),
        ];
        let furthest = [
            farthest_axis(self.center[0], bounds.min[0], bounds.max[0]),
            farthest_axis(self.center[1], bounds.min[1], bounds.max[1]),
        ];
        Some([
            length2(sub2(nearest, self.center)) - self.radius,
            length2(sub2(furthest, self.center)) - self.radius,
        ])
    }

    fn eval_points(&self, points: &[[f32; 2]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            out[index] = length2(sub2(*point, self.center)) - self.radius;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 2]], out: &mut [[f32; 3]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let delta = sub2(*point, self.center);
            let distance = length2(delta);
            out[index][0] = distance - self.radius;
            out[index][1..3].copy_from_slice(&normalize2_or_nan(delta));
        }
    }
}

impl ScalarField2d for RectField2d {
    fn eval_interval(&self, bounds: &Aabb2) -> Option<[f32; 2]> {
        Some(conservative_sampled_interval(self, bounds))
    }

    fn eval_points(&self, points: &[[f32; 2]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        for (index, point) in points.iter().enumerate() {
            let delta = abs2(sub2(*point, self.center));
            let q = sub2(delta, self.half_extents);
            let outside = max2(q, [0.0; 2]);
            let outside_distance = length2(outside);
            let inside_distance = q[0].max(q[1]).min(0.0);
            out[index] = outside_distance + inside_distance;
        }
    }

    fn eval_gradients(&self, points: &[[f32; 2]], out: &mut [[f32; 3]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        for (index, point) in points.iter().enumerate() {
            let delta = sub2(*point, self.center);
            let abs_delta = abs2(delta);
            let q = sub2(abs_delta, self.half_extents);
            let outside = max2(q, [0.0; 2]);
            let outside_distance = length2(outside);
            out[index][0] = outside_distance + q[0].max(q[1]).min(0.0);
            out[index][1..3].copy_from_slice(&rect_gradient(delta, q, outside, outside_distance));
        }
    }
}

impl ScalarField2d for HalfPlaneField2d {
    fn eval_interval(&self, bounds: &Aabb2) -> Option<[f32; 2]> {
        let normal = normalize2(self.normal)?;
        let mut minimum = dot2(normal, sub2(bounds.min, self.point));
        let mut maximum = minimum;
        for corner in bounds.corners() {
            let value = dot2(normal, sub2(corner, self.point));
            minimum = minimum.min(value);
            maximum = maximum.max(value);
        }
        Some([minimum, maximum])
    }

    fn eval_points(&self, points: &[[f32; 2]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let gradient = normalize2_or_nan(self.normal);
        for (index, point) in points.iter().enumerate() {
            out[index] = dot2(gradient, sub2(*point, self.point));
        }
    }

    fn eval_gradients(&self, points: &[[f32; 2]], out: &mut [[f32; 3]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let gradient = normalize2_or_nan(self.normal);
        for (index, point) in points.iter().enumerate() {
            out[index][0] = dot2(gradient, sub2(*point, self.point));
            out[index][1..3].copy_from_slice(&gradient);
        }
    }
}

fn conservative_sampled_interval<F: ScalarField2d>(field: &F, bounds: &Aabb2) -> [f32; 2] {
    let samples = sample_points(bounds);
    let mut values = [0.0_f32; 5];
    field.eval_points(&samples, &mut values);

    let inflation = length2(sub2(bounds.max, bounds.min));
    let mut minimum = values[0];
    let mut maximum = values[0];
    for value in values {
        minimum = minimum.min(value);
        maximum = maximum.max(value);
    }
    [minimum - inflation, maximum + inflation]
}

fn sample_points(bounds: &Aabb2) -> [[f32; 2]; 5] {
    let corners = bounds.corners();
    [
        corners[0],
        corners[1],
        corners[2],
        corners[3],
        bounds.center(),
    ]
}

fn rect_gradient(
    delta: [f32; 2],
    q: [f32; 2],
    outside: [f32; 2],
    outside_distance: f32,
) -> [f32; 2] {
    if outside_distance > 0.0 {
        return [
            signed_component(delta[0], outside[0], outside_distance),
            signed_component(delta[1], outside[1], outside_distance),
        ];
    }

    let max_q = q[0].max(q[1]);
    let mut axis = None;
    for (index, component) in q.into_iter().enumerate() {
        if (component - max_q).abs() <= 1.0e-6 {
            if axis.is_some() {
                return [f32::NAN; 2];
            }
            axis = Some(index);
        }
    }
    let Some(axis) = axis else {
        return [f32::NAN; 2];
    };
    let Some(sign) = sign_nonzero(delta[axis]) else {
        return [f32::NAN; 2];
    };
    let mut gradient = [0.0; 2];
    gradient[axis] = sign;
    gradient
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
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

fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn sub2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn abs(value: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        value.abs()
    }
    #[cfg(all(not(feature = "std"), feature = "libm"))]
    {
        libm::fabsf(value)
    }
}

fn abs2(vector: [f32; 2]) -> [f32; 2] {
    [abs(vector[0]), abs(vector[1])]
}

fn max2(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0].max(b[0]), a[1].max(b[1])]
}

fn signed_component(delta: f32, numerator: f32, denominator: f32) -> f32 {
    if numerator == 0.0 {
        0.0
    } else {
        sign_nonzero(delta).unwrap_or(f32::NAN) * numerator / denominator
    }
}

fn sign_nonzero(value: f32) -> Option<f32> {
    if value > 0.0 {
        Some(1.0)
    } else if value < 0.0 {
        Some(-1.0)
    } else {
        None
    }
}

fn normalize2(vector: [f32; 2]) -> Option<[f32; 2]> {
    if !vector.iter().all(|component| component.is_finite()) {
        return None;
    }
    let vector_length = length2(vector);
    if vector_length == 0.0 {
        return None;
    }
    Some([vector[0] / vector_length, vector[1] / vector_length])
}

fn normalize2_or_nan(vector: [f32; 2]) -> [f32; 2] {
    normalize2(vector).unwrap_or([f32::NAN; 2])
}

fn length2(vector: [f32; 2]) -> f32 {
    sqrt(vector[0] * vector[0] + vector[1] * vector[1])
}

#[cfg(feature = "std")]
fn sqrt(value: f32) -> f32 {
    value.sqrt()
}

#[cfg(all(not(feature = "std"), feature = "libm"))]
fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}

#[cfg(test)]
mod tests {
    use crate::{Aabb2, ScalarField2d};

    use super::{CircleField2d, HalfPlaneField2d, RectField2d};

    fn finite_difference_gradient<F: ScalarField2d>(field: &F, point: [f32; 2]) -> [f32; 2] {
        let epsilon = 1.0e-3;
        let mut output = [0.0_f32; 2];
        let mut gradient = [0.0; 2];
        for axis in 0..2 {
            let mut plus = point;
            let mut minus = point;
            plus[axis] += epsilon;
            minus[axis] -= epsilon;
            field.eval_points(&[plus, minus], &mut output);
            gradient[axis] = (output[0] - output[1]) / (2.0 * epsilon);
        }
        gradient
    }

    fn assert_gradient_matches_finite_difference<F: ScalarField2d>(field: &F, point: [f32; 2]) {
        let expected = finite_difference_gradient(field, point);
        let mut out = [[0.0_f32; 3]; 1];
        field.eval_gradients(&[point], &mut out);
        for axis in 0..2 {
            assert!(
                (out[0][axis + 1] - expected[axis]).abs() <= 2.0e-2,
                "axis {axis}: analytic={}, finite={}",
                out[0][axis + 1],
                expected[axis]
            );
        }
    }

    fn assert_interval_covers_bruteforce<F: ScalarField2d>(field: &F, bounds: &Aabb2) {
        let interval = field.eval_interval(bounds).expect("interval should exist");
        let mut values = [0.0_f32; 1];
        for x in 0..=4 {
            for y in 0..=4 {
                let point = [
                    bounds.min[0] + (bounds.max[0] - bounds.min[0]) * (x as f32 / 4.0),
                    bounds.min[1] + (bounds.max[1] - bounds.min[1]) * (y as f32 / 4.0),
                ];
                field.eval_points(&[point], &mut values);
                assert!(values[0] >= interval[0] - 1.0e-5);
                assert!(values[0] <= interval[1] + 1.0e-5);
            }
        }
    }

    #[test]
    fn primitive_gradients_match_finite_difference() {
        let circle = CircleField2d {
            center: [0.0, 0.0],
            radius: 1.0,
        };
        let rect = RectField2d {
            center: [0.0, 0.0],
            half_extents: [0.75, 0.5],
        };

        assert_gradient_matches_finite_difference(&circle, [0.8, 0.3]);
        assert_gradient_matches_finite_difference(&rect, [0.95, 0.1]);
    }

    #[test]
    fn rect_interval_is_conservative() {
        let field = RectField2d {
            center: [0.1, -0.2],
            half_extents: [0.75, 0.5],
        };
        let bounds = Aabb2::new([-1.0, -1.0], [1.0, 0.75]).expect("valid bounds");

        assert_interval_covers_bruteforce(&field, &bounds);
    }

    #[test]
    fn circle_interval_is_conservative() {
        let field = CircleField2d {
            center: [0.2, -0.1],
            radius: 0.75,
        };
        let bounds = Aabb2::new([-1.0, -1.0], [1.2, 1.0]).expect("valid bounds");

        assert_interval_covers_bruteforce(&field, &bounds);
    }

    #[test]
    fn half_plane_gradient_is_constant() {
        let field = HalfPlaneField2d {
            point: [0.0, 0.5],
            normal: [0.0, 2.0],
        };
        let mut out = [[0.0_f32; 3]; 1];
        field.eval_gradients(&[[0.0, 1.0]], &mut out);

        assert!((out[0][0] - 0.5).abs() <= 1.0e-6);
        assert_eq!(out[0][1], 0.0);
        assert_eq!(out[0][2], 1.0);
    }
}
