// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use cambium::assembly::{
    InstanceTemplate, LinearRepeat, MetadataEntry, NamedAssemblyPattern, repeat_linear,
};
use exedra_constructive::ir::Placement3;

use super::BuildContext;
use crate::PlanSection;
use crate::geometry::box_recipe;
use crate::names;

pub(super) fn build(context: &mut BuildContext, plan: &PlanSection) {
    let half_total = plan.half_total.as_meters();
    let buttress = context.add_part(
        names::parts::AISLE_BUTTRESS,
        box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
        "weathered-limestone",
    );
    let arcade_bays =
        u32::try_from(plan.arcade_bays.get()).expect("arcade bay count fits repeat topology");
    // The exact massing owns the total length and repeat count. These inset
    // endpoints still belong to the authored buttress pattern; moving the
    // repeat itself into `setout_generate` is the next architectural seam.
    let bay_pitch = (plan.length.as_meters() - 4.0) / f64::from(arcade_bays);
    let occurrences = repeat_linear(&LinearRepeat {
        count: arcade_bays + 1,
        start: [1.7, 0.0, 0.0],
        step: [bay_pitch, 0.0, 0.0],
        omitted: &[],
    })
    .expect("accepted buttress repeat must be finite");
    let role = [MetadataEntry {
        key: names::ARCHITECTURAL_ROLE,
        value: names::roles::AISLE_BUTTRESS,
    }];
    for (side, y) in [("north", half_total - 0.18), ("south", -half_total - 0.92)] {
        let members = [InstanceTemplate {
            key_suffix: "",
            part: buttress,
            placement: Placement3::translate(0.0, y, 0.0),
            bindings: &[],
            metadata: &role,
        }];
        let key_prefix = format!("buttress-{side}");
        context.instantiate_pattern(
            &NamedAssemblyPattern {
                parent: None,
                key_prefix: &key_prefix,
                ordinal_width: 2,
                members: &members,
            },
            &occurrences,
        );
    }
}

#[cfg(test)]
mod tests {
    use exedra_assembly::assembly_fingerprint;

    use super::*;
    use crate::{BasilicaPremises, BasilicaSetout};

    #[test]
    fn named_patterns_match_the_accepted_nested_loop_payload() {
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default basilica resolves");
        let plan = setout.plan();
        let mut patterned = BuildContext::new();
        build(&mut patterned, plan);

        let mut legacy = BuildContext::new();
        let part = legacy.add_part(
            names::parts::AISLE_BUTTRESS,
            box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
            "weathered-limestone",
        );
        let arcade_bays = u32::try_from(plan.arcade_bays.get()).expect("bay count fits u32");
        let bay_pitch = (plan.length.as_meters() - 4.0) / f64::from(arcade_bays);
        for (side, y) in [
            ("north", plan.half_total.as_meters() - 0.18),
            ("south", -plan.half_total.as_meters() - 0.92),
        ] {
            for bay in 0..=arcade_bays {
                legacy.add_instance(
                    &format!("buttress-{side}-{bay:02}"),
                    part,
                    Placement3::translate(1.7 + f64::from(bay) * bay_pitch, y, 0.0),
                    names::roles::AISLE_BUTTRESS,
                );
            }
        }

        let patterned = patterned.finish();
        let legacy = legacy.finish();
        assert_eq!(patterned.instances().len(), 16);
        assert_eq!(
            assembly_fingerprint(&patterned),
            assembly_fingerprint(&legacy)
        );
    }
}
