// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! An authored dàjǐ: fitted ridge-foot closures, a fillet course, and clay crown.

use exedra_constructive::ir::{NodeKind, Placement3, Plane3, PlaneSide, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_constructive::section::CutCap;

use super::{
    CAP_EAVE_PROJECTION, CAP_LIFT, CAP_RADIUS, LENGTH, PAN_HALF_WIDTH, PAN_LIFT, PAN_RADIUS, TAPER,
    THICKNESS, courses, shell,
};
use crate::layout::interval_count;
use crate::roof_section::{DECK_TOP, RoofSection};
use crate::scene::Scene;
use crate::{Result, geometry};

pub(super) const RADIUS: f64 = 0.180;
pub(super) const LIFT: f64 = 0.240;
const FILLET_BASE_HALF_WIDTH: f64 = 0.180;
pub(super) const FOOT_TOP: f64 = 0.215;
const FILLET_HALF_WIDTH: f64 = 0.198;
const END_PROJECTION: f64 = 0.100;
const END_THICKNESS: f64 = 0.016;

pub(super) fn build(
    scene: &mut Scene,
    section: &RoofSection,
    roof_width: f64,
    columns: u32,
) -> Result<()> {
    let pitch = roof_width / f64::from(columns);
    let z = section.layer(DECK_TOP)[4][1];
    let foot_part = scene.part("ridge-foot", foot(section, pitch, false)?, "tile.2")?;
    let end_foot = scene.part(
        "ridge-foot-end",
        foot(section, END_PROJECTION + END_THICKNESS, true)?,
        "tile.2",
    )?;
    let fillet_part = scene.part("ridge-fillet", fillet(pitch)?, "tile.2")?;
    let end_fillet = scene.part(
        "ridge-fillet-end",
        fillet(END_PROJECTION + END_THICKNESS)?,
        "tile.2",
    )?;
    for column in 0..columns {
        let x = -roof_width * 0.5 + f64::from(column) * pitch;
        scene.place(&format!("ridge-foot-{column}"), foot_part, [x, 0.0, z])?;
        scene.place(&format!("ridge-fillet-{column}"), fillet_part, [x, 0.0, z])?;
    }
    for side in [-1.0, 1.0] {
        let placement = Placement3::from_axes(
            [side, 0.0, 0.0],
            [0.0, side, 0.0],
            [0.0, 0.0, 1.0],
            [side * roof_width * 0.5, 0.0, z],
        );
        scene.orient(&format!("ridge-foot-end-{side}"), end_foot, placement)?;
        scene.orient(&format!("ridge-fillet-end-{side}"), end_fillet, placement)?;
    }
    let width = roof_width + 2.0 * END_PROJECTION;
    let part = scene.part("ridge-tile", shell(true, RADIUS, LENGTH)?, "tile.2")?;
    let intervals = interval_count(width - LENGTH, 0.245);
    for i in 0..=intervals {
        let x = width * 0.5 - (width - LENGTH) * f64::from(i) / f64::from(intervals);
        scene.orient(&format!("ridge-{i}"), part, crown_placement(x, z))?;
    }
    for (side, radius) in [(1.0, RADIUS), (-1.0, RADIUS - TAPER)] {
        let end = scene.part(&format!("ridge-end-tile-{side}"), end(radius)?, "tile.2")?;
        scene.orient(
            &format!("ridge-end-{side}"),
            end,
            Placement3::from_axes(
                [0.0, side, 0.0],
                [0.0, 0.0, 1.0],
                [side, 0.0, 0.0],
                [side * width * 0.5, 0.0, z + LIFT],
            ),
        )?;
    }
    Ok(())
}

fn crown_placement(x: f64, z: f64) -> Placement3 {
    Placement3::from_axes(
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [x, 0.0, z + LIFT],
    )
}

/// A ridge-foot tile shares its lower contour with the terminal roof tiles.
/// Extrusion follows the final roof slope; checked cuts join the halves on the
/// ridge symmetry plane and give the fillet course one horizontal bearing bed.
pub(super) fn foot(section: &RoofSection, width: f64, end: bool) -> Result<Recipe> {
    let mut builder = RecipeBuilder::new();
    let material = builder.material_slot("surface");
    let course = *courses(section, 0.245, CAP_EAVE_PROJECTION).last().unwrap();
    let [n, c] = course.normal;
    let tail = 0.110;
    let top = (FOOT_TOP + 0.020 - tail * n) / c;
    let mut outline = bottom_contour(width, end);
    outline.extend([[width, top], [0.0, top]]);
    // Reversing the profile's normal coordinate keeps its placement right handed
    // while extruding uphill (+Y on the negative roof half).
    for point in &mut outline {
        point[1] = -point[1];
    }
    let profile = builder.add_profile(geometry::polygon(&outline)?);
    let half = builder
        .with_material(material)
        .add(NodeKind::ExtrudeToPlane {
            profile,
            placement: Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, -n, -c],
                [0.0, c, -n],
                [0.0, -tail * c, tail * n],
            ),
            plane: Plane3 {
                normal: [0.0, 1.0, 0.0],
                distance: 0.0,
            },
        })?;
    let half = builder.add(NodeKind::PlaneCut {
        child: half,
        plane: Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: FOOT_TOP,
        },
        side: PlaneSide::Negative,
        cap: CutCap {
            region: 100,
            material: Some(material),
        },
    })?;
    let other = builder.add(NodeKind::Mirror {
        child: half,
        plane: Plane3 {
            normal: [0.0, 1.0, 0.0],
            distance: 0.0,
        },
    })?;
    let root = builder.add(NodeKind::Group {
        children: vec![half, other],
    })?;
    Ok(builder.finish(root)?)
}

