// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Single-course tile coverings on pitched coping, with fitted plan boundaries.

use crate::{Result, scene::Scene};
use exedra_assembly::{PartId, compose};
use exedra_constructive::discretize::discretize_loop;
use exedra_constructive::ir::{CapMode, CsgOp, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use std::f64::consts::FRAC_PI_2;

const RUN: f64 = 0.36;
const SLOPE: f64 = 0.28;
const EAVE_Z: f64 = 0.065;
const RIDGE_Y: f64 = 0.055;
const PITCH: f64 = 0.18;
const THICKNESS: f64 = 0.010;

fn deck(y: f64) -> f64 {
    EAVE_Z + (RUN - y.abs()) * SLOPE
}

/// `x >= start*y` and `x <= length + end*y` bound a cap in wall-local plan.
/// Zero ends are square; signed unit slopes form matching corner mitres.
#[derive(Clone, Copy)]
pub(crate) struct Ends {
    pub(crate) start: f64,
    pub(crate) end: f64,
}
impl Ends {
    pub(crate) const SQUARE: Self = Self {
        start: 0.0,
        end: 0.0,
    };

    fn profile(self, length: f64) -> Result<Profile2> {
        polygon(&[
            [self.start * -0.5, -0.5],
            [length + self.end * -0.5, -0.5],
            [length + self.end * 0.5, 0.5],
            [self.start * 0.5, 0.5],
        ])
    }
}

pub(crate) fn build(
    scene: &mut Scene,
    key: &str,
    length: f64,
    frame: Placement3,
    ends: Ends,
) -> Result<()> {
    let footprint = ends.profile(length)?;
    let base = polygon(&[
        [-RUN, -0.03],
        [RUN, -0.03],
        [RUN, EAVE_Z],
        [0.0, deck(0.0)],
        [-RUN, EAVE_Z],
    ])?;
    let base = scene.recipe(
        &format!("{key}-coping"),
        extrusion(&base, length + 1.0, along_wall(-0.5), Some(&footprint))?,
        "stone",
    )?;
    scene.place(base, frame, None)?;

    let c = 1.0 / (1.0 + SLOPE * SLOPE).sqrt();
    let s = SLOPE * c;
    let travel = (RUN - RIDGE_Y) / c;
    let cap = Tile::new(
        scene,
        key,
        shell(0.057, FRAC_PI_2, true)?,
        travel,
        cross_wall(travel),
    )?;
    let pan = Tile::new(
        scene,
        &format!("{key}-pan"),
        shell(0.11, (0.082_f64 / 0.11).asin(), false)?,
        travel,
        cross_wall(travel),
    )?;
    let mut placement = TilePlacement {
        scene,
        key,
        frame,
        footprint,
        ends,
        length,
        serial: 0,
    };
    // Fixed pitch retains the pan/cover fit. End tiles are cut to the wall,
    // rather than squeezing the pitch and colliding adjacent clay shells.
    for column in -3..=count(length, PITCH) + 3 {
        for side in [-1.0, 1.0] {
            for (tile, x, lift) in [
                (&cap, f64::from(column) * PITCH, 0.022),
                (&pan, (f64::from(column) + 0.5) * PITCH, 0.001),
            ] {
                let local = Placement3::from_axes(
                    [side, 0.0, 0.0],
                    [0.0, side * c, -s],
                    [0.0, side * s, c],
                    [x, side * (RIDGE_Y + lift * s), deck(RIDGE_Y) + lift * c],
                );
                placement.emit(tile, local)?;
            }
        }
    }
    // A raised ridge cap stands on the coping; its side lips shelter the
    // upper ends of both tile slopes. Each segment is a closed clay solid.
    let ridge = Tile::new(
        placement.scene,
        &format!("{key}-ridge"),
        ridge_profile()?,
        0.298,
        along_wall(0.0),
    )?;
    for i in -2..=count(length, 0.3) + 2 {
        let x = f64::from(i) * 0.3 + 0.001;
        placement.emit(&ridge, Placement3::translate(x, 0.0, 0.0))?;
    }
    Ok(())
}

/// An extrusion prototype and its vertices at the example's export tolerance.
/// Plane bounds use the actual shell, avoiding empty cuts from its loose box.
struct Tile {
    part: PartId,
    profile: Profile2,
    travel: f64,
    section: Placement3,
    vertices: Vec<[f64; 3]>,
}
impl Tile {
    fn new(
        scene: &mut Scene,
        key: &str,
        profile: Profile2,
        travel: f64,
        section: Placement3,
    ) -> Result<Self> {
        let part = scene.recipe(
            key,
            extrusion(&profile, travel, section, None)?,
            "roof.tile",
        )?;
        let outline = discretize_loop(profile.outer(), &crate::policy().evaluation.discretize)?;
        let mut vertices = Vec::new();
        for p in outline.points {
            for along in [0.0, travel] {
                vertices.push(point(section, [p[0], p[1], along]));
            }
        }
        Ok(Self {
            part,
            profile,
            travel,
            section,
            vertices,
        })
    }
}
fn point(frame: Placement3, p: [f64; 3]) -> [f64; 3] {
    frame
        .rows
        .map(|r| r[0] * p[0] + r[1] * p[1] + r[2] * p[2] + r[3])
}

struct TilePlacement<'a> {
    scene: &'a mut Scene,
    key: &'a str,
    frame: Placement3,
    footprint: Profile2,
    ends: Ends,
    length: f64,
    serial: u32,
}
impl TilePlacement<'_> {
    fn emit(&mut self, tile: &Tile, local: Placement3) -> Result<()> {
        let mut minimum = [f64::INFINITY; 2];
        let mut maximum = [f64::NEG_INFINITY; 2];
        for &vertex in &tile.vertices {
            let [x, y, _] = point(local, vertex);
            let distances = [x - self.ends.start * y, self.length + self.ends.end * y - x];
            for i in 0..2 {
                minimum[i] = minimum[i].min(distances[i]);
                maximum[i] = maximum[i].max(distances[i]);
            }
        }
        if maximum.iter().any(|d| *d <= 1e-9) {
            return Ok(());
        }
        if minimum.iter().all(|d| *d >= -1e-9) {
            self.scene
                .place(tile.part, compose(&self.frame, &local), None)?;
        } else {
            let recipe = extrusion(
                &tile.profile,
                tile.travel,
                compose(&local, &tile.section),
                Some(&self.footprint),
            )?;
            let cut = self.scene.recipe(
                &format!("{}-end-cut-{}", self.key, self.serial),
                recipe,
                "roof.tile",
            )?;
            self.serial += 1;
            self.scene.place(cut, self.frame, None)?;
        }
        Ok(())
    }
}

