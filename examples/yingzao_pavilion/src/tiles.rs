// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Nested clay shells, continuous courses, and authored eave ornaments.

use exedra_constructive::builders::circle;
use exedra_constructive::ir::{CapMode, LoftPolicy, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};

use crate::roof_section::{DECK_TOP, RoofSection, normal};
use crate::scene::Scene;
use crate::{Result, geometry, layout::Layout};

const LENGTH: f64 = 0.34;
const THICKNESS: f64 = 0.012;
const PAN_RADIUS: f64 = 0.13;
const PAN_HALF_WIDTH: f64 = 0.098;
const CAP_RADIUS: f64 = 0.083;
const TAPER: f64 = 0.032;
const PAN_LIFT: f64 = 0.184;
const CAP_LIFT: f64 = 0.09;
const CAP_EAVE_PROJECTION: f64 = 0.035;
const PAN_EAVE_PROJECTION: f64 = 0.025;
// The narrow end's inner arch clears the upper cover-tile envelope, including
// clay thickness and the 1 mm tessellation allowance near the springing line.
const RIDGE_RADIUS: f64 = 0.210;
const RIDGE_LIFT: f64 = 0.125;

pub(crate) fn roof(scene: &mut Scene, layout: &Layout) -> Result<()> {
    let section = RoofSection::new(layout);
    let width = layout.width + 1.2;
    let deck = scene.part("roof-decking", section.decking(width)?, "timber.dark")?;
    let cap = scene.part("cap-tile", shell(true, CAP_RADIUS, LENGTH)?, "tile.0")?;
    let pan = scene.part("pan-tile", shell(false, PAN_RADIUS, LENGTH)?, "tile.1")?;
    let end = scene.part("lotus-end-tile", terminal()?, "tile.2")?;
    let drip = scene.part("pointed-drip-tile", drip()?, "tile.1")?;
    let cap_courses = courses(&section, 0.245, CAP_EAVE_PROJECTION);
    let pan_courses = courses(&section, 0.230, PAN_EAVE_PROJECTION);
    let mut columns = 1_u32;
    while f64::from(columns) * 0.225 < width {
        columns += 1;
    }
    let pitch = width / f64::from(columns);
    for side in [-1.0, 1.0] {
        scene.orient(&format!("decking-{side}"), deck, section.placement(side))?;
        for column in 0..=columns {
            let x = -width * 0.5 + f64::from(column) * pitch;
            for (course, placement) in cap_courses.iter().enumerate() {
                scene.orient(
                    &format!("cap-{side}-{column}-{course}"),
                    cap,
                    placement.world(side, section.run(), x, CAP_LIFT),
                )?;
            }
            scene.orient(
                &format!("lotus-{side}-{column}"),
                end,
                cap_courses[0].world(side, section.run(), x, CAP_LIFT),
            )?;
            if column < columns {
                for (course, placement) in pan_courses.iter().enumerate() {
                    scene.orient(
                        &format!("pan-{side}-{column}-{course}"),
                        pan,
                        placement.world(side, section.run(), x + pitch * 0.5, PAN_LIFT),
                    )?;
                }
                scene.orient(
                    &format!("drip-{side}-{column}"),
                    drip,
                    pan_courses[0].world(side, section.run(), x + pitch * 0.5, PAN_LIFT),
                )?;
            }
        }
    }
    ridge(scene, &section, width)?;
    Ok(())
}

/// One rigid tile follows the chord over its course, including pitch changes.
#[derive(Clone, Copy)]
pub(crate) struct Course {
    pub(crate) start: [f64; 2],
    pub(crate) normal: [f64; 2],
}

impl Course {
    pub(crate) fn world(self, side: f64, run: f64, x: f64, lift: f64) -> Placement3 {
        let [n, c] = self.normal;
        Placement3::from_axes(
            [-side, 0.0, 0.0],
            [0.0, -side * c, -n],
            [0.0, -side * n, c],
            [
                x,
                side * (run - self.start[0] - lift * n),
                self.start[1] + lift * c,
            ],
        )
    }
}