/// The exposed upper clay surfaces, including the vertical shoulder where a
/// cover ends above the pan. The shared polygon vertices make the contact
/// independent of differing arc tessellation on the two mating parts.
fn bottom_contour(width: f64, end: bool) -> Vec<[f64; 2]> {
    let cap: Vec<_> = terminal_arc(true, false)
        .into_iter()
        .map(|[x, z]| [x, z + CAP_LIFT])
        .collect();
    let mut contour: Vec<_> = cap.iter().copied().filter(|p| p[0] >= -1.0e-12).collect();
    contour[0][0] = 0.0;
    if end {
        contour.extend([[CAP_RADIUS, 0.0], [width, 0.0]]);
    } else {
        let pan: Vec<_> = terminal_arc(false, true)
            .into_iter()
            .map(|[x, z]| [x + width * 0.5, z + PAN_LIFT])
            .collect();
        contour.extend(clipped_chain(&pan, CAP_RADIUS, width - CAP_RADIUS));
        contour.extend(
            cap.iter()
                .copied()
                .filter(|p| p[0] <= 1.0e-12)
                .map(|[x, z]| [x + width, z]),
        );
        contour.last_mut().unwrap()[0] = width;
    }
    contour
}

fn clipped_chain(chain: &[[f64; 2]], left: f64, right: f64) -> Vec<[f64; 2]> {
    let at = |x: f64| {
        let pair = chain
            .windows(2)
            .find(|pair| pair[0][0] <= x && x <= pair[1][0])
            .unwrap();
        let t = (x - pair[0][0]) / (pair[1][0] - pair[0][0]);
        [x, pair[0][1] + t * (pair[1][1] - pair[0][1])]
    };
    let mut result = vec![at(left)];
    result.extend(
        chain
            .iter()
            .copied()
            .filter(|p| left < p[0] && p[0] < right),
    );
    result.push(at(right));
    result
}

/// Terminal pieces keep one section where the ridge-foot closure seats. The
/// lower end still nests with the ordinary tapered course beneath it.
pub(super) fn terminal_shell(cap: bool) -> Result<Recipe> {
    let mut outline = terminal_arc(cap, false);
    outline.extend(terminal_arc(cap, true).into_iter().rev());
    geometry::extrude(
        geometry::polygon(&outline)?,
        LENGTH,
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -1.0, 0.0],
            [0.0, LENGTH, 0.0],
        ),
    )
}

fn terminal_arc(cap: bool, inner: bool) -> Vec<[f64; 2]> {
    let radius = if cap { CAP_RADIUS } else { PAN_RADIUS };
    let half_angle = if cap {
        core::f64::consts::FRAC_PI_2
    } else {
        (PAN_HALF_WIDTH / radius).asin()
    };
    let r = radius - if inner { THICKNESS } else { 0.0 };
    // Shared facets stay below the export's sharp-normal threshold and have
    // at most 0.09 mm chord error, below the 1 mm export tolerance.
    let segments = if cap { 48 } else { 24 };
    (0..=segments)
        .map(|i| {
            let angle = -half_angle + 2.0 * half_angle * f64::from(i) / f64::from(segments);
            [
                r * angle.sin(),
                if cap {
                    r * angle.cos()
                } else {
                    -r * angle.cos()
                },
            ]
        })
        .collect()
}

/// A small weathering fillet leaves an unbroken flat bed under both crown feet.
fn fillet(width: f64) -> Result<Recipe> {
    geometry::extrude(
        geometry::polygon(&[
            [-FILLET_BASE_HALF_WIDTH, FOOT_TOP],
            [FILLET_BASE_HALF_WIDTH, FOOT_TOP],
            [FILLET_HALF_WIDTH, FOOT_TOP + 0.006],
            [FILLET_HALF_WIDTH, LIFT - 0.004],
            [FILLET_HALF_WIDTH - 0.004, LIFT],
            [-FILLET_HALF_WIDTH + 0.004, LIFT],
            [-FILLET_HALF_WIDTH, LIFT - 0.004],
            [-FILLET_HALF_WIDTH, FOOT_TOP + 0.006],
        ])?,
        width,
        Placement3::from_axes(
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ),
    )
}

fn end(radius: f64) -> Result<Recipe> {
    let outline = Loop2::new(vec![
        Seg2::arc((radius, 0.0), -1.0),
        Seg2::line((-radius, 0.0)),
    ])?;
    geometry::extrude(
        Profile2::simple(outline.reversed())?,
        END_THICKNESS,
        Placement3::IDENTITY,
    )
}
