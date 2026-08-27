// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, CLERESTORY_BASE, Layout};
use crate::geometry::{
    arcaded_wall_profile, box_recipe, extruded_profile_recipe, gable_profile, roof_panel_frame,
    transverse_wall_frame, vertical_wall_frame, west_facade_profile,
};
use crate::{BasilicaParams, RoofSection, names};

pub(super) fn build(
    context: &mut BuildContext,
    p: &BasilicaParams,
    layout: Layout,
    roof: &RoofSection,
) {
    let Layout {
        wall_thickness,
        half_total,
        crossing_west,
        crossing_east,
        ..
    } = layout;
    let half_nave = roof.half_span.as_metres();
    let wall_head = roof.wall_head.as_metres();
    let east_nave_length = p.length - crossing_east;
    let clerestory_height = wall_head - CLERESTORY_BASE;
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

    // The wall plate is the physical bearing datum that the setting-out
    // network resolves. It is continuous where the roof survives and repeats
    // the authored south-west ruin gap rather than bridging lost fabric.
    let plate_width = roof.wall_plate_width.as_metres();
    let plate_height = roof.wall_plate_height.as_metres();
    let plate_y = |north: bool| {
        let centerline = if north { half_nave } else { -half_nave };
        centerline - plate_width * 0.5
    };
    let west_plate = context.add_part(
        names::parts::NAVE_WALL_PLATE_WEST,
        box_recipe(
            [crossing_west, plate_width, plate_height],
            "basilica:nave-wall-plate-west",
        ),
        "aged-timber",
    );
    context.add_instance(
        names::instances::NAVE_WALL_PLATE_NORTH_WEST,
        west_plate,
        Placement3::translate(0.0, plate_y(true), wall_head),
        names::roles::NAVE_WALL_PLATE,
    );
    let ruined_plate_a = context.add_part(
        names::parts::NAVE_WALL_PLATE_RUIN_A,
        box_recipe(
            [7.15, plate_width, plate_height],
            "basilica:nave-wall-plate-ruin-a",
        ),
        "aged-timber",
    );
    context.add_instance(
        names::instances::NAVE_WALL_PLATE_SOUTH_WEST_A,
        ruined_plate_a,
        Placement3::translate(0.0, plate_y(false), wall_head),
        names::roles::NAVE_WALL_PLATE,
    );
    let ruined_plate_b = context.add_part(
        names::parts::NAVE_WALL_PLATE_RUIN_B,
        box_recipe(
            [crossing_west - 10.75, plate_width, plate_height],
            "basilica:nave-wall-plate-ruin-b",
        ),
        "aged-timber",
    );
    context.add_instance(
        names::instances::NAVE_WALL_PLATE_SOUTH_WEST_B,
        ruined_plate_b,
        Placement3::translate(10.75, plate_y(false), wall_head),
        names::roles::NAVE_WALL_PLATE,
    );
    let east_plate = context.add_part(
        names::parts::NAVE_WALL_PLATE_EAST,
        box_recipe(
            [east_nave_length, plate_width, plate_height],
            "basilica:nave-wall-plate-east",
        ),
        "aged-timber",
    );
    for (key, north) in [
        (names::instances::NAVE_WALL_PLATE_NORTH_EAST, true),
        (names::instances::NAVE_WALL_PLATE_SOUTH_EAST, false),
    ] {
        context.add_instance(
            key,
            east_plate,
            Placement3::translate(crossing_east, plate_y(north), wall_head),
            names::roles::NAVE_WALL_PLATE,
        );
    }
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

    let facade_profile = west_facade_profile(p, roof);
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
    let roof_slope = roof.roof_slope_length.as_metres();
    let roof_depth = roof.roof_skin_depth.as_metres();
    let roof_west = context.add_part(
        "nave-roof-west",
        box_recipe(
            [crossing_west, roof_slope, roof_depth],
            "basilica:nave-roof-west",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-north-west",
        roof_west,
        roof_panel_frame(0.0, roof, true),
        "nave_roof_intact",
    );
    let broken_roof_west = context.add_part(
        "nave-roof-broken-west",
        box_recipe(
            [7.15, roof_slope, roof_depth],
            "basilica:nave-roof-broken-west",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-south-west-a",
        broken_roof_west,
        roof_panel_frame(0.0, roof, false),
        "nave_roof_ruin",
    );
    let broken_roof_east = context.add_part(
        "nave-roof-broken-east",
        box_recipe(
            [crossing_west - 10.75, roof_slope, roof_depth],
            "basilica:nave-roof-broken-east",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-south-west-b",
        broken_roof_east,
        roof_panel_frame(10.75, roof, false),
        "nave_roof_ruin",
    );
    let roof_east = context.add_part(
        "nave-roof-east",
        box_recipe(
            [p.length - crossing_east, roof_slope, roof_depth],
            "basilica:nave-roof-east",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "nave-roof-north-east",
        roof_east,
        roof_panel_frame(crossing_east, roof, true),
        "nave_roof_intact",
    );
    context.add_instance(
        "nave-roof-south-east",
        roof_east,
        roof_panel_frame(crossing_east, roof, false),
        "nave_roof_intact",
    );

    let roof_shoulder = context.add_part(
        "crossing-roof-shoulder",
        extruded_profile_recipe(
            gable_profile(roof.span.as_metres(), roof.rise.as_metres()),
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
            Placement3::translate(x, -half_nave, roof.wall_plate_top.as_metres()),
            "crossing_roof_shoulder",
        );
    }
}

#[cfg(test)]
mod tests {
    use exedra_constructive::ir::Placement3;

    use crate::output::{bounds_for_path, build_scenario};
    use crate::{
        BasilicaParams, BasilicaRoofSetout, RoofSide, instance_id_at, names, role_instances,
    };

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

        let north_west = instance_id_at(&scenario.assembly, names::instances::NAVE_WALL_NORTH_WEST)
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
    fn wall_plates_materialize_the_roof_bearing_datum_and_preserve_the_ruin() {
        let p = BasilicaParams::default();
        let setout = BasilicaRoofSetout::new(&p).expect("default roof resolves");
        let roof = setout.section();
        let scenario = build_scenario();
        let plates = role_instances(&scenario.assembly, names::roles::NAVE_WALL_PLATE);
        assert_eq!(plates.len(), 5);

        for (path, side) in [
            (names::instances::NAVE_WALL_PLATE_NORTH_WEST, 1.0),
            (names::instances::NAVE_WALL_PLATE_NORTH_EAST, 1.0),
            (names::instances::NAVE_WALL_PLATE_SOUTH_WEST_A, -1.0),
            (names::instances::NAVE_WALL_PLATE_SOUTH_WEST_B, -1.0),
            (names::instances::NAVE_WALL_PLATE_SOUTH_EAST, -1.0),
        ] {
            let (min, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, path);
            // The timber is centered on the exact wall/rafter seat line and
            // its top is the datum through which the roof underside passes.
            assert_close(max[2], roof.wall_plate_top.as_metres());
            assert_close((min[1] + max[1]) * 0.5, side * roof.half_span.as_metres());
        }

        let (_, lost_west_end) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::NAVE_WALL_PLATE_SOUTH_WEST_A,
        );
        let (lost_east_start, _) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::NAVE_WALL_PLATE_SOUTH_WEST_B,
        );
        assert_close(lost_west_end[0], 7.15);
        assert_close(lost_east_start[0], 10.75);
    }

    #[test]
    fn placed_nave_roof_slabs_are_mirrored_outward_from_the_same_baseline() {
        let p = BasilicaParams::default();
        let setout = BasilicaRoofSetout::new(&p).expect("default roof resolves");
        let roof = setout.section();
        let roof_thickness = roof.roof_skin_depth.as_metres();
        let scenario = build_scenario();
        let mut placed_bounds = Vec::new();

        for (path, side) in [
            ("nave-roof-north-east", RoofSide::North),
            ("nave-roof-south-east", RoofSide::South),
        ] {
            let item = scenario
                .render_list
                .items
                .iter()
                .find(|item| item.address.to_string().trim_start_matches('/') == path)
                .unwrap_or_else(|| panic!("missing placed roof {path}"));
            assert_close(rotation_determinant(&item.world), 1.0);
            let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
            let normal = roof.outward_normal(side);
            let ridge = setout_joiner::lower_point(roof.ridge_point);
            let (min_distance, max_distance) = body
                .tri
                .positions
                .iter()
                .map(|&position| transform_point(&item.world, position))
                .map(|position| {
                    normal[0] * (position[0] - ridge[0])
                        + normal[1] * (position[1] - ridge[1])
                        + normal[2] * (position[2] - ridge[2])
                })
                .fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(min_distance, max_distance), distance| {
                        (min_distance.min(distance), max_distance.max(distance))
                    },
                );
            assert_close(min_distance, 0.0);
            assert_close(max_distance, roof_thickness);
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
        // The eaves extend below the wall-plate top only after passing exactly
        // through it; this is a true overhang, not a roof/wall mismatch.
        assert!(north_min[2] <= roof.wall_head.as_metres() + 1.0e-5);
        assert!(south_min[2] <= roof.wall_head.as_metres() + 1.0e-5);
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
