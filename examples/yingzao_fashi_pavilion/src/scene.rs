// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Explicit pavilion construction and shared-part placement.

use std::collections::HashMap;

use exedra_assembly::{Assembly, CompiledParts, Instance, InstanceId, PartId};
use exedra_constructive::edge_finish::RoundPolicy;
use exedra_constructive::ir::{Placement3, Recipe};
use joiner::compose;

use crate::{
    Result, brackets, geometry, joinery::FittedConstruction, layout::Layout, rafters,
    roof_section::RoofSection, seats, tiles,
};

pub(crate) struct PavilionScene {
    pub(crate) assembly: Assembly,
    roof_instances: HashMap<String, InstanceId>,
}

impl PavilionScene {
    pub(crate) fn fitted_instance(&self, key: &str) -> Option<InstanceId> {
        self.roof_instances.get(key).copied()
    }

    pub(crate) fn verify(
        &self,
        roof: &FittedConstruction,
        compiled: &CompiledParts,
    ) -> Result<f64> {
        roof.verify(&self.assembly, compiled, |key| self.fitted_instance(key))
    }
}

#[cfg(test)]
pub(crate) fn build(layout: &Layout) -> Result<Assembly> {
    Ok(build_with_roof(layout, &rafters::build(layout)?)?.assembly)
}

pub(crate) fn build_with_roof(layout: &Layout, roof: &FittedConstruction) -> Result<PavilionScene> {
    build_with_separation(layout, roof, 0.0)
}

/// Separate supports, purlins, rafter courses and pins; keep tiles as one layer.
pub(crate) fn build_with_separation(
    layout: &Layout,
    roof: &FittedConstruction,
    separation: f64,
) -> Result<PavilionScene> {
    let fitted = roof.lower()?;
    let mut scene = Scene::default();
    let roof_instances = scene.group("pavilion", Placement3::IDENTITY, |scene| {
        scene.group("foundation", Placement3::IDENTITY, |scene| {
            platform(scene, layout)
        })?;
        scene.group("timber", Placement3::IDENTITY, |scene| frame(scene, layout))?;
        scene.group(
            "roof",
            Placement3::translate(0.0, 0.0, separation * 0.5),
            |scene| roof_layers(scene, layout, roof, &fitted, separation),
        )
    })?;
    Ok(PavilionScene {
        assembly: scene.assembly,
        roof_instances,
    })
}

fn roof_layers(
    scene: &mut Scene,
    layout: &Layout,
    roof: &FittedConstruction,
    fitted: &Assembly,
    separation: f64,
) -> Result<HashMap<String, InstanceId>> {
    let mut instances = HashMap::new();
    for (name, role, lift) in [
        ("supports", "purlin-support", 0.0),
        ("purlins", "round-purlin", 1.0),
        ("pins", "wooden-pin", 2.6),
    ] {
        instances.extend(scene.fitted_group(
            fitted,
            name,
            Placement3::translate(
                if name == "pins" {
                    0.3 * separation
                } else {
                    0.0
                },
                0.0,
                lift * separation,
            ),
            |instance| {
                roof.construction
                    .element(instance.key())
                    .is_some_and(|e| e.role == role)
            },
        )?);
    }
    scene.group(
        "rafters",
        Placement3::translate(0.0, 0.0, 2.0 * separation),
        |scene| {
            for segment in 0..RoofSection::SEGMENTS {
                let segment = u32::try_from(segment)?;
                let suffix = format!("-{segment}");
                let offset = Placement3::translate(
                    if segment.is_multiple_of(2) {
                        -0.1 * separation
                    } else {
                        0.1 * separation
                    },
                    (f64::from(segment) - 3.5) * 0.14 * separation,
                    f64::from(segment % 2) * 0.15 * separation,
                );
                instances.extend(scene.fitted_group(
                    fitted,
                    &format!("course-{segment}"),
                    offset,
                    |instance| {
                        roof.construction
                            .element(instance.key())
                            .is_some_and(|e| e.role == "common-rafter")
                            && instance.key().ends_with(&suffix)
                    },
                )?);
            }
            Ok(())
        },
    )?;
    scene.group(
        "tiles",
        Placement3::translate(0.0, 0.0, 3.6 * separation),
        |scene| tiles::roof(scene, layout),
    )?;
    if instances.len() != fitted.instances().len() {
        return Err("roof grouping omitted fitted members".into());
    }
    Ok(instances)
}