pub(crate) fn courses(
    section: &RoofSection,
    maximum_pitch: f64,
    eave_projection: f64,
) -> Vec<Course> {
    let mut path = section.layer(DECK_TOP);
    let n = normal(path[0], path[1]);
    // Recess the pan and its attached drip apron behind the round cover ends.
    path[0][0] -= eave_projection * n[1];
    path[0][1] += eave_projection * n[0];
    let mut stations = [0.0; 5];
    for i in 1..5 {
        stations[i] =
            stations[i - 1] + (path[i][0] - path[i - 1][0]).hypot(path[i][1] - path[i - 1][1]);
    }
    let travel = stations[4] - LENGTH;
    let mut intervals = 1_u32;
    while f64::from(intervals) * maximum_pitch < travel {
        intervals += 1;
    }
    let at = |distance: f64| {
        let i = (0..4).find(|&i| distance <= stations[i + 1]).unwrap_or(3);
        let t = (distance - stations[i]) / (stations[i + 1] - stations[i]);
        [
            path[i][0] + t * (path[i + 1][0] - path[i][0]),
            path[i][1] + t * (path[i + 1][1] - path[i][1]),
        ]
    };
    (0..=intervals)
        .map(|i| {
            let station = travel * f64::from(i) / f64::from(intervals);
            let start = at(station);
            Course {
                start,
                normal: normal(start, at(station + LENGTH)),
            }
        })
        .collect()
}

/// Cover tiles narrow uphill; pans widen in radius while retaining their width.
/// The next tile nests outside a cover or inside a pan, with room for the clay
/// thickness and coarse tessellation. Equal-section extrusions cannot do this.
pub(crate) fn shell(cap: bool, radius: f64, length: f64) -> Result<Recipe> {
    let mut builder = RecipeBuilder::new();
    let material = builder.material_slot("surface");
    let mut sections = Vec::new();
    // The X/Z section frame faces downhill (-Y), so loft in that direction
    // too. Reversing only the section order would turn every face inward.
    for along in [length, 0.0] {
        let r = radius + if cap { -1.0 } else { 1.0 } * TAPER * along / LENGTH;
        let half_angle = if cap {
            core::f64::consts::FRAC_PI_2
        } else {
            (PAN_HALF_WIDTH / r).asin()
        };
        let profile = builder.add_profile(section(r, half_angle, cap)?);
        sections.push((
            Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -1.0, 0.0],
                [0.0, along, 0.0],
            ),
            profile,
        ));
    }
    let root = builder.with_material(material).add(NodeKind::Loft {
        sections,
        policy: LoftPolicy::Ruled,
        caps: CapMode::Both,
    })?;
    Ok(builder.finish(root)?)
}

fn section(radius: f64, half_angle: f64, cap: bool) -> Result<Profile2> {
    let sign = if cap { 1.0 } else { -1.0 };
    let point = |r: f64, side: f64| (side * r * half_angle.sin(), sign * r * half_angle.cos());
    let bulge = -sign * (half_angle * 0.5).tan();
    let outline = Loop2::new(vec![
        Seg2::arc(point(radius, 1.0), bulge),
        Seg2::line(point(radius - THICKNESS, 1.0)),
        Seg2::arc(point(radius - THICKNESS, -1.0), -bulge),
        Seg2::line(point(radius, -1.0)),
    ])?;
    Ok(Profile2::simple(if cap {
        outline.reversed()
    } else {
        outline
    })?)
}

