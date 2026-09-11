// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::{
    Result,
    drainage::{CHANNEL_WIDTH, CHANNEL_X},
    geometry,
    scene::Scene,
    wall_caps::{self, Ends},
};
use exedra_assembly::compose;
use exedra_constructive::ir::Placement3;
use joiner::{
    Construction, Element, Evidence, EvidenceClass, EvidenceSource, OrientedBox, Relation,
    RelationKind, lower_shared,
};
use joiner_masonry::{CircularOpening, Length, RunningBondParams, RunningBondRule};

pub(crate) fn mm(value: u64) -> Length {
    Length::millimeters(value).expect("positive authored millimeters")
}

pub(crate) fn gate(scene: &mut Scene, radius_mm: u64) -> Result<Construction> {
    if !(1350..=1700).contains(&radius_mm) {
        return Err("gate radius must be 1350–1700 mm".into());
    }
    let mut c = Construction::new();
    let evidence = Evidence::new("courtyard-study", EvidenceClass::ModernEngineeringInference);
    c.add_evidence_source(EvidenceSource::new(
        "courtyard-study",
        evidence.class,
        "local:moon-gate-courtyard",
        "Authored garden composition and masonry detail; not a historical reconstruction",
    ))?;
    let radius = Length::millimeters(radius_mm).ok_or("gate radius must be positive")?;
    let center = [5.5, radius.as_meters() + 0.4];
    c.add_element(
        Element::new(
            "gate-wall",
            "masonry-wall",
            "brick.0",
            OrientedBox::axis_aligned([-5.5, 0.0, -0.6], [11.0, 0.42, 4.2]),
            evidence.clone(),
        )
        .without_part(),
    )?;
    c.add_relation(Relation::new(
        "gate-bond",
        RelationKind::element_units("gate-wall"),
        "running-bond-with-circular-surround",
        evidence,
    ))?;
    c.apply_rule(
        "lay-gate-wall",
        "gate-bond",
        &RunningBondRule,
        &RunningBondParams {
            unit_length: mm(240),
            unit_height: mm(65),
            joint: mm(9),
            minimum_closure: mm(25),
            opening: Some(CircularOpening {
                center,
                radius,
                surround: mm(240),
                segments: 72,
            }),
        },
    )?;
    crate::drainage::outlets(&mut c)?;
    let mut masonry = lower_shared(&c, |e| e.role.clone())?;
    let materials: Vec<_> = masonry
        .instances_with_ids()
        .map(|(id, instance)| {
            let surround = instance
                .metadata()
                .iter()
                .any(|(k, v)| k == "role" && v == "masonry-surround");
            let tone = instance
                .key()
                .bytes()
                .fold(0_u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)))
                % 5;
            (
                id,
                format!("{}.{tone}", if surround { "surround" } else { "brick" }),
            )
        })
        .collect();
    for (id, material) in materials {
        masonry.bind_material(id, "surface", &material)?;
    }
    scene
        .assembly
        .append(scene.parent, &masonry, "entry", Placement3::IDENTITY)?;
    let radius = radius.as_meters() + 0.255;
    let plaster = scene.recipe(
        "gate-plaster",
        geometry::pierced_panel(
            [11.0, 0.022, 3.20],
            [center[0], center[1] - 0.6 - 0.4],
            radius,
        )?,
        "plaster",
    )?;
    scene.at(plaster, [-5.5, -0.025, 0.4])?;
    scene.at(plaster, [-5.5, 0.423, 0.4])?;
    wall_caps::build(
        scene,
        "entry-cap",
        11.0,
        Placement3::translate(-5.5, 0.21, 3.6),
        Ends::SQUARE,
    )?;
    Ok(c)
}
pub(crate) fn enclosure(scene: &mut Scene) -> Result<()> {
    for (key, length, frame, ends) in [
        (
            "east-wall",
            9.5,
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, 5.5, 0.42, 0.0),
            Ends {
                start: 0.0,
                end: -1.0,
            },
        ),
        (
            "west-wall",
            9.5,
            Placement3::rotate_z_then_translate(core::f64::consts::FRAC_PI_2, -5.18, 0.42, 0.0),
            Ends {
                start: 0.0,
                end: 1.0,
            },
        ),
        (
            "rear-wall",
            11.0,
            Placement3::translate(-5.5, 9.6, 0.0),
            Ends {
                start: -1.0,
                end: 1.0,
            },
        ),
    ] {
        let recipe = if key == "rear-wall" {
            geometry::pierced_panel([length, 0.32, 2.8], [8.1, 1.6], 0.72)?
        } else {
            geometry::block([length, 0.32, 2.8], 0.015)?
        };
        let solid = scene.recipe(key, recipe, "plaster")?;
        scene.place(solid, frame, None)?;
        let base = scene.recipe(
            &format!("{key}-base"),
            geometry::block([length, 0.35, 0.32], 0.006)?,
            "brick.2",
        )?;
        scene.place(
            base,
            compose(&frame, &Placement3::translate(0.0, -0.015, -0.04)),
            None,
        )?;
        let (cap_length, along) = if key == "rear-wall" {
            (10.68, 0.16)
        } else {
            (9.31, 0.03)
        };
        wall_caps::build(
            scene,
            &format!("{key}-cap"),
            cap_length,
            compose(&frame, &Placement3::translate(along, 0.16, 2.8)),
            ends,
        )?;
    }
    // A second framed view and geometric timber screen against the rear wall.
    let panel = scene.recipe(
        "screen-surround",
        geometry::pierced_panel([2.0, 0.1, 1.8], [1.0, 0.9], 0.72)?,
        "stone",
    )?;
    scene.at(panel, [1.6, 9.45, 0.7])?;
    for i in -3..=3 {
        let offset = f64::from(i) * 0.19;
        let half = (0.72_f64.powi(2) - (offset.abs() + 0.016).powi(2)).sqrt();
        let vertical = scene.recipe(
            &format!("screen-vertical-{i}"),
            geometry::block([0.032, 0.04, 2.0 * half], 0.001)?,
            "timber.dark",
        )?;
        scene.at(vertical, [2.584 + offset, 9.54, 1.6 - half])?;
        let horizontal = scene.recipe(
            &format!("screen-horizontal-{i}"),
            geometry::block([2.0 * half, 0.04, 0.032], 0.001)?,
            "timber.dark",
        )?;
        scene.at(horizontal, [2.6 - half, 9.585, 1.584 + offset])?;
    }
    Ok(())
}
pub(crate) fn ground(scene: &mut Scene) -> Result<()> {
    scene.block(
        "site",
        [18.0, 20.0, 0.11],
        [-9.0, -5.0, -0.35],
        "earth",
        0.0,
    )?;
    // Two open channels collect crossfall before it reaches a raised bed or
    // enclosing wall. Their longitudinal fall leads through small outlets in the front wall.
    pave(scene, "west", [-5.5, 0.31], [-3.6, 9.6], -0.01, 0.4)?;
    pave(scene, "east", [4.97, 5.5], [-3.6, 9.6], 0.01, 4.87)?;
    for (name, y) in [("approach", [-3.6, 2.55]), ("rear", [7.42, 9.6])] {
        pave(scene, &format!("{name}-west"), [0.50, 2.64], y, 0.01, 0.4)?;
        pave(scene, &format!("{name}-east"), [2.64, 4.78], y, -0.01, 4.87)?;
    }
    scene.block(
        "planting-soil",
        [3.95, 4.65, 0.16],
        [0.65, 2.65, -0.16],
        "earth",
        0.0,
    )?;
    let curb = scene.recipe(
        "bed-curb",
        geometry::block([0.56, 0.12, 0.22], 0.007)?,
        "stone",
    )?;
    for (y, start, end) in [(2.55, 0.55, 4.7), (7.3, 0.55, 4.7)] {
        for i in 0..7 {
            scene.at(curb, [start + f64::from(i) * (end - start) / 7.0, y, -0.09])?;
        }
    }
    for x in [0.55, 4.58] {
        for i in 0..8 {
            scene.place(
                curb,
                Placement3::rotate_z_then_translate(
                    core::f64::consts::FRAC_PI_2,
                    x + 0.12,
                    2.68 + f64::from(i) * 0.58,
                    -0.09,
                ),
                None,
            )?;
        }
    }
    let channel = scene.recipe(
        "drain-channel",
        geometry::block([CHANNEL_WIDTH, 13.2, 0.025], 0.0)?,
        "drain",
    )?;
    let grate = scene.recipe(
        "drain-grate",
        geometry::block([CHANNEL_WIDTH, 0.022, 0.025], 0.002)?,
        "metal",
    )?;
    for x in CHANNEL_X {
        scene.place(
            channel,
            Placement3::from_axes(
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.003],
                [0.0, 0.0, 1.0],
                [x, -3.6, -0.211],
            ),
            None,
        )?;
        for i in 0..220 {
            let y = -3.6 + f64::from(i) * 0.06;
            scene.at(grate, [x, y, -0.125 + 0.003 * y])?;
        }
    }
    scene.block(
        "garden-bench",
        [1.7, 0.46, 0.13],
        [3.45, 7.8, 0.43],
        "stone",
        0.02,
    )?;
    let foot = scene.recipe(
        "bench-foot",
        geometry::block([0.23, 0.35, 0.45], 0.01)?,
        "stone",
    )?;
    scene.at(foot, [3.64, 7.86, -0.01])?;
    scene.at(foot, [4.72, 7.86, -0.01])?;
    Ok(())
}

