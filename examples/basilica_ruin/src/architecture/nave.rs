// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, CLERESTORY_BASE, Layout};
use crate::geometry::{
    arcaded_wall_profile, box_recipe, extruded_profile_recipe, gable_profile, roof_panel_frame,
    transverse_wall_frame, vertical_wall_frame, west_facade_profile,
};
use crate::{BasilicaParams, names};

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let Layout {
        wall_thickness,
        half_nave,
        half_total,
        crossing_west,
        crossing_east,
    } = layout;
    let east_nave_length = p.length - crossing_east;
    let clerestory_height = p.nave_wall_height - CLERESTORY_BASE;
    let clerestory_sill = 6.55 - CLERESTORY_BASE;
    let clerestory_spring = 7.9 - CLERESTORY_BASE;
    // The nave walls terminate at the crossing rather than continuing behind
    // its pierced stage. This makes the crossing bay spatially open, not just
    // visually decorated with arch-shaped holes.
    let west_clerestory = arcaded_wall_profile(
        crossing_west,
        clerestory_height,
        5,
        1.8,
        clerestory_sill,
        clerestory_spring,
        None,
    );
    let ruined_west_clerestory = arcaded_wall_profile(
        crossing_west,
        clerestory_height,
        5,
        1.8,
        clerestory_sill,
        clerestory_spring,
        Some((7.25, 10.75, 1.7)),
    );
    let east_clerestory = arcaded_wall_profile(
        east_nave_length,
        clerestory_height,
        1,
        1.8,
        clerestory_sill,
        clerestory_spring,
        None,
    );
    let outer_arcade = arcaded_wall_profile(
        p.length,
        p.aisle_wall_height,
        p.arcade_bays,
        2.2,
        0.65,
        3.0,
        None,
    );

    let west_nave_wall = context.add_part(
        names::parts::NAVE_CLERESTORY_WEST,
        extruded_profile_recipe(
            west_clerestory,
            wall_thickness,
            vertical_wall_frame(),
            "basilica:nave-clerestory-west",
        ),
        "limestone",
    );
    let ruined_west_nave_wall = context.add_part(
        "nave-clerestory-west-ruined",
        extruded_profile_recipe(
            ruined_west_clerestory,
            wall_thickness,
            vertical_wall_frame(),
            "basilica:nave-clerestory-west-ruined",
        ),
        "weathered-limestone",
    );
    let east_nave_wall = context.add_part(
        names::parts::NAVE_CLERESTORY_EAST,
        extruded_profile_recipe(
            east_clerestory,
            wall_thickness,
            vertical_wall_frame(),
            "basilica:nave-clerestory-east",
        ),
        "limestone",
    );
    let aisle_wall = context.add_part(
        "aisle-arcade-wall",
        extruded_profile_recipe(
            outer_arcade,
            wall_thickness,
            vertical_wall_frame(),
            "basilica:aisle-arcade",
        ),
        "limestone",
    );

    context.add_instance(
        names::instances::NAVE_WALL_NORTH_WEST,
        west_nave_wall,
        Placement3::translate(0.0, half_nave, CLERESTORY_BASE),
        names::roles::NAVE_CLERESTORY,
    );
    context.add_instance(
        names::instances::NAVE_WALL_SOUTH_WEST_RUIN,
        ruined_west_nave_wall,
        Placement3::translate(0.0, -half_nave + wall_thickness, CLERESTORY_BASE),
        names::roles::NAVE_CLERESTORY_RUIN,
    );
    context.add_instance(
        names::instances::NAVE_WALL_NORTH_EAST,
        east_nave_wall,
        Placement3::translate(crossing_east, half_nave, CLERESTORY_BASE),
        names::roles::NAVE_CLERESTORY,
    );
    context.add_instance(
        "nave-wall-south-east",
        east_nave_wall,
        Placement3::translate(crossing_east, -half_nave + wall_thickness, CLERESTORY_BASE),
        names::roles::NAVE_CLERESTORY,
    );
    context.add_instance(
        "aisle-wall-north",
        aisle_wall,
        Placement3::translate(0.0, half_total, 0.0),
        "aisle_outer_arcade",
    );
    context.add_instance(
        "aisle-wall-south",
        aisle_wall,
        Placement3::translate(0.0, -half_total + wall_thickness, 0.0),
        "aisle_outer_arcade",
    );

    let facade_profile = west_facade_profile(p);
    let facade = context.add_part(
        "west-facade",
        extruded_profile_recipe(
            facade_profile,
            0.55,
            transverse_wall_frame(),
            "basilica:west-facade",
        ),
        "limestone",
    );
    context.add_instance(
        "west-facade",
        facade,
        Placement3::translate(-0.55, 0.0, 0.0),
        "west_front",
    );

    // Thin roof planes keep the arcades legible. The crossing is open
    // beneath the drum, and the south-west slope omits one bay aligned
    // with the authored clerestory break.
    let roof_run = half_nave + 0.35;
    let roof_slope = libm::sqrt(roof_run * roof_run + p.roof_rise * p.roof_rise);
    let roof_west = context.add_part(
        "nave-roof-west",
        box_recipe([crossing_west, roof_slope, 0.28], "basilica:nave-roof-west"),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-north-west",
        roof_west,
        roof_panel_frame(0.0, roof_run, p, true),
        "nave_roof_intact",
    );
    let broken_roof_west = context.add_part(
        "nave-roof-broken-west",
        box_recipe([7.15, roof_slope, 0.28], "basilica:nave-roof-broken-west"),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-south-west-a",
        broken_roof_west,
        roof_panel_frame(0.0, roof_run, p, false),
        "nave_roof_ruin",
    );
    let broken_roof_east = context.add_part(
        "nave-roof-broken-east",
        box_recipe(
            [crossing_west - 10.75, roof_slope, 0.28],
            "basilica:nave-roof-broken-east",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-south-west-b",
        broken_roof_east,
        roof_panel_frame(10.75, roof_run, p, false),
        "nave_roof_ruin",
    );
    let roof_east = context.add_part(
        "nave-roof-east",
        box_recipe(
            [p.length - crossing_east, roof_slope, 0.28],
            "basilica:nave-roof-east",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-north-east",
        roof_east,
        roof_panel_frame(crossing_east, roof_run, p, true),
        "nave_roof_intact",
    );
    context.add_instance(
        "nave-roof-south-east",
        roof_east,
        roof_panel_frame(crossing_east, roof_run, p, false),
        "nave_roof_intact",
    );

    let roof_shoulder = context.add_part(
        "crossing-roof-shoulder",
        extruded_profile_recipe(
            gable_profile(p.nave_width, p.roof_rise),
            0.5,
            transverse_wall_frame(),
            "basilica:crossing-roof-shoulder",
        ),
        "warm-stone",
    );
    for (key, x) in [
        ("crossing-shoulder-west", crossing_west - 0.25),
        ("crossing-shoulder-east", crossing_east - 0.25),
    ] {
        context.add_instance(
            key,
            roof_shoulder,
            Placement3::translate(x, -half_nave, p.nave_wall_height),
            "crossing_roof_shoulder",
        );
    }
}

#[cfg(test)]
mod tests {
    use exedra_constructive::ir::Placement3;

    use crate::output::{bounds_for_path, build_scenario};
    use crate::{BasilicaParams, names, resolve_instance_path};

    #[test]
    fn crossing_bay_is_open_between_named_nave_wall_segments() {
        let p = BasilicaParams::default();
        let crossing_west = p.crossing_x - p.drum_radius - 0.6;
        let crossing_east = p.crossing_x + p.drum_radius + 0.6;
        let scenario = build_scenario();

        for west_path in [
            names::instances::NAVE_WALL_NORTH_WEST,
            names::instances::NAVE_WALL_SOUTH_WEST_RUIN,
        ] {
            let (_, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, west_path);
            assert!(
                (max[0] - crossing_west).abs() < 1.0e-5,
                "{west_path} must terminate at the crossing: {max:?}"
            );
        }
        for east_path in [
            names::instances::NAVE_WALL_NORTH_EAST,
            "nave-wall-south-east",
        ] {
            let (min, _) = bounds_for_path(&scenario.compiled, &scenario.render_list, east_path);
            assert!(
                (min[0] - crossing_east).abs() < 1.0e-5,
                "{east_path} must begin after the crossing: {min:?}"
            );
        }

        let north_west =
            resolve_instance_path(&scenario.assembly, names::instances::NAVE_WALL_NORTH_WEST)
                .expect("named west clerestory resolves");
        assert_eq!(
            scenario.assembly.instance(north_west).unwrap().part(),
            scenario
                .assembly
                .part_by_key(names::parts::NAVE_CLERESTORY_WEST)
                .unwrap()
        );
        assert!(crossing_east - crossing_west > p.nave_width);
    }

    #[test]
    fn placed_nave_roof_slabs_are_mirrored_outward_from_the_same_baseline() {
        const ROOF_THICKNESS: f64 = 0.28;

        let p = BasilicaParams::default();
        let scenario = build_scenario();
        let run = p.nave_width * 0.5 + 0.35;
        let slope = libm::sqrt(run * run + p.roof_rise * p.roof_rise);
        let roof_sin = p.roof_rise / slope;
        let roof_cos = run / slope;
        let peak = p.nave_wall_height + p.roof_rise;
        let mut placed_bounds = Vec::new();

        for (path, normal_y) in [
            ("nave-roof-north-east", roof_sin),
            ("nave-roof-south-east", -roof_sin),
        ] {
            let item = scenario
                .render_list
                .items
                .iter()
                .find(|item| item.path.to_string() == path)
                .unwrap_or_else(|| panic!("missing placed roof {path}"));
            assert_close(rotation_determinant(&item.world), 1.0);
            let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
            let (min_distance, max_distance) = body
                .tri
                .positions
                .iter()
                .map(|&position| transform_point(&item.world, position))
                .map(|position| normal_y * position[1] + roof_cos * position[2] - roof_cos * peak)
                .fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(min_distance, max_distance), distance| {
                        (min_distance.min(distance), max_distance.max(distance))
                    },
                );
            assert_close(min_distance, 0.0);
            assert_close(max_distance, ROOF_THICKNESS);
            placed_bounds.push(bounds_for_path(
                &scenario.compiled,
                &scenario.render_list,
                path,
            ));
        }

        let (north_min, north_max) = placed_bounds[0];
        let (south_min, south_max) = placed_bounds[1];
        assert_close(north_min[0], south_min[0]);
        assert_close(north_max[0], south_max[0]);
        assert_close(north_min[1], -south_max[1]);
        assert_close(north_max[1], -south_min[1]);
        assert_close(north_min[2], south_min[2]);
        assert_close(north_max[2], south_max[2]);
        assert!(north_min[2] <= p.nave_wall_height + 1.0e-5);
        assert!(south_min[2] <= p.nave_wall_height + 1.0e-5);
    }

    fn transform_point(placement: &Placement3, point: [f32; 3]) -> [f64; 3] {
        let point = point.map(f64::from);
        let rows = &placement.rows;
        [
            rows[0][0] * point[0] + rows[0][1] * point[1] + rows[0][2] * point[2] + rows[0][3],
            rows[1][0] * point[0] + rows[1][1] * point[1] + rows[1][2] * point[2] + rows[1][3],
            rows[2][0] * point[0] + rows[2][1] * point[1] + rows[2][2] * point[2] + rows[2][3],
        ]
    }

    fn rotation_determinant(placement: &Placement3) -> f64 {
        let r = &placement.rows;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
    }
}