fn along_wall(x: f64) -> Placement3 {
    Placement3::from_axes(
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [x, 0.0, 0.0],
    )
}
fn cross_wall(travel: f64) -> Placement3 {
    Placement3::from_axes(
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [0.0, travel, 0.0],
    )
}
fn polygon(points: &[[f64; 2]]) -> Result<Profile2> {
    Ok(Profile2::simple(Loop2::new(
        points.iter().map(|p| Seg2::line((p[0], p[1]))).collect(),
    )?)?)
}
fn extrusion(
    profile: &Profile2,
    travel: f64,
    section: Placement3,
    clip: Option<&Profile2>,
) -> Result<Recipe> {
    let mut builder = RecipeBuilder::new();
    let slot = builder.material_slot("surface");
    let profile = builder.add_profile(profile.clone());
    let mut root = builder.with_material(slot).add(NodeKind::Extrude {
        profile,
        height: travel,
        caps: CapMode::Both,
        placement: section,
    })?;
    if let Some(clip) = clip {
        let profile = builder.add_profile(clip.clone());
        let boundary = builder.with_material(slot).add(NodeKind::Extrude {
            profile,
            height: 1.0,
            caps: CapMode::Both,
            placement: Placement3::translate(0.0, 0.0, -0.1),
        })?;
        root = builder.add(NodeKind::Csg {
            op: CsgOp::Intersection,
            operands: vec![root, boundary],
        })?;
    }
    Ok(builder.finish(root)?)
}
fn shell(radius: f64, angle: f64, cover: bool) -> Result<Profile2> {
    let sign = if cover { 1.0 } else { -1.0 };
    let offset = if cover { 0.0 } else { radius };
    let point = |r: f64, side: f64| (side * r * angle.sin(), offset + sign * r * angle.cos());
    let bulge = -sign * (angle * 0.5).tan();
    let outline = Loop2::new(vec![
        Seg2::arc(point(radius, 1.0), bulge),
        Seg2::line(point(radius - THICKNESS, 1.0)),
        Seg2::arc(point(radius - THICKNESS, -1.0), -bulge),
        Seg2::line(point(radius, -1.0)),
    ])?;
    Ok(Profile2::simple(if cover {
        outline.reversed()
    } else {
        outline
    })?)
}
fn ridge_profile() -> Result<Profile2> {
    Ok(Profile2::simple(Loop2::new(vec![
        Seg2::line((-0.05, deck(0.05))),
        Seg2::line((0.0, deck(0.0))),
        Seg2::line((0.05, deck(0.05))),
        Seg2::line((0.05, 0.239)),
        Seg2::line((0.12, 0.239)),
        Seg2::line((0.12, 0.251)),
        Seg2::line((0.05, 0.269)),
        Seg2::line((0.04, 0.290)),
        Seg2::arc((-0.04, 0.290), 1.0),
        Seg2::line((-0.05, 0.269)),
        Seg2::line((-0.12, 0.251)),
        Seg2::line((-0.12, 0.239)),
        Seg2::line((-0.05, 0.239)),
    ])?)?)
}

fn count(length: f64, pitch: f64) -> i32 {
    let mut count = 1;
    while f64::from(count) * pitch < length {
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests;