fn ridge(scene: &mut Scene, section: &RoofSection, width: f64) -> Result<()> {
    // The closed ends project past the outer cover-tile rows, keeping the
    // end plates clear of the clay running up each gable edge.
    let width = width + 0.20;
    let part = scene.part("ridge-tile", shell(true, RIDGE_RADIUS, LENGTH)?, "tile.2")?;
    let mut intervals = 1_u32;
    while f64::from(intervals) * 0.245 < width - LENGTH {
        intervals += 1;
    }
    for i in 0..=intervals {
        let x = width * 0.5 - (width - LENGTH) * f64::from(i) / f64::from(intervals);
        scene.orient(
            &format!("ridge-{i}"),
            part,
            Placement3::from_axes(
                [0.0, 1.0, 0.0],
                [-1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [x, 0.0, section.layer(DECK_TOP)[4][1] + RIDGE_LIFT],
            ),
        )?;
    }
    for (side, radius) in [(1.0, RIDGE_RADIUS), (-1.0, RIDGE_RADIUS - TAPER)] {
        let end = scene.part(
            &format!("ridge-end-tile-{side}"),
            ridge_end(radius)?,
            "tile.2",
        )?;
        scene.orient(
            &format!("ridge-end-{side}"),
            end,
            Placement3::from_axes(
                [0.0, side, 0.0],
                [0.0, 0.0, 1.0],
                [side, 0.0, 0.0],
                [
                    side * width * 0.5,
                    0.0,
                    section.layer(DECK_TOP)[4][1] + RIDGE_LIFT,
                ],
            ),
        )?;
    }
    Ok(())
}

fn ridge_end(radius: f64) -> Result<Recipe> {
    let outline = Loop2::new(vec![
        Seg2::arc((radius, 0.0), -1.0),
        Seg2::line((-radius, 0.0)),
    ])?;
    geometry::extrude(
        Profile2::simple(outline.reversed())?,
        0.016,
        Placement3::IDENTITY,
    )
}

/// A round wadang face with an authored eight-petal lotus relief.
fn terminal() -> Result<Recipe> {
    let mut builder = RecipeBuilder::new();
    let material = builder.material_slot("surface");
    let mut nodes = Vec::new();
    let mut add = |profile, depth, outward: f64, angle: f64| -> Result<()> {
        let profile = builder.add_profile(profile);
        let (s, c) = angle.sin_cos();
        nodes.push(builder.with_material(material).add(NodeKind::Extrude {
            profile,
            height: depth,
            caps: CapMode::Both,
            placement: Placement3::from_axes(
                [c, 0.0, s],
                [-s, 0.0, c],
                [0.0, -1.0, 0.0],
                [0.0, -outward, 0.0],
            ),
        })?);
        Ok(())
    };
    add(circle(0.086)?, 0.016, 0.0, 0.0)?;
    let ring = Profile2::new(
        circle(0.078)?.outer().clone(),
        vec![circle(0.072)?.outer().reversed()],
    )?;
    add(ring, 0.003, 0.016, 0.0)?;
    add(circle(0.011)?, 0.004, 0.016, 0.0)?;
    let petal = Profile2::simple(
        Loop2::new(vec![
            Seg2::cubic((0.0, 0.064), (-0.012, 0.029), (-0.012, 0.050)),
            Seg2::cubic((0.0, 0.016), (0.012, 0.050), (0.012, 0.029)),
        ])?
        .reversed(),
    )?;
    for i in 0..8 {
        add(
            petal.clone(),
            0.003,
            0.016,
            f64::from(i) * core::f64::consts::FRAC_PI_4,
        )?;
    }
    let root = builder.add(NodeKind::Group { children: nodes })?;
    Ok(builder.finish(root)?)
}

/// A pointed dishui apron, whose curved upper rim meets the first pan tile.
fn drip() -> Result<Recipe> {
    let angle = (PAN_HALF_WIDTH / PAN_RADIUS).asin();
    let inner = PAN_RADIUS - THICKNESS;
    let x = inner * angle.sin();
    let z = -inner * angle.cos();
    let outline = Loop2::new(vec![
        Seg2::arc((x, z), (angle * 0.5).tan()),
        Seg2::line((0.105, -0.115)),
        Seg2::line((0.07, -0.163)),
        Seg2::line((0.0, -0.193)),
        Seg2::line((-0.07, -0.163)),
        Seg2::line((-0.105, -0.115)),
        Seg2::line((-x, z)),
    ])?;
    geometry::extrude(
        Profile2::simple(outline.reversed())?,
        0.018,
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
        ),
    )
}

#[cfg(test)]
mod tests;