pub(crate) fn bracket_study(layout: &Layout) -> Result<Assembly> {
    let mut scene = Scene::default();
    for fitted in brackets::fitted(layout)? {
        let part = scene.part(&fitted.key, fitted.recipe, "timber")?;
        for (name, x, explode) in [("assembled", -0.95, false), ("exploded", 0.95, true)] {
            let mut extent = fitted.extent.clone();
            extent.origin[0] += x;
            if explode {
                extent.origin[2] += match fitted.key.as_str() {
                    "lower-arm" => 0.20,
                    "upper-arm" => 0.55,
                    _ => 0.0,
                };
            }
            scene.orient(&format!("{name}-{}", fitted.key), part, extent.placement())?;
        }
    }
    Ok(scene.assembly)
}

pub(crate) fn seat_study(layout: &Layout) -> Result<Assembly> {
    let seats = seats::study(layout)?;
    let mut scene = Scene::default();
    let center = [0.0, -layout.roof[0][0], layout.bearing_height(0)];
    for key in ["roof-purlin-0--1", "eave-pad-0--1"] {
        let element = seats
            .construction
            .element(key)
            .ok_or("missing study part")?;
        let part = scene.part(
            key,
            compose(&seats.construction, element)?,
            if key.starts_with("roof") {
                "timber"
            } else {
                "timber.end"
            },
        )?;
        for (name, x, lift) in [("assembled", -0.43, 0.0), ("exploded", 0.43, 0.22)] {
            let z = if key.starts_with("roof") { lift } else { 0.0 };
            let extent = element
                .extent
                .translated([x - center[0], -center[1], z - center[2]]);
            scene.orient(&format!("{name}-{key}"), part, extent.placement())?;
        }
    }
    let footprint = seats.construction.contacts()[0]
        .footprint_meters()
        .ok_or("missing seat footprint")?;
    let marker = scene.part(
        "bearing-footprint",
        geometry::block([footprint[1], footprint[0], 0.0005])?,
        "contact",
    )?;
    scene.place(
        "bearing-footprint",
        marker,
        [0.43 - footprint[1] * 0.5, -footprint[0] * 0.5, 0.0],
    )?;
    Ok(scene.assembly)
}

pub(crate) fn concave_study() -> Result<Assembly> {
    let mut scene = Scene::default();
    for (name, x, finish) in [
        ("sharp", -0.42, None),
        ("chamfer", -0.10, Some(RoundPolicy::chamfer(0.02))),
        ("fillet", 0.22, Some(RoundPolicy::fillet(0.02))),
    ] {
        let part = scene.part(name, geometry::concave_shoulder(finish)?, "timber.end")?;
        scene.place(name, part, [x, 0.0, 0.0])?;
    }
    Ok(scene.assembly)
}

pub(crate) fn rafter_study(layout: &Layout) -> Result<Assembly> {
    let frame = rafters::study(layout)?;
    let mut scene = Scene::default();
    let center = [0.0, -layout.roof[1][0], layout.rafter_bearing_height(1)];
    for key in [
        "rafter-0-0",
        "rafter-0-1",
        "rafter-pin-0-1",
        "roof-purlin-0--1",
        "roof-purlin-1--1",
        "roof-purlin-2--1",
    ] {
        let element = frame
            .construction
            .element(key)
            .ok_or("missing roof study member")?;
        let part = scene.part(
            key,
            compose(&frame.construction, element)?,
            &element.material,
        )?;
        for (name, x, exploded) in [("assembled", -0.65, false), ("exploded", 0.65, true)] {
            let [dx, dz] = if exploded {
                match key {
                    "rafter-0-0" => [-0.13, 0.25],
                    "rafter-0-1" => [0.13, 0.25],
                    "rafter-pin-0-1" => [0.33, 0.25],
                    _ => [0.0, 0.0],
                }
            } else {
                [0.0, 0.0]
            };
            let placed = element
                .extent
                .translated([x + dx, -center[1], dz - center[2]]);
            scene.orient(&format!("{name}-{key}"), part, placed.placement())?;
        }
    }
    Ok(scene.assembly)
}

