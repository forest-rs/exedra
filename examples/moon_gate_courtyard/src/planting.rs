// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic branch and leaf geometry; no billboard or renderer-only plants.

use crate::{Result, scene::Scene};
use exedra_constructive::ir::Placement3;
use exedra_math::{add, cross, narrow, normalize, scale, sub};
use exedra_mesh::MeshBuilder;
use std::f64::consts::TAU;

#[derive(Default)]
struct Plant {
    wood: MeshBuilder,
    leaves: [MeshBuilder; 5],
}
struct Seed(u64);
impl Seed {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        f64::from((self.0 >> 32) as u32) / f64::from(u32::MAX)
    }
    fn signed(&mut self) -> f64 {
        self.next() * 2.0 - 1.0
    }
}
fn point(p: [f64; 3]) -> [f32; 3] {
    narrow(p)
}
fn tube(mesh: &mut MeshBuilder, path: &[[f64; 3]], r0: f64, r1: f64) -> Result<()> {
    let mut rings = Vec::new();
    for (i, &p) in path.iter().enumerate() {
        let tangent = normalize(sub(
            path[(i + 1).min(path.len() - 1)],
            path[i.saturating_sub(1)],
        ))
        .ok_or("zero branch tangent")?;
        let u = normalize(cross(
            tangent,
            if tangent[2].abs() > 0.9 {
                [1.0, 0.0, 0.0]
            } else {
                [0.0, 0.0, 1.0]
            },
        ))
        .ok_or("zero branch frame")?;
        let v = cross(tangent, u);
        let r = r0 + (r1 - r0) * i as f64 / (path.len() - 1) as f64;
        let mut ring = Vec::new();
        for j in 0..9 {
            let a = f64::from(j) * TAU / 9.0;
            ring.push(mesh.push_vertex(point(add(
                p,
                add(scale(u, r * a.cos()), scale(v, r * a.sin())),
            ))));
        }
        rings.push(ring);
    }
    let mut cap = rings[0].clone();
    cap.reverse();
    mesh.add_face(&cap)?;
    mesh.add_face(rings.last().ok_or("empty tube")?)?;
    for pair in rings.windows(2) {
        for j in 0..9 {
            let k = (j + 1) % 9;
            mesh.add_face(&[pair[0][j], pair[0][k], pair[1][k], pair[1][j]])?;
        }
    }
    Ok(())
}
fn leaf(
    mesh: &mut MeshBuilder,
    p: [f64; 3],
    length: f64,
    width: f64,
    angle: f64,
    tilt: f64,
) -> Result<()> {
    let along = [
        angle.cos() * tilt.cos(),
        angle.sin() * tilt.cos(),
        tilt.sin(),
    ];
    let across = [-angle.sin(), angle.cos(), 0.0];
    let positions = [
        p,
        add(
            p,
            add(scale(along, length * 0.45), scale(across, width * 0.5)),
        ),
        add(p, scale(along, length)),
        add(
            p,
            add(scale(along, length * 0.45), scale(across, -width * 0.5)),
        ),
        add(
            add(p, scale(along, length * 0.45)),
            [0.0, 0.0, width * 0.16],
        ),
    ];
    let ids = positions.map(|p| mesh.push_vertex(point(p)));
    for j in 0..4 {
        mesh.add_face(&[ids[j], ids[(j + 1) % 4], ids[4]])?;
    }
    Ok(())
}
impl Plant {
    fn crown(
        &mut self,
        center: [f64; 3],
        spread: [f64; 3],
        rng: &mut Seed,
        count: u32,
    ) -> Result<()> {
        for i in 0..count {
            let a = rng.next() * TAU;
            let r = rng.next().sqrt();
            let h = rng.signed();
            let p = add(
                center,
                [
                    spread[0] * r * a.cos(),
                    spread[1] * r * a.sin(),
                    spread[2] * h * (1.0 - r * 0.4),
                ],
            );
            let length = 0.13 + rng.next() * 0.10;
            leaf(
                &mut self.leaves[(i % 5) as usize],
                p,
                length,
                length * 0.53,
                rng.next() * TAU,
                rng.signed() * 0.65,
            )?;
        }
        Ok(())
    }
    fn branch(
        &mut self,
        start: [f64; 3],
        direction: [f64; 3],
        length: f64,
        radius: f64,
        depth: u32,
        rng: &mut Seed,
    ) -> Result<()> {
        let direction = normalize(direction).ok_or("zero branch direction")?;
        let tip = add(start, scale(direction, length));
        let path: Vec<_> = (0..=5)
            .map(|i| {
                let t = f64::from(i) / 5.0;
                let mut p = add(start, scale(direction, length * t));
                p[0] += 0.10 * length * (t * core::f64::consts::PI).sin();
                p[2] -= length * 0.08 * t * t;
                p
            })
            .collect();
        tube(&mut self.wood, &path, radius, radius * 0.43)?;
        if depth == 0 {
            self.crown(tip, [0.43, 0.42, 0.19], rng, 105)?;
            return Ok(());
        }
        let azimuth = direction[1].atan2(direction[0]);
        for j in 0..3 {
            let angle = azimuth + (f64::from(j) - 1.0) * 1.6 + rng.signed() * 0.35;
            let d = [angle.cos(), angle.sin(), 0.2 + rng.next() * 0.65];
            let at = add(
                start,
                scale(direction, length * (0.68 + f64::from(j) * 0.13)),
            );
            self.branch(
                at,
                d,
                length * (0.56 + rng.next() * 0.09),
                radius * 0.48,
                depth - 1,
                rng,
            )?;
        }
        Ok(())
    }
    fn emit(self, scene: &mut Scene, key: &str, at: [f64; 3], wood: &str) -> Result<()> {
        let bark = scene.mesh(&format!("{key}-branches"), self.wood.build()?.mesh, wood)?;
        scene.at(bark, at)?;
        for (i, leaves) in self.leaves.into_iter().enumerate() {
            let material = format!("leaf.{i}");
            let part = scene.mesh(
                &format!("{key}-leaves-{i}"),
                leaves.build()?.mesh,
                &material,
            )?;
            scene.at(part, at)?;
        }
        Ok(())
    }
}
pub(crate) fn garden(scene: &mut Scene) -> Result<()> {
    let mut tree = Plant::default();
    let mut rng = Seed(81721);
    tube(
        &mut tree.wood,
        &[
            [0.0, 0.0, -0.12],
            [0.06, 0.04, 0.65],
            [0.21, 0.0, 1.3],
            [0.16, 0.11, 1.8],
        ],
        0.16,
        0.09,
    )?;
    for (start, direction, length) in [
        ([0.12, 0.05, 1.35], [-1.0, 0.25, 1.0], 1.55),
        ([0.16, 0.1, 1.65], [0.7, 0.0, 1.0], 1.50),
        ([0.19, 0.02, 1.5], [0.0, -1.0, 0.7], 1.1),
        ([0.2, 0.12, 1.7], [0.1, 1.0, 1.0], 1.45),
    ] {
        tree.branch(start, direction, length, 0.075, 2, &mut rng)?;
    }
    tree.emit(scene, "courtyard-tree", [1.55, 5.6, 0.0], "bark")?;
    // A reusable clump of arching ground foliage, with a folded leaf ridge.
    let mut foliage = MeshBuilder::new();
    for _ in 0..28 {
        let a = rng.next() * TAU;
        let length = 0.23 + rng.next() * 0.32;
        leaf(
            &mut foliage,
            [
                rng.signed() * 0.09,
                rng.signed() * 0.09,
                0.10 + rng.next() * 0.12,
            ],
            length,
            length * 0.33,
            a,
            0.30 + rng.next() * 0.35,
        )?;
    }
    let clump = scene.mesh("ground-leaves", foliage.build()?.mesh, "leaf.1")?;
    for i in 0..150 {
        let x = 0.85 + rng.next() * 3.48;
        let y = 2.88 + rng.next() * 4.22;
        if ((x - 2.8) / 0.85).powi(2) + ((y - 4.1) / 0.8).powi(2) < 1.0 {
            continue;
        }
        scene.place(
            clump,
            Placement3::rotate_z_then_translate(rng.next() * TAU, x, y, -0.02),
            Some(&format!("leaf.{}", i % 5)),
        )?;
    }
    let mut bamboo = Plant::default();
    for i in 0..13 {
        let x = rng.signed() * 0.55;
        let y = rng.signed() * 0.4;
        let height = 2.4 + rng.next() * 1.6;
        let lean = rng.signed() * 0.35;
        tube(
            &mut bamboo.wood,
            &[
                [x, y, 0.0],
                [x + lean * 0.25, y, 1.0],
                [x + lean, y + 0.10, height],
            ],
            0.017,
            0.010,
        )?;
        for j in 2..7 {
            let z = height * f64::from(j) / 7.0;
            let sign = if (i + j) % 2 == 0 { -1.0 } else { 1.0 };
            let tip = [x + lean * z / height + sign * 0.4, y + 0.13, z + 0.1];
            tube(
                &mut bamboo.wood,
                &[[x + lean * z / height, y, z], tip],
                0.004,
                0.001,
            )?;
            for k in 0..16 {
                let p = add(
                    tip,
                    [
                        rng.signed() * 0.24,
                        rng.signed() * 0.25,
                        rng.signed() * 0.08,
                    ],
                );
                leaf(
                    &mut bamboo.leaves[(k % 5) as usize],
                    p,
                    0.16,
                    0.028,
                    rng.next() * TAU,
                    -0.3 + rng.next() * 0.6,
                )?;
            }
        }
    }
    bamboo.emit(scene, "bamboo", [3.5, 6.7, -0.03], "bamboo")?;
    Ok(())
}
