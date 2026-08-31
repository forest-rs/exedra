// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;
use exedra_constructive::profile::Profile2;

use super::BuildContext;
use crate::geometry::{arcaded_wall_profile, extruded_profile_recipe, vertical_wall_frame};
use crate::{LevelSection, PlanSection, names};

const WEST_BAYS: u32 = 5;
const EAST_BAYS: u32 = 1;

/// Adds the lower pierced walls that carry the clerestory and connect the
/// central nave spatially to both side aisles.
///
/// These are masonry arcade segments with genuine round-headed voids, not a
/// decorative row of columns masking a solid nave wall. Their west/east split
/// repeats the upper-wall split so the full crossing bay stays open.
pub(super) fn build(context: &mut BuildContext, plan: &PlanSection, levels: &LevelSection) {
    let wall_thickness = plan.wall_thickness.as_meters();
    let half_nave = plan.half_nave.as_meters();
    let crossing_west = plan.crossing_west.as_meters();
    let crossing_east = plan.crossing_east.as_meters();
    let east_length = plan.east_nave_length.as_meters();
    let clerestory_base = levels.clerestory_base.as_meters();

    let west = context.add_part(
        names::parts::INTERIOR_ARCADE_WEST,
        extruded_profile_recipe(
            arcade_profile(crossing_west, clerestory_base, WEST_BAYS),
            wall_thickness,
            vertical_wall_frame(),
            "basilica:interior-arcade-west",
        ),
        "warm-stone",
    );
    let east = context.add_part(
        names::parts::INTERIOR_ARCADE_EAST,
        extruded_profile_recipe(
            arcade_profile(east_length, clerestory_base, EAST_BAYS),
            wall_thickness,
            vertical_wall_frame(),
            "basilica:interior-arcade-east",
        ),
        "warm-stone",
    );

    for (key, part, x, y) in [
        (
            names::instances::INTERIOR_ARCADE_NORTH_WEST,
            west,
            0.0,
            half_nave,
        ),
        (
            names::instances::INTERIOR_ARCADE_SOUTH_WEST,
            west,
            0.0,
            -half_nave + wall_thickness,
        ),
        (
            names::instances::INTERIOR_ARCADE_NORTH_EAST,
            east,
            crossing_east,
            half_nave,
        ),
        (
            names::instances::INTERIOR_ARCADE_SOUTH_EAST,
            east,
            crossing_east,
            -half_nave + wall_thickness,
        ),
    ] {
        context.add_instance(
            key,
            part,
            Placement3::translate(x, y, 0.0),
            names::roles::INTERIOR_ARCADE,
        );
    }
}

fn arcade_profile(length: f64, height: f64, bays: u32) -> Profile2 {
    arcaded_wall_profile(length, height, bays, 2.9, 0.02, 3.65, None)
}

#[cfg(test)]
mod tests {
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{
        BasilicaPremises, BasilicaSetout, instances_with_role, names, resolve_instance_path,
    };

    use super::{EAST_BAYS, WEST_BAYS, arcade_profile};

    #[test]
    fn profiles_have_real_round_head_voids() {
        // Profile topology must remain independent of the exact dimension
        // source supplied by the setout layer.
        assert_eq!(arcade_profile(21.3, 5.75, WEST_BAYS).holes().len(), 5);
        assert_eq!(arcade_profile(5.3, 5.75, EAST_BAYS).holes().len(), 1);
    }

    #[test]
    fn named_segments_keep_the_crossing_clear() {
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default basilica resolves");
        let crossing_west = setout.plan().crossing_west.as_meters();
        let crossing_east = setout.plan().crossing_east.as_meters();
        let scenario = build_scenario();

        let arcades = instances_with_role(&scenario.assembly, names::roles::INTERIOR_ARCADE);
        assert_eq!(arcades.len(), 4);

        for path in [
            names::instances::INTERIOR_ARCADE_NORTH_WEST,
            names::instances::INTERIOR_ARCADE_SOUTH_WEST,
        ] {
            let (_, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, path);
            assert!((max[0] - crossing_west).abs() < 1.0e-5, "{path}: {max:?}");
        }
        for path in [
            names::instances::INTERIOR_ARCADE_NORTH_EAST,
            names::instances::INTERIOR_ARCADE_SOUTH_EAST,
        ] {
            let (min, _) = bounds_for_path(&scenario.compiled, &scenario.render_list, path);
            assert!((min[0] - crossing_east).abs() < 1.0e-5, "{path}: {min:?}");
        }

        let north_west = resolve_instance_path(
            &scenario.assembly,
            names::instances::INTERIOR_ARCADE_NORTH_WEST,
        )
        .expect("named interior arcade resolves");
        assert_eq!(
            scenario.assembly.instance(north_west).unwrap().part(),
            scenario
                .assembly
                .part_by_key(names::parts::INTERIOR_ARCADE_WEST)
                .unwrap()
        );
    }

    #[test]
    fn arcade_clerestory_and_aisle_roof_share_a_bearing_band() {
        let setout =
            BasilicaSetout::new(&BasilicaPremises::default()).expect("default basilica resolves");
        let clerestory_base = setout.levels().clerestory_base.as_meters();
        let clerestory_sill = setout.levels().clerestory_sill.as_meters();
        let scenario = build_scenario();
        let (_, arcade_max) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::INTERIOR_ARCADE_NORTH_WEST,
        );
        let (clerestory_min, _) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::NAVE_WALL_NORTH_WEST,
        );
        let (_, roof_max) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            "aisle-roof-north",
        );

        // All three independently built systems must meet the same resolved
        // vertical band rather than restating its numeric coordinates.
        assert!((arcade_max[2] - clerestory_base).abs() < 1.0e-5);
        assert!((clerestory_min[2] - clerestory_base).abs() < 1.0e-5);
        assert!(
            roof_max[2] > clerestory_base && roof_max[2] < clerestory_sill,
            "aisle roof must meet the solid band below the clerestory sill: {roof_max:?}"
        );
    }
}
