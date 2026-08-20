// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use cambium::assembly::{
    InstanceTemplate, LinearRepeat, MetadataEntry, NamedAssemblyPattern, repeat_linear,
};
use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::BasilicaParams;
use crate::geometry::box_recipe;
use crate::names;

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let half_total = layout.half_total;
    let buttress = context.add_part(
        names::parts::AISLE_BUTTRESS,
        box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
        "weathered-limestone",
    );
    let bay_pitch = (p.length - 4.0) / f64::from(p.arcade_bays);
    let occurrences = repeat_linear(&LinearRepeat {
        count: p.arcade_bays + 1,
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

    #[test]
    fn named_patterns_match_the_accepted_nested_loop_payload() {
        let p = BasilicaParams::default();
        let layout = Layout::from_params(&p);
        let mut patterned = BuildContext::new();
        build(&mut patterned, &p, layout);

        let mut legacy = BuildContext::new();
        let part = legacy.add_part(
            names::parts::AISLE_BUTTRESS,
            box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
            "weathered-limestone",
        );
        let bay_pitch = (p.length - 4.0) / f64::from(p.arcade_bays);
        for (side, y) in [
            ("north", layout.half_total - 0.18),
            ("south", -layout.half_total - 0.92),
        ] {
            for bay in 0..=p.arcade_bays {
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
