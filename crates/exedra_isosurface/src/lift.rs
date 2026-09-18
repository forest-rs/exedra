// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Lift 2D profile fields into 3D scalar fields.

use exedra_spatial::Aabb;

use crate::{Aabb2, ScalarField, ScalarField2d};
use exedra_math::{add, scale};

/// Finite extrusion of a 2D profile field along the world-space z axis.
///
/// Interval evaluation composes the profile's bounds with the axial bounds.
/// A profile without a known interval remains uncullable.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Extrude<F> {
    profile: F,
    half_height: f32,
}

/// Revolution of a 2D radius-height profile around the world-space y axis.
///
/// Interval evaluation maps the box to radius-height bounds and queries the
/// profile. A profile without a known interval remains uncullable.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Revolve<F> {
    profile: F,
}

impl<F> Extrude<F> {
    /// Creates a finite extrusion.
    ///
    /// `half_height` must be finite and strictly positive.
    #[must_use]
    pub fn new(profile: F, half_height: f32) -> Option<Self> {
        if half_height.is_finite() && half_height > 0.0 {
            Some(Self {
                profile,
                half_height,
            })
        } else {
            None
        }
    }

    /// Returns the source profile field.
    #[must_use]
    pub const fn profile(&self) -> &F {
        &self.profile
    }

    /// Returns the half height of the extrusion.
    #[must_use]
    pub const fn half_height(&self) -> f32 {
        self.half_height
    }
}

impl<F> Revolve<F> {
    /// Creates a revolution field around the world-space y axis.
    #[must_use]
    pub const fn new(profile: F) -> Self {
        Self { profile }
    }

    /// Returns the source profile field.
    #[must_use]
    pub const fn profile(&self) -> &F {
        &self.profile
    }
}

impl<F: ScalarField2d> ScalarField for Extrude<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let profile_bounds = Aabb2::new(
            [bounds.min[0], bounds.min[1]],
            [bounds.max[0], bounds.max[1]],
        )?;
        let [lower, upper] = self.profile.eval_interval(&profile_bounds)?;
        let [near, far] = absolute_interval(bounds.min[2], bounds.max[2]);
        // The cap composition is monotone in both profile value and |z|.
        Some([
            capped_distance(lower, near, self.half_height),
            capped_distance(upper, far, self.half_height),
        ])
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut profile_value = [0.0_f32; 1];

        for (index, point) in points.iter().enumerate() {
            self.profile
                .eval_points(&[[point[0], point[1]]], &mut profile_value);
            out[index] = capped_distance(profile_value[0], point[2], self.half_height);
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut profile_gradient = [[0.0_f32; 3]; 1];

        for (index, point) in points.iter().enumerate() {
            self.profile
                .eval_gradients(&[[point[0], point[1]]], &mut profile_gradient);
            let profile_distance = profile_gradient[0][0];
            let axial_distance = abs(point[2]) - self.half_height;
            let outside = [profile_distance.max(0.0), axial_distance.max(0.0)];
            let outside_distance = length2(outside);
            out[index][0] = capped_distance(profile_distance, point[2], self.half_height);

            let profile_normal = [profile_gradient[0][1], profile_gradient[0][2], 0.0];
            let axial_normal = [0.0, 0.0, sign_nonzero(point[2]).unwrap_or(f32::NAN)];
            let gradient = if outside_distance > 0.0 {
                add(
                    scale(profile_normal, outside[0] / outside_distance),
                    scale(axial_normal, outside[1] / outside_distance),
                )
            } else if (profile_distance - axial_distance).abs() <= 1.0e-6 {
                [f32::NAN; 3]
            } else if profile_distance > axial_distance {
                profile_normal
            } else {
                axial_normal
            };
            out[index][1..4].copy_from_slice(&gradient);
        }
    }
}

