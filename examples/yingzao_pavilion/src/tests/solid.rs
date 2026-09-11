// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test-only membership sampling of the triangles actually sent to the renderer.

use std::f64::consts::PI;

use exedra_assembly::CompiledPart;
use exedra_constructive::{evaluate::Aabb3, ir::Placement3};
use exedra_math::{cross, dot, norm, sub};

pub(crate) struct Solid {
    pub(crate) bounds: Aabb3,
    bodies: Vec<Vec<[[f64; 3]; 3]>>,
}

impl Solid {
    pub(crate) fn new(part: &CompiledPart, placement: Placement3) -> Self {
        let mut bounds = Aabb3::EMPTY;
        let bodies = part
            .bodies
            .iter()
            .map(|body| {
                let positions: Vec<_> = body
                    .tri
                    .positions
                    .iter()
                    .map(|p| {
                        let p = p.map(f64::from);
                        let world = placement.rows.map(|r| dot([r[0], r[1], r[2]], p) + r[3]);
                        bounds.union(&Aabb3 {
                            min: world,
                            max: world,
                        });
                        world
                    })
                    .collect();
                body.tri
                    .indices
                    .chunks_exact(3)
                    .map(|t| {
                        [
                            positions[t[0] as usize],
                            positions[t[1] as usize],
                            positions[t[2] as usize],
                        ]
                    })
                    .collect()
            })
            .collect();
        assert!(
            !bounds.is_empty(),
            "membership needs real compiled geometry"
        );
        Self { bounds, bodies }
    }

    /// Solid-angle winding avoids ray/edge coincidences. Callers must sample
    /// away from surfaces: fractional winding is an error, not guessed inside.
    /// Absolute winding supports reflected placements; bodies form a union.
    pub(crate) fn contains(&self, point: [f64; 3]) -> bool {
        if (0..3).any(|i| point[i] < self.bounds.min[i] || point[i] > self.bounds.max[i]) {
            return false;
        }
        self.bodies.iter().any(|triangles| {
            let angle: f64 = triangles
                .iter()
                .map(|t| {
                    let [a, b, c] = t.map(|v| sub(v, point));
                    let [la, lb, lc] = [norm(a), norm(b), norm(c)];
                    2.0 * dot(a, cross(b, c))
                        .atan2(la * lb * lc + dot(a, b) * lc + dot(b, c) * la + dot(c, a) * lb)
                })
                .sum();
            let winding = angle.abs() / (4.0 * PI);
            assert!(
                winding.min((winding - 1.0).abs()) < 1.0e-5,
                "sample {point:?} is on a boundary or the mesh is not closed: winding={winding}"
            );
            winding > 0.5
        })
    }
}