#[derive(Default)]
pub(crate) struct Scene {
    assembly: Assembly,
    parent: Option<InstanceId>,
}

impl Scene {
    fn group<T>(
        &mut self,
        key: &str,
        placement: Placement3,
        build: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let frame = self.assembly.add_frame(self.parent, key, placement)?;
        let previous = self.parent.replace(frame);
        let result = build(self);
        self.parent = previous;
        result
    }

    fn fitted_group(
        &mut self,
        source: &Assembly,
        key: &str,
        placement: Placement3,
        include: impl Fn(&Instance) -> bool,
    ) -> Result<HashMap<String, InstanceId>> {
        self.group(key, placement, |scene| {
            let map = scene.assembly.append_selected(
                scene.parent,
                source,
                "fitted",
                Placement3::IDENTITY,
                |_, instance| include(instance),
            )?;
            Ok(source
                .instances_with_ids()
                .filter_map(|(id, instance)| {
                    map.instance(id).map(|id| (instance.key().to_owned(), id))
                })
                .collect())
        })
    }

    pub(crate) fn part(&mut self, key: &str, recipe: Recipe, material: &str) -> Result<PartId> {
        let part = self.assembly.add_recipe_part(key, recipe)?;
        self.assembly.set_part_material(part, "surface", material)?;
        Ok(part)
    }
    pub(crate) fn place(&mut self, key: &str, part: PartId, origin: [f64; 3]) -> Result<()> {
        self.orient(
            key,
            part,
            Placement3::translate(origin[0], origin[1], origin[2]),
        )
    }
    pub(crate) fn orient(&mut self, key: &str, part: PartId, placement: Placement3) -> Result<()> {
        self.assembly
            .add_instance(self.parent, key, part, placement)?;
        Ok(())
    }
}

fn platform(scene: &mut Scene, l: &Layout) -> Result<()> {
    let width = l.width + 2.4;
    let depth = l.depth + 2.4;
    let ground = scene.part(
        "ground",
        geometry::block([width + 12.0, depth + 12.0, 0.15])?,
        "earth",
    )?;
    scene.place(
        "courtyard-ground",
        ground,
        [-width * 0.5 - 6.0, -depth * 0.5 - 6.0, -0.17],
    )?;
    let foundation = scene.part(
        "foundation",
        geometry::block([width, depth, 0.28])?,
        "stone",
    )?;
    scene.place(
        "platform-foundation",
        foundation,
        [-width * 0.5, -depth * 0.5, 0.0],
    )?;
    let edging = scene.part(
        "platform-cap",
        geometry::dressed_block([width + 0.12, depth + 0.12, 0.08])?,
        "stone.light",
    )?;
    scene.place(
        "platform-cap",
        edging,
        [-width * 0.5 - 0.06, -depth * 0.5 - 0.06, 0.28],
    )?;
    let step = scene.part(
        "step",
        geometry::dressed_block([2.0, 0.38, 0.12])?,
        "stone.light",
    )?;
    for i in 0..3 {
        scene.place(
            &format!("steps-{i}"),
            step,
            [
                -1.0,
                -depth * 0.5 - 0.38 * f64::from(3 - i),
                0.12 * f64::from(i),
            ],
        )?;
    }
    let slab = scene.part(
        "paving-slab",
        geometry::dressed_block([0.59, 0.59, 0.035])?,
        "stone.light",
    )?;
    for x in -2..2 {
        for y in 0..7 {
            scene.place(
                &format!("path-{x}-{y}"),
                slab,
                [
                    f64::from(x) * 0.61,
                    -depth * 0.5 - 1.3 - f64::from(y) * 0.61,
                    -0.005,
                ],
            )?;
        }
    }
    Ok(())
}