impl<F: ScalarField2d> ScalarField for Revolve<F> {
    fn eval_interval(&self, bounds: &Aabb) -> Option<[f32; 2]> {
        let x = absolute_interval(bounds.min[0], bounds.max[0]);
        let z = absolute_interval(bounds.min[2], bounds.max[2]);
        let profile_bounds = Aabb2::new(
            [radial_distance([x[0], 0.0, z[0]]), bounds.min[1]],
            [radial_distance([x[1], 0.0, z[1]]), bounds.max[1]],
        )?;
        self.profile.eval_interval(&profile_bounds)
    }

    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        assert_same_len(points.len(), out.len(), "point/value");
        let mut profile_value = [0.0_f32; 1];

        for (index, point) in points.iter().enumerate() {
            self.profile
                .eval_points(&[[radial_distance(*point), point[1]]], &mut profile_value);
            out[index] = profile_value[0];
        }
    }

    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        assert_same_len(points.len(), out.len(), "point/gradient");
        let mut profile_gradient = [[0.0_f32; 3]; 1];

        for (index, point) in points.iter().enumerate() {
            let radial = radial_distance(*point);
            self.profile
                .eval_gradients(&[[radial, point[1]]], &mut profile_gradient);
            out[index][0] = profile_gradient[0][0];
            out[index][1..4].copy_from_slice(&revolved_gradient(
                [profile_gradient[0][1], profile_gradient[0][2]],
                *point,
                radial,
            ));
        }
    }
}

fn capped_distance(profile_distance: f32, z: f32, half_height: f32) -> f32 {
    let q = [profile_distance, abs(z) - half_height];
    let outside = [q[0].max(0.0), q[1].max(0.0)];
    length2(outside) + q[0].max(q[1]).min(0.0)
}

fn revolved_gradient(profile_gradient: [f32; 2], point: [f32; 3], radial: f32) -> [f32; 3] {
    if radial == 0.0 {
        if abs(profile_gradient[0]) <= 1.0e-6 {
            return [0.0, profile_gradient[1], 0.0];
        }
        return [f32::NAN, profile_gradient[1], f32::NAN];
    }

    [
        profile_gradient[0] * point[0] / radial,
        profile_gradient[1],
        profile_gradient[0] * point[2] / radial,
    ]
}

fn absolute_interval(minimum: f32, maximum: f32) -> [f32; 2] {
    let near = if minimum <= 0.0 && maximum >= 0.0 {
        0.0
    } else {
        abs(minimum).min(abs(maximum))
    };
    [near, abs(minimum).max(abs(maximum))]
}

fn radial_distance(point: [f32; 3]) -> f32 {
    sqrt(point[0] * point[0] + point[2] * point[2])
}

