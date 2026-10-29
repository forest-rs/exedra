// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{RectilinearError, RectilinearPolicy};
use alloc::vec::Vec;

pub(super) struct Grid {
    pub coordinates: [Vec<f64>; 3],
    pub references: [Vec<f64>; 3],
    pub planes: [usize; 3],
    pub dimensions: [usize; 3],
    pub cells: usize,
    pub arithmetic: f64,
    pub face_test_bound: usize,
}
impl Grid {
    pub(super) fn new(
        points: impl Iterator<Item = [f64; 3]>,
        policy: &RectilinearPolicy,
        source_faces: usize,
    ) -> Result<Self, RectilinearError> {
        let mut coordinates: [Vec<f64>; 3] = core::array::from_fn(|_| Vec::new());
        for point in points {
            for axis in 0..3 {
                coordinates[axis].push(point[axis]);
            }
        }
        let magnitude = coordinates
            .iter()
            .flatten()
            .map(|v| v.abs())
            .fold(0.0, f64::max);
        let arithmetic = 512.0 * f64::EPSILON * magnitude;
        if !arithmetic.is_finite() || arithmetic > policy.geometry_gap {
            return Err(RectilinearError::NumericLimit);
        }
        for (axis, values) in coordinates.iter_mut().enumerate() {
            values.sort_by(f64::total_cmp);
            values.dedup_by(|a, b| *a == *b);
            for pair in values.windows(2) {
                let distance = pair[1] - pair[0];
                if !distance.is_finite() {
                    return Err(RectilinearError::NumericLimit);
                }
                if distance < policy.minimum_spacing + arithmetic {
                    return Err(RectilinearError::PlaneSpacing {
                        axis,
                        lower: pair[0],
                        upper: pair[1],
                        required: policy.minimum_spacing + arithmetic,
                    });
                }
            }
            if values.len() < 2 {
                return Err(RectilinearError::NumericLimit);
            }
        }
        let planes = coordinates.each_ref().map(Vec::len);
        let dimensions = planes.map(|n| n - 1);
        let cells = dimensions[0]
            .checked_mul(dimensions[1])
            .and_then(|n| n.checked_mul(dimensions[2]));
        let Some(cells) = cells else {
            return Err(RectilinearError::CellBudget {
                required: None,
                maximum: policy.max_cells,
            });
        };
        if cells > policy.max_cells
            || cells
                .checked_mul(24)
                .is_none_or(|n| u32::try_from(n).is_err())
        {
            return Err(RectilinearError::CellBudget {
                required: Some(cells),
                maximum: policy.max_cells,
            });
        }
        let face_test_bound = cells
            .checked_mul(source_faces)
            .and_then(|n| n.checked_mul(9));
        let face_test_bound = face_test_bound
            .filter(|&n| n <= policy.max_face_tests)
            .ok_or(RectilinearError::FaceTestBudget {
                required: face_test_bound,
                maximum: policy.max_face_tests,
            })?;
        let [x, y, z] = coordinates.each_ref().map(|values| {
            values
                .windows(2)
                .map(|p| {
                    let middle = p[0] + (p[1] - p[0]) * 0.5;
                    if !middle.is_finite() || middle <= p[0] || middle >= p[1] {
                        Err(RectilinearError::NumericLimit)
                    } else {
                        Ok(middle)
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let references = [x?, y?, z?];
        Ok(Self {
            coordinates,
            references,
            planes,
            dimensions,
            cells,
            arithmetic,
            face_test_bound,
        })
    }
}