fn intervals(length: f64, maximum: f64) -> u32 {
    let mut count = 1;
    while f64::from(count) * maximum < length {
        count += 1;
    }
    count
}

fn pave(
    scene: &mut Scene,
    key: &str,
    x: [f64; 2],
    y: [f64; 2],
    crossfall: f64,
    drain_x: f64,
) -> Result<()> {
    let columns = intervals(x[1] - x[0], 0.60);
    let rows = intervals(y[1] - y[0], 0.40);
    let width = (x[1] - x[0]) / f64::from(columns);
    let length = (y[1] - y[0]) / f64::from(rows);
    let part = scene.recipe(
        &format!("paving-{key}"),
        geometry::block([width - 0.006, length - 0.006, 0.075], 0.003)?,
        "paving.0",
    )?;
    for row in 0..rows {
        for column in 0..columns {
            let px = x[0] + f64::from(column) * width + 0.003;
            let py = y[0] + f64::from(row) * length + 0.003;
            let z = -0.175 + crossfall * (px - drain_x) + 0.003 * py;
            let material = format!("paving.{}", (column * 13 + row * 7) % 5);
            scene.place(
                part,
                Placement3::from_axes(
                    [1.0, 0.0, crossfall],
                    [0.0, 1.0, 0.003],
                    [0.0, 0.0, 1.0],
                    [px, py, z],
                ),
                Some(&material),
            )?;
        }
    }
    Ok(())
}