fn frame(scene: &mut Scene, l: &Layout) -> Result<()> {
    let column = scene.part(
        "column",
        geometry::cylinder(l.fen * 10.0, l.column_height)?,
        "timber",
    )?;
    let base = scene.part(
        "column-base",
        geometry::cylinder(0.24, 0.18)?,
        "stone.light",
    )?;
    let collar = scene.part(
        "column-collar",
        geometry::cylinder(0.165, 0.06)?,
        "timber.dark",
    )?;
    let mut bracket_parts = Vec::new();
    for fitted in brackets::fitted(l)? {
        let material = if fitted.key == "bearing-block" {
            "timber.end"
        } else {
            "timber"
        };
        let part = scene.part(&fitted.key, fitted.recipe, material)?;
        bracket_parts.push((fitted.key, part, fitted.extent));
    }
    let small_block = scene.part(
        "bracket-tip-block",
        geometry::block([0.21, 0.21, 0.12])?,
        "timber.end",
    )?;
    let plate = scene.part(
        "longitudinal-beam",
        geometry::block([l.span, 0.22, 0.24])?,
        "timber",
    )?;
    let top = 0.54 + l.column_height;
    let cross_top = l.bearing_height(1);
    for (i, x) in l.frames.iter().copied().enumerate() {
        for (side, y) in [("front", -l.depth * 0.5), ("back", l.depth * 0.5)] {
            let key = format!("frame-{i}-{side}");
            scene.place(&format!("{key}-base"), base, [x, y, 0.36])?;
            scene.place(&format!("{key}-column"), column, [x, y, 0.54])?;
            scene.place(&format!("{key}-collar"), collar, [x, y, top - 0.06])?;
            for (name, part, extent) in &bracket_parts {
                let mut placed = extent.clone();
                placed.origin[0] += x;
                placed.origin[1] += y;
                placed.origin[2] += top;
                scene.orient(&format!("{key}-{name}"), *part, placed.placement())?;
            }
            for (end, dx) in [(-1, -0.42), (1, 0.42)] {
                scene.place(
                    &format!("{key}-tip-{end}"),
                    small_block,
                    [x + dx - 0.105, y - 0.105, top + 0.375],
                )?;
            }
        }
    }
    for (i, pair) in l.frames.windows(2).enumerate() {
        for (side, y) in [("front", -l.depth * 0.5), ("back", l.depth * 0.5)] {
            scene.place(
                &format!("bay-{i}-{side}-beam"),
                plate,
                [pair[0], y - 0.11, top + 0.495],
            )?;
        }
    }
    // Successively shorter transverse beams and short posts carry the inner
    // purlins. The stack follows the evaluated roof datums at each frame.
    let mut previous_top = cross_top;
    for level in 2..=4 {
        let run = l.roof[level][0];
        let bearing = l.bearing_height(level);
        let beam_bottom = bearing - 0.10;
        let support_run = if level == 4 { 0.10 } else { run * 0.7 };
        let post = scene.part(
            &format!("roof-support-post-{level}"),
            geometry::block([0.18, 0.18, beam_bottom - previous_top])?,
            "timber.end",
        )?;
        for (i, x) in l.frames.iter().copied().enumerate() {
            for side in [-1.0, 1.0] {
                scene.place(
                    &format!("frame-{i}-stack-{level}-post-{side}"),
                    post,
                    [x - 0.09, side * support_run - 0.09, previous_top],
                )?;
            }
        }
        previous_top = bearing;
    }
    Ok(())
}
