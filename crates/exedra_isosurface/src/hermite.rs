// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hermite data types and edge intersection search.

extern crate alloc;

use alloc::vec::Vec;

use crate::ScalarField;
use exedra_math::lerp;

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
    /// Precision of each crossing: the search stops once the crossing is
    /// bracketed within `2^-precision_bits` of the edge's length.
    pub precision_bits: u8,
}

impl Default for EdgeSearchParams {
    fn default() -> Self {
        Self { precision_bits: 8 }
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
/// The search samples both endpoints, brackets the crossing by false
/// position through the bulk [`ScalarField::eval_points`] boundary, then uses
/// [`ScalarField::eval_gradients`] once at the final point to gather the
/// Hermite normal.
pub fn locate_edge_intersection<F: ScalarField>(
    field: &F,
    start: [f32; 3],
    end: [f32; 3],
    params: &EdgeSearchParams,
) -> Result<HermiteIntersection, EdgeIntersectionError> {
    let mut values = [0.0; 2];
    field.eval_points(&[start, end], &mut values);
    let (point, t) = search_edge_zero(field, [start, end], values, params)?;
    sample_gradient(field, point, t)
}

/// Locates a zero-crossing without sampling its gradient.
pub(crate) fn locate_edge_zero<F: ScalarField>(
    field: &F,
    start: [f32; 3],
    end: [f32; 3],
    params: &EdgeSearchParams,
) -> Result<([f32; 3], f32), EdgeIntersectionError> {
    let mut values = [0.0; 2];
    field.eval_points(&[start, end], &mut values);
    search_edge_zero(field, [start, end], values, params)
}

/// Locates the zero-crossing between `ends`, whose field values are
/// `values`, and its gradient.
pub(crate) fn search_edge_intersection<F: ScalarField>(
    field: &F,
    ends: [[f32; 3]; 2],
    values: [f32; 2],
    params: &EdgeSearchParams,
) -> Result<HermiteIntersection, EdgeIntersectionError> {
    let (point, t) = search_edge_zero(field, ends, values, params)?;
    sample_gradient(field, point, t)
}

/// Brackets the zero-crossing between `ends`, whose field values are
/// `values`, to one of `2^precision_bits` equal steps along the edge, and
/// returns that step's midpoint.
///
/// Steps are chosen by false position, rounded to the step grid; the
/// Illinois rule halves the weight of an end kept twice in a row, so both
/// ends close in, and a bracket that fails to halve twice in a row is
/// bisected. A smooth crossing takes a few samples where bisection takes
/// `precision_bits`. The step found depends only on signs, as with
/// bisection, so edges crossing one plane at the same place agree exactly.
fn search_edge_zero<F: ScalarField>(
    field: &F,
    [start, end]: [[f32; 3]; 2],
    [start_value, end_value]: [f32; 2],
    params: &EdgeSearchParams,
) -> Result<([f32; 3], f32), EdgeIntersectionError> {
    if start_value == 0.0 {
        return Ok((start, 0.0));
    }
    if end_value == 0.0 {
        return Ok((end, 1.0));
    }
    if !same_sign(start_value, -end_value) {
        return Err(EdgeIntersectionError::NoSignChange);
    }
    // Step indices stay exact in `f32` up to 2^24.
    let steps = 1_u32 << params.precision_bits.min(24);
    #[expect(clippy::cast_precision_loss, reason = "step indices are below 2^24")]
    let at = |index: u32| index as f32 / steps as f32;
    let (mut low, mut high) = ((0_u32, start_value), (steps, end_value));
    // Weights for false position: the values, halved per the Illinois rule.
    let (mut low_weight, mut high_weight) = (start_value, end_value);
    let mut kept: Option<bool> = None;
    let mut slow_steps = 0;
    while high.0 - low.0 > 1 {
        let span = high.0 - low.0;
        let guess = f64::from(low.0)
            + f64::from(span) * f64::from(low_weight)
                / (f64::from(low_weight) - f64::from(high_weight));
        let index = if slow_steps >= 2 || !guess.is_finite() {
            low.0 + span / 2
        } else {
            // Rounds to the nearest step inside the bracket.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to the bracket's interior first"
            )]
            let nearest = (guess.clamp(f64::from(low.0 + 1), f64::from(high.0 - 1)) + 0.5) as u32;
            nearest
        };
        let t = at(index);
        let point = lerp(start, end, t);
        let mut value = 0.0;
        field.eval_points(&[point], core::slice::from_mut(&mut value));
        if value == 0.0 {
            return Ok((point, t));
        }
        if same_sign(low.1, value) {
            low = (index, value);
            low_weight = value;
            if kept == Some(true) {
                high_weight *= 0.5;
            }
            kept = Some(true);
        } else {
            high = (index, value);
            high_weight = value;
            if kept == Some(false) {
                low_weight *= 0.5;
            }
            kept = Some(false);
        }
        slow_steps = if 2 * (high.0 - low.0) > span {
            slow_steps + 1
        } else {
            0
        };
    }
    let t = 0.5 * (at(low.0) + at(high.0));
    Ok((lerp(start, end, t), t))
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

#[cfg(test)]
mod tests {
    use super::{
        CellHermiteData, EdgeIntersectionError, EdgeSearchParams, locate_edge_intersection,
        locate_edge_zero,
    };
    use crate::ScalarField;
    use crate::analytic::SphereField;
    use crate::spatial::Aabb;

    #[derive(Copy, Clone, Debug)]
    struct NanGradientField;

    #[derive(Copy, Clone, Debug)]
    struct ScalarOnlyPlane;

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

    impl ScalarField for ScalarOnlyPlane {
        fn eval_interval(&self, _bounds: &Aabb) -> Option<[f32; 2]> {
            Some([-1.0, 1.0])
        }

        fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
            for (index, point) in points.iter().enumerate() {
                out[index] = point[0];
            }
        }

        fn eval_gradients(&self, _points: &[[f32; 3]], _out: &mut [[f32; 4]]) {
            panic!("scalar-only zero search must not request gradients");
        }
    }

    #[test]
    fn locate_edge_zero_does_not_sample_gradients() {
        let (point, t) = locate_edge_zero(
            &ScalarOnlyPlane,
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            &EdgeSearchParams::default(),
        )
        .expect("plane edge should cross");

        assert_eq!(point, [0.0, 0.0, 0.0]);
        assert_eq!(t, 0.5);
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
            &EdgeSearchParams { precision_bits: 12 },
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
