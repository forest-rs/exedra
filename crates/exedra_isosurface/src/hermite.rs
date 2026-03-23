// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hermite data types and edge intersection search.

extern crate alloc;

use alloc::vec::Vec;

use crate::ScalarField;

/// Hermite data at one edge-surface intersection.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HermiteIntersection {
    /// Position of the zero-crossing on the sampled edge.
    pub position: [f32; 3],
    /// Surface normal at `position`.
    ///
    /// NaN components are permitted for non-differentiable points.
    pub normal: [f32; 3],
    /// Parametric distance along the edge (`0.0` = start, `1.0` = end).
    pub t: f32,
}

/// One per-cell edge intersection tagged by cube-edge index.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CellHermiteIntersection {
    /// Cube-edge index (`0..12` for a cube cell).
    pub edge_index: u8,
    /// Hermite intersection data for `edge_index`.
    pub intersection: HermiteIntersection,
}

/// Hermite data collected for one sampled cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellHermiteData {
    /// Sign-change intersections stored in deterministic edge order.
    pub intersections: Vec<CellHermiteIntersection>,
    /// Corner sign mask where bit `i` means corner `i` is inside the surface.
    pub corner_signs: u8,
}

impl CellHermiteData {
    /// Creates an empty cell Hermite record for one corner sign mask.
    #[must_use]
    pub fn new(corner_signs: u8) -> Self {
        Self {
            intersections: Vec::new(),
            corner_signs,
        }
    }

    /// Appends one intersection in deterministic caller order.
    pub fn push(&mut self, edge_index: u8, intersection: HermiteIntersection) {
        self.intersections.push(CellHermiteIntersection {
            edge_index,
            intersection,
        });
    }
}

/// Parameters controlling edge intersection search.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EdgeSearchParams {
    /// Number of bisection refinement rounds.
    pub bisection_steps: u8,
}

impl Default for EdgeSearchParams {
    fn default() -> Self {
        Self { bisection_steps: 8 }
    }
}

/// Edge intersection search error.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EdgeIntersectionError {
    /// Edge endpoints do not straddle the surface.
    NoSignChange,
}

/// Locates a zero-crossing along one sampled edge.
///
/// The search uses the bulk [`ScalarField::eval_points`] boundary to perform
/// deterministic bisection, then uses [`ScalarField::eval_gradients`] once at
/// the final point to gather the Hermite normal.
pub fn locate_edge_intersection<F: ScalarField>(
    field: &F,
    start: [f32; 3],
    end: [f32; 3],
    params: &EdgeSearchParams,
) -> Result<HermiteIntersection, EdgeIntersectionError> {
    let mut values = [0.0; 2];
    field.eval_points(&[start, end], &mut values);
    let [mut a_value, b_value] = values;

    if a_value == 0.0 {
        return sample_gradient(field, start, 0.0);
    }
    if b_value == 0.0 {
        return sample_gradient(field, end, 1.0);
    }
    if same_sign(a_value, b_value) {
        return Err(EdgeIntersectionError::NoSignChange);
    }

    let mut a_t = 0.0_f32;
    let mut b_t = 1.0_f32;
    let mut point_value = a_value;
    let mut point_t = a_t;
    let mut point = start;

    for _ in 0..params.bisection_steps {
        point_t = 0.5 * (a_t + b_t);
        point = lerp(start, end, point_t);
        field.eval_points(&[point], core::slice::from_mut(&mut point_value));

        if point_value == 0.0 {
            break;
        }
        if same_sign(a_value, point_value) {
            a_t = point_t;
            a_value = point_value;
        } else {
            b_t = point_t;
        }
    }

    sample_gradient(field, point, point_t)
}

fn sample_gradient<F: ScalarField>(
    field: &F,
    point: [f32; 3],
    t: f32,
) -> Result<HermiteIntersection, EdgeIntersectionError> {
    let mut out = [[0.0; 4]; 1];
    field.eval_gradients(&[point], &mut out);
    Ok(HermiteIntersection {
        position: point,
        normal: [out[0][1], out[0][2], out[0][3]],
        t,
    })
}

fn same_sign(a: f32, b: f32) -> bool {
    (a < 0.0 && b < 0.0) || (a > 0.0 && b > 0.0)
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        CellHermiteData, EdgeIntersectionError, EdgeSearchParams, locate_edge_intersection,
    };
    use crate::ScalarField;
    use crate::analytic::SphereField;
    use exedra_spatial::Aabb;

    #[derive(Copy, Clone, Debug)]
    struct NanGradientField;

    impl ScalarField for NanGradientField {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            Some([-1.0, 1.0])
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            for (index, point) in points.iter().enumerate() {
                out[index] = point[0];
            }
        }

        fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
            for (index, point) in points.iter().enumerate() {
                out[index] = [point[0], f32::NAN, f32::NAN, f32::NAN];
            }
        }
    }

    #[test]
    fn locate_edge_intersection_finds_known_sphere_crossing() {
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let hit = locate_edge_intersection(
            &sphere,
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            &EdgeSearchParams {
                bisection_steps: 12,
            },
        )
        .expect("sphere edge should cross once");

        assert!((hit.position[0] - 1.0).abs() <= 1.0e-3);
        assert!(hit.position[1].abs() <= 1.0e-5);
        assert!(hit.position[2].abs() <= 1.0e-5);
        assert!((hit.normal[0] - 1.0).abs() <= 1.0e-3);
        assert!(hit.normal[1].abs() <= 1.0e-5);
        assert!(hit.normal[2].abs() <= 1.0e-5);
        assert!((hit.t - 0.5).abs() <= 1.0e-3);
    }

    #[test]
    fn locate_edge_intersection_rejects_edges_without_sign_change() {
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let error = locate_edge_intersection(
            &sphere,
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            &EdgeSearchParams::default(),
        )
        .expect_err("outside edge should not cross");
        assert_eq!(error, EdgeIntersectionError::NoSignChange);
    }

    #[test]
    fn locate_edge_intersection_preserves_nan_gradients() {
        let hit = locate_edge_intersection(
            &NanGradientField,
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            &EdgeSearchParams::default(),
        )
        .expect("sign-changing edge should cross");

        assert!(hit.normal[0].is_nan());
        assert!(hit.normal[1].is_nan());
        assert!(hit.normal[2].is_nan());
    }

    #[test]
    fn cell_hermite_data_keeps_deterministic_edge_order() {
        let mut data = CellHermiteData::new(0b0101_1010);
        let sphere = SphereField {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
        };
        let first = locate_edge_intersection(
            &sphere,
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            &EdgeSearchParams::default(),
        )
        .expect("edge should cross");
        let second = locate_edge_intersection(
            &sphere,
            [0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            &EdgeSearchParams::default(),
        )
        .expect("edge should cross");

        data.push(0, first);
        data.push(3, second);

        assert_eq!(data.corner_signs, 0b0101_1010);
        assert_eq!(data.intersections[0].edge_index, 0);
        assert_eq!(data.intersections[1].edge_index, 3);
    }
}
