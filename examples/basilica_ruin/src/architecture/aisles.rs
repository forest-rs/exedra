// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::BasilicaParams;
use crate::geometry::{
    aisle_end_profile, aisle_roof_frame, aisle_roof_section, box_recipe, extruded_profile_recipe,
    transverse_wall_frame,
};

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let Layout {
        half_nave,
        half_total,
        ..
    } = layout;
    // Low lean-to roofs attach each exterior arcade wall to the nave. Their
    // inner edges stay below the clerestory sills, keeping both tiers of
    // round-headed openings visible.
    let aisle_run = half_total - half_nave;
    let (aisle_roof_run, aisle_roof_drop, aisle_inner_height) = aisle_roof_section(p);
    let aisle_roof_slope =
        libm::sqrt(aisle_roof_run * aisle_roof_run + aisle_roof_drop * aisle_roof_drop);
    let aisle_roof = context.add_part(
        "aisle-roof",
        box_recipe([p.length, aisle_roof_slope, 0.22], "basilica:aisle-roof"),
        "aged-roof-tile",
    );
    context.add_instance(
        "aisle-roof-north",
        aisle_roof,
        aisle_roof_frame(p, aisle_roof_run, aisle_roof_drop, aisle_inner_height, true),
        "aisle_roof",
    );
    context.add_instance(
        "aisle-roof-south",
        aisle_roof,
        aisle_roof_frame(
            p,
            aisle_roof_run,
            aisle_roof_drop,
            aisle_inner_height,
            false,
        ),
        "aisle_roof",
    );
    let aisle_eave = context.add_part(
        "aisle-eave-fascia",
        box_recipe([p.length, 0.2, 0.34], "basilica:aisle-eave-fascia"),
        "aged-roof-tile",
    );
    context.add_instance(
        "aisle-eave-north",
        aisle_eave,
        Placement3::translate(0.0, half_total + 0.06, p.aisle_wall_height - 0.12),
        "aisle_eave",
    );
    context.add_instance(
        "aisle-eave-south",
        aisle_eave,
        Placement3::translate(0.0, -half_total - 0.26, p.aisle_wall_height - 0.12),
        "aisle_eave",
    );
    let aisle_end_north = context.add_part(
        "east-aisle-end-north",
        extruded_profile_recipe(
            aisle_end_profile(aisle_run, p.aisle_wall_height - 0.08, aisle_inner_height),
            0.5,
            transverse_wall_frame(),
            "basilica:east-aisle-end-north",
        ),
        "limestone",
    );
    context.add_instance(
        "east-aisle-end-north",
        aisle_end_north,
        Placement3::translate(p.length - 0.5, half_nave, 0.0),
        "aisle_end",
    );
    let aisle_end_south = context.add_part(
        "east-aisle-end-south",
        extruded_profile_recipe(
            aisle_end_profile(aisle_run, aisle_inner_height, p.aisle_wall_height - 0.08),
            0.5,
            transverse_wall_frame(),
            "basilica:east-aisle-end-south",
        ),
        "limestone",
    );
    context.add_instance(
        "east-aisle-end-south",
        aisle_end_south,
        Placement3::translate(p.length - 0.5, -half_total, 0.0),
        "aisle_end",
    );
}

#[cfg(test)]
mod tests {
    use crate::BasilicaParams;
    use crate::geometry::aisle_roof_section;
    use crate::output::{bounds_for_path, build_scenario};

    #[test]
    fn lean_to_roofs_bear_on_aisle_walls_below_clerestory_sills() {
        let p = BasilicaParams::default();
        let half_nave = p.nave_width * 0.5;
        let half_total = p.total_width * 0.5;
        let bearing_run = half_total - half_nave;
        let roof_thickness = 0.22;
        let clerestory_sill = 6.0;
        let (run, drop, inner_height) = aisle_roof_section(&p);
        let angle = libm::atan2(drop, run);
        let bearing_underside = inner_height - drop * bearing_run / run;
        let inner_roof_top = inner_height + libm::cos(angle) * roof_thickness;

        assert!(run > bearing_run, "roof needs an exterior eave overhang");
        assert!(
            bearing_underside <= p.aisle_wall_height
                && p.aisle_wall_height - bearing_underside <= 0.1,
            "roof underside must overlap the aisle wall head: {bearing_underside} vs {}",
            p.aisle_wall_height
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
            assert!((wall_max[2] - p.aisle_wall_height).abs() < 1.0e-5);
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
