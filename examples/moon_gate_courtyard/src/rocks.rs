// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Garden stones extracted from an authored scalar field.

use crate::{Result, scene::Scene};
use exedra_constructive::ir::Placement3;
use exedra_isosurface::{
    Aabb, DualContourParams, EdgeSearchParams, QefParams, ScalarField, dual_contour,
};

struct GardenRock;
impl GardenRock {
    fn value(p: [f32; 3]) -> f32 {
        let [x, y, z] = p;
        let ellipsoid = |cx: f32, cy: f32, cz: f32, sx: f32, sy: f32, sz: f32| {
            (((x - cx) / sx).powi(2) + ((y - cy) / sy).powi(2) + ((z - cz) / sz).powi(2)).sqrt()
                - 1.0
        };
        let a = ellipsoid(-0.12, 0.02, 0.65, 0.54, 0.44, 0.95);
        let b = ellipsoid(0.14, 0.0, 1.38, 0.46, 0.32, 0.75);
        let k = 0.35;
        let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
        let mut value = b + (a - b) * h - k * h * (1.0 - h);
        value += 0.09 * (8.0 * x + 1.2).sin() * (9.0 * y - 0.7).sin() * (5.0 * z + 0.4).sin()
            + 0.025
                * (23.0 * x + 11.0 * y).sin()
                * (18.0 * y + 7.0 * z).sin()
                * (19.0 * z + 4.0 * x).sin();
        for (cx, cz, rx, rz) in [(0.02, 0.80, 0.20, 0.25), (0.12, 1.52, 0.12, 0.17)] {
            let hole = (((x - cx + 0.035 * (z * 13.0 + y * 5.0).sin()) / rx).powi(2)
                + ((z - cz) / rz).powi(2))
            .sqrt()
                - 1.0;
            value = value.max(-hole * 0.32);
        }
        value
    }
}
impl ScalarField for GardenRock {
    fn eval_interval(&self, _: &Aabb) -> Option<[f32; 2]> {
        None
    }
    fn eval_points(&self, points: &[[f32; 3]], out: &mut [f32]) {
        for (p, v) in points.iter().zip(out) {
            *v = Self::value(*p);
        }
    }
    fn eval_gradients(&self, points: &[[f32; 3]], out: &mut [[f32; 4]]) {
        for (p, row) in points.iter().zip(out) {
            row[0] = Self::value(*p);
            for axis in 0..3 {
                let mut a = *p;
                let mut b = *p;
                a[axis] -= 0.001;
                b[axis] += 0.001;
                row[axis + 1] = (Self::value(b) - Self::value(a)) / 0.002;
            }
        }
    }
}
pub(crate) fn garden(scene: &mut Scene) -> Result<()> {
    let result = dual_contour(
        &GardenRock,
        &DualContourParams {
            root_bounds: Aabb::new([-0.9, -0.8, -0.6], [0.9, 0.8, 2.4]).ok_or("rock bounds")?,
            max_depth: 6,
            cell_budget: None,
            edge_search: EdgeSearchParams::default(),
            qef: QefParams::default(),
        },
    )?;
    if !result.mesh.validate_deep().is_empty() {
        return Err("invalid garden rock mesh".into());
    }
    let rock = scene.mesh("pierced-garden-rock", result.mesh, "rock")?;
    scene.place(
        rock,
        Placement3::rotate_z_then_translate(-0.22, 2.3, 4.0, 0.02),
        None,
    )?;
    scene.place(
        rock,
        Placement3::from_axes(
            [0.38, 0.2, 0.0],
            [-0.2, 0.38, 0.0],
            [0.0, 0.0, 0.40],
            [1.1, 3.4, -0.08],
        ),
        None,
    )?;
    Ok(())
}