fn assert_same_len(expected: usize, found: usize, label: &str) {
    assert_eq!(expected, found, "{label} slice lengths must match");
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
    use alloc::vec;

    use exedra_spatial::Aabb;

    use crate::{
        Aabb2, EdgeSearchParams, ScalarField, ScalarField2d,
        analytic::{CylinderField, TorusField},
        analytic2d::CircleField2d,
        dual_contour::{DualContourParams, dual_contour},
    };
    use exedra_qef::QefParams;

    use super::{Extrude, Revolve};

    fn params(bounds: Aabb, max_depth: u8) -> DualContourParams {
        DualContourParams {
            root_bounds: bounds,
            max_depth,
            cell_budget: None,
            vertex_merge_tolerance: 0.0,
            edge_search: EdgeSearchParams {
                bisection_steps: 10,
            },
            qef: QefParams::default(),
        }
    }

    fn assert_fields_match<A: ScalarField, B: ScalarField>(
        left: &A,
        right: &B,
        points: &[[f32; 3]],
    ) {
        let mut left_values = vec![0.0; points.len()];
        let mut right_values = vec![0.0; points.len()];
        let mut left_gradients = vec![[0.0; 4]; points.len()];
        let mut right_gradients = vec![[0.0; 4]; points.len()];

        left.eval_points(points, &mut left_values);
        right.eval_points(points, &mut right_values);
        left.eval_gradients(points, &mut left_gradients);
        right.eval_gradients(points, &mut right_gradients);

        for index in 0..points.len() {
            assert!(
                (left_values[index] - right_values[index]).abs() <= 1.0e-5,
                "value mismatch at {:?}: {} vs {}",
                points[index],
                left_values[index],
                right_values[index]
            );
            for axis in 0..4 {
                assert!(
                    (left_gradients[index][axis] - right_gradients[index][axis]).abs() <= 1.0e-5,
                    "gradient mismatch at {:?} axis {}: {} vs {}",
                    points[index],
                    axis,
                    left_gradients[index][axis],
                    right_gradients[index][axis]
                );
            }
        }
    }

    #[test]
    fn extruded_circle_matches_cylinder_reference() {
        let field = Extrude::new(
            CircleField2d {
                center: [0.0, 0.0],
                radius: 0.75,
            },
            0.8,
        )
        .expect("valid extrusion");
        let reference = CylinderField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, 1.0],
            radius: 0.75,
            half_height: 0.8,
        };
        let points = [
            [0.75, 0.0, 0.0],
            [0.2, 0.6, 0.3],
            [0.0, 0.0, 0.8],
            [0.9, 0.0, 1.0],
        ];

        assert_fields_match(&field, &reference, &points);
    }

    #[test]
    fn revolved_circle_matches_torus_reference() {
        let field = Revolve::new(CircleField2d {
            center: [1.0, 0.0],
            radius: 0.25,
        });
        let reference = TorusField {
            center: [0.0, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            major_radius: 1.0,
            minor_radius: 0.25,
        };
        let points = [
            [1.25, 0.0, 0.0],
            [0.0, 0.0, 1.25],
            [1.0, 0.25, 0.0],
            [0.7, -0.1, 0.7],
        ];

        assert_fields_match(&field, &reference, &points);
    }

    #[test]
    fn dual_contour_extruded_circle_builds_valid_mesh() {
        let field = Extrude::new(
            CircleField2d {
                center: [0.0, 0.0],
                radius: 0.7,
            },
            0.8,
        )
        .expect("valid extrusion");
        let bounds = Aabb::new([-1.2, -1.2, -1.2], [1.2, 1.2, 1.2]).expect("bounds");
        let result =
            dual_contour(&field, &params(bounds, 4)).expect("extruded profile should extract");

        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.stats.faces > 0);
        assert!(result.stats.vertices > 0);
    }

    #[test]
    fn extrusion_rejects_non_positive_heights() {
        assert!(
            Extrude::new(
                CircleField2d {
                    center: [0.0, 0.0],
                    radius: 1.0,
                },
                0.0
            )
            .is_none()
        );
    }

    #[test]
    fn revolve_interval_is_conservative_for_sample_points() {
        let field = Revolve::new(CircleField2d {
            center: [1.0, 0.0],
            radius: 0.25,
        });
        let bounds = Aabb::new([-1.5, -0.5, -1.5], [1.5, 0.5, 1.5]).expect("bounds");
        let interval = field.eval_interval(&bounds).expect("interval");
        let points = [
            bounds.min,
            [bounds.max[0], bounds.min[1], bounds.min[2]],
            [bounds.min[0], bounds.max[1], bounds.min[2]],
            [bounds.max[0], bounds.max[1], bounds.max[2]],
            bounds.center(),
        ];
        let mut values = [0.0_f32; 5];
        field.eval_points(&points, &mut values);

        for value in values {
            assert!(value >= interval[0] - 1.0e-5);
            assert!(value <= interval[1] + 1.0e-5);
        }
    }

    #[test]
    fn extrusion_profile_accessors_round_trip() {
        let profile = CircleField2d {
            center: [0.0, 0.0],
            radius: 0.5,
        };
        let field = Extrude::new(profile, 0.75).expect("valid extrusion");

        assert_eq!(field.profile(), &profile);
        assert_eq!(field.half_height(), 0.75);
    }

    struct ScaledCircle {
        scale: f32,
        interval_known: bool,
    }

    impl ScalarField2d for ScaledCircle {
        fn eval_interval(&self, bounds: &Aabb2) -> Option<[f32; 2]> {
            self.interval_known.then(|| {
                self.circle()
                    .eval_interval(bounds)
                    .unwrap()
                    .map(|v| self.scale * v)
            })
        }

        fn eval_points(&self, points: &[[f32; 2]], out: &mut [f32]) {
            self.circle().eval_points(points, out);
            for value in out {
                *value *= self.scale;
            }
        }

        fn eval_gradients(&self, points: &[[f32; 2]], out: &mut [[f32; 3]]) {
            self.circle().eval_gradients(points, out);
            for value in out {
                *value = value.map(|v| self.scale * v);
            }
        }
    }

    impl ScaledCircle {
        fn circle(&self) -> CircleField2d {
            CircleField2d {
                center: [0.015, 0.0],
                radius: 0.005,
            }
        }
    }

    #[test]
    fn lifted_intervals_preserve_unknown_profile_bounds() {
        let bounds = Aabb::new([-0.032; 3], [0.032; 3]).unwrap();
        let profile = || ScaledCircle {
            scale: 1000.0,
            interval_known: false,
        };
        assert_eq!(Revolve::new(profile()).eval_interval(&bounds), None);
        assert_eq!(
            Extrude::new(profile(), 0.01)
                .unwrap()
                .eval_interval(&bounds),
            None
        );
    }

    #[test]
    fn scaled_revolution_is_not_culled_as_empty() {
        let field = Revolve::new(ScaledCircle {
            scale: 1000.0,
            interval_known: true,
        });
        let bounds = Aabb::new([-0.0317, -0.0313, -0.0319], [0.0323, 0.0327, 0.0321]).unwrap();
        let interval = field.eval_interval(&bounds).unwrap();
        assert!(interval[0] < 0.0 && interval[1] > 0.0);
        let result = dual_contour(&field, &params(bounds, 5)).unwrap();
        assert!(result.stats.faces > 0);
        assert!(result.mesh.validate_deep().is_empty());
        assert!(result.mesh.boundary_loops().unwrap().is_empty());
    }

    #[test]
    fn lifted_intervals_enclose_scaled_fields_across_and_away_from_the_axis() {
        for scale in [0.01, 1.0, 1000.0] {
            let profile = || ScaledCircle {
                scale,
                interval_known: true,
            };
            let revolved = Revolve::new(profile());
            let extruded = Extrude::new(profile(), 0.01).unwrap();
            for (min, max) in [
                ([-0.032; 3], [0.032; 3]),
                ([0.011, -0.003, -0.006], [0.020, 0.002, 0.013]),
                ([-0.020, -0.012, -0.010], [-0.011, -0.004, -0.002]),
                ([0.04, 0.03, 0.06], [0.08, 0.07, 0.09]),
            ] {
                let bounds = Aabb::new(min, max).unwrap();
                for field in [&revolved as &dyn ScalarField, &extruded as &dyn ScalarField] {
                    let interval = field.eval_interval(&bounds).unwrap();
                    for x in 0_u8..=8 {
                        for y in 0_u8..=8 {
                            for z in 0_u8..=8 {
                                let steps = [x, y, z];
                                let p = core::array::from_fn(|i| {
                                    (min[i] + (max[i] - min[i]) * f32::from(steps[i]) / 8.0)
                                        .clamp(min[i], max[i])
                                });
                                let mut value = [0.0];
                                field.eval_points(&[p], &mut value);
                                assert!(
                                    interval[0] <= value[0] && value[0] <= interval[1],
                                    "scale={scale}, point={p:?}, value={value:?}, interval={interval:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
