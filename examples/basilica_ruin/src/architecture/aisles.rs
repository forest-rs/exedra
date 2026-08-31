// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::BuildContext;
use crate::geometry::{
    aisle_end_profile, aisle_roof_frame, box_recipe, extruded_profile_recipe, transverse_wall_frame,
};
use crate::{AisleSection, LevelSection, PlanSection};

pub(super) fn build(
    context: &mut BuildContext,
    plan: &PlanSection,
    levels: &LevelSection,
    aisle: &AisleSection,
) {
    let half_nave = plan.half_nave.as_meters();
    let half_total = plan.half_total.as_meters();
    // Low lean-to roofs attach each exterior arcade wall to the nave. Their
    // inner edges stay below the clerestory sills, keeping both tiers of
    // round-headed openings visible.
    let aisle_run = plan.aisle_run.as_meters();
    let aisle_inner_height = aisle.inner_height.as_meters();
    let aisle_roof_slope = aisle.slope_length.as_meters();
    let aisle_wall_top = levels.aisle_wall_top.as_meters();
    let length = plan.length.as_meters();
    let aisle_roof = context.add_part(
        "aisle-roof",
        box_recipe(
            [length, aisle_roof_slope, aisle.roof_depth.as_meters()],
            "basilica:aisle-roof",
        ),
        "aged-roof-tile",
    );
    context.add_instance(
        "aisle-roof-north",
        aisle_roof,
        aisle_roof_frame(plan, aisle, true),
        "aisle_roof",
    );
    context.add_instance(
        "aisle-roof-south",
        aisle_roof,
        aisle_roof_frame(plan, aisle, false),
        "aisle_roof",
    );
    let aisle_eave = context.add_part(
        "aisle-eave-fascia",
        box_recipe([length, 0.2, 0.34], "basilica:aisle-eave-fascia"),
        "aged-roof-tile",
    );
    context.add_instance(
        "aisle-eave-north",
        aisle_eave,
        Placement3::translate(0.0, half_total + 0.06, aisle_wall_top - 0.12),
        "aisle_eave",
    );
    context.add_instance(
        "aisle-eave-south",
        aisle_eave,
        Placement3::translate(0.0, -half_total - 0.26, aisle_wall_top - 0.12),
        "aisle_eave",
    );
    let aisle_end_north = context.add_part(
        "east-aisle-end-north",
        extruded_profile_recipe(
            aisle_end_profile(
                aisle_run,
                aisle.bearing_height.as_meters(),
                aisle_inner_height,
            ),
            0.5,
            transverse_wall_frame(),
            "basilica:east-aisle-end-north",
        ),
        "limestone",
    );
    context.add_instance(
        "east-aisle-end-north",
        aisle_end_north,
        Placement3::translate(plan.east_end.as_meters() - 0.5, half_nave, 0.0),
        "aisle_end",
    );
    let aisle_end_south = context.add_part(
        "east-aisle-end-south",
        extruded_profile_recipe(
            aisle_end_profile(
                aisle_run,
                aisle_inner_height,
                aisle.bearing_height.as_meters(),
            ),
            0.5,
            transverse_wall_frame(),
            "basilica:east-aisle-end-south",
        ),
        "limestone",
    );
    context.add_instance(
        "east-aisle-end-south",
        aisle_end_south,
        Placement3::translate(plan.east_end.as_meters() - 0.5, -half_total, 0.0),
        "aisle_end",
    );
}

#[cfg(test)]
mod tests {
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{BasilicaPremises, BasilicaSetout};

    #[test]
    fn lean_to_roofs_bear_on_aisle_walls_below_clerestory_sills() {
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default basilica resolves");
        let aisle = setout.aisle();
        let levels = setout.levels();
        let bearing_run = aisle.bearing_run.as_meters();
        let roof_thickness = aisle.roof_depth.as_meters();
        let clerestory_sill = levels.clerestory_sill.as_meters();
        let run = aisle.roof_run.as_meters();
        let drop = aisle.roof_drop.as_meters();
        let inner_height = aisle.inner_height.as_meters();
        let aisle_wall_top = levels.aisle_wall_top.as_meters();
        let angle = libm::atan2(drop, run);
        let bearing_underside = inner_height - drop * bearing_run / run;
        let inner_roof_top = inner_height + libm::cos(angle) * roof_thickness;

        assert!(run > bearing_run, "roof needs an exterior eave overhang");
        assert!(
            bearing_underside <= aisle_wall_top && aisle_wall_top - bearing_underside <= 0.1,
            "roof underside must overlap the aisle wall head: {bearing_underside} vs {}",
            aisle_wall_top
        );
        assert!(
            inner_roof_top < clerestory_sill,
            "complete roof build-up must stay below clerestory sill: {inner_roof_top}"
        );

        let scenario = build_scenario();
        for (wall_path, roof_path) in [
            ("aisle-wall-north", "aisle-roof-north"),
            ("aisle-wall-south", "aisle-roof-south"),
        ] {
            let (wall_min, wall_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, wall_path);
            let (roof_min, roof_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, roof_path);
            assert!((wall_max[2] - aisle_wall_top).abs() < 1.0e-5);
            assert!(
                roof_min[2] < wall_max[2] && roof_max[2] > wall_max[2],
                "{roof_path} must cross the {wall_path} head: {roof_min:?}..{roof_max:?} vs {wall_min:?}..{wall_max:?}"
            );
            assert!(
                roof_max[2] < clerestory_sill,
                "{roof_path} must remain below the clerestory sill: {roof_max:?}"
            );
        }
    }
}
