// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! One roof section owns slopes and normal offsets for the example's layers.

use exedra_constructive::ir::{Placement3, Recipe};

use crate::{Result, geometry, layout::Layout};

pub(crate) const RAFTER_DEPTH: f64 = 0.12;
pub(crate) const DECK_TOP: f64 = RAFTER_DEPTH + 0.028;
pub(crate) const EAVE_EXTENSION: f64 = 0.14;
pub(crate) const BEARING_ABOVE_BASE: f64 = 0.03;

/// Coordinates are horizontal distance inward from the eave purlin and height.
pub(crate) struct RoofSection {
    pub(crate) points: [[f64; 2]; 5],
}

impl RoofSection {
    pub(crate) fn new(layout: &Layout) -> Self {
        let run = layout.roof[0][0];
        Self {
            points: core::array::from_fn(|i| {
                [
                    run - layout.roof[i][0],
                    layout.rafter_bearing_height(i) - BEARING_ABOVE_BASE,
                ]
            }),
        }
    }

    pub(crate) fn run(&self) -> f64 {
        self.points[4][0]
    }

    /// Intersect adjacent offset lines, rather than adding a vertical lift.
    /// The ridge ends on its symmetry plane; the eave extends along its slope.
    pub(crate) fn layer(&self, offset: f64) -> [[f64; 2]; 5] {
        let normals: Vec<_> = self.points.windows(2).map(|p| normal(p[0], p[1])).collect();
        core::array::from_fn(|i| {
            let [mut u, mut z] = self.points[i];
            let shift = if i == 0 {
                u -= EAVE_EXTENSION;
                z -= EAVE_EXTENSION * -normals[0][0] / normals[0][1];
                normals[0]
            } else if i == 4 {
                [0.0, 1.0 / normals[3][1]]
            } else {
                let [a, b] = [normals[i - 1], normals[i]];
                let divisor = 1.0 + a[0] * b[0] + a[1] * b[1];
                [(a[0] + b[0]) / divisor, (a[1] + b[1]) / divisor]
            };
            [u + offset * shift[0], z + offset * shift[1]]
        })
    }

    pub(crate) fn decking(&self, width: f64) -> Result<Recipe> {
        let mut outline = self.layer(RAFTER_DEPTH).to_vec();
        outline.extend(self.layer(DECK_TOP).into_iter().rev());
        // Local X is across the building, local Y is uphill from the eave.
        geometry::extrude(
            geometry::polygon(&outline)?,
            width,
            Placement3::from_axes(
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [-width * 0.5, 0.0, 0.0],
            ),
        )
    }

    /// Place local X across the roof and local Y inward on either roof half.
    pub(crate) fn placement(&self, side: f64) -> Placement3 {
        Placement3::from_axes(
            [-side, 0.0, 0.0],
            [0.0, -side, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, side * self.run(), 0.0],
        )
    }
}

pub(crate) fn normal(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let [du, dz] = [b[0] - a[0], b[1] - a[1]];
    let length = du.hypot(dz);
    [-dz / length, du / length]
}

pub(crate) fn height(a: [f64; 2], b: [f64; 2], u: f64) -> f64 {
    a[1] + (b[1] - a[1]) * (u - a[0]) / (b[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Parameters;

    #[test]
    fn layers_stay_parallel_and_meet_at_every_pitch_change() -> Result<()> {
        for depth_mm in [2_400, 3_601, 5_400] {
            let section = RoofSection::new(&Layout::resolve(Parameters {
                depth_mm,
                ..Parameters::default()
            })?);
            for offset in [RAFTER_DEPTH, DECK_TOP] {
                let layer = section.layer(offset);
                for (i, pair) in section.points.windows(2).enumerate() {
                    let n = normal(pair[0], pair[1]);
                    for point in [layer[i], layer[i + 1]] {
                        let distance =
                            (point[0] - pair[0][0]) * n[0] + (point[1] - pair[0][1]) * n[1];
                        assert!((distance - offset).abs() < 1.0e-12);
                    }
                }
                assert_eq!(layer[4][0], section.run(), "closed ridge symmetry plane");
            }
        }
        Ok(())
    }
}
