// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;
use setout_generate::{LinearFragment, LinearStation};
use setout_joiner::lower_rational_iotas;

use super::BuildContext;
use crate::PlanSection;
use crate::buttress_instance_key;
use crate::geometry::box_recipe;
use crate::names;

pub(super) fn build(context: &mut BuildContext, plan: &PlanSection, stations: &LinearFragment) {
    let half_total = plan.half_total.as_meters();
    let buttress = context.add_part(
        names::parts::AISLE_BUTTRESS,
        box_recipe([0.72, 1.1, 5.5], "basilica:aisle-buttress"),
        "weathered-limestone",
    );
    for (side, y) in [("north", half_total - 0.18), ("south", -half_total - 0.92)] {
        for station in stations.items() {
            context.add_instance(
                &buttress_instance_key(side, station.label()),
                buttress,
                Placement3::translate(lower_x(station), y, 0.0),
                names::roles::AISLE_BUTTRESS,
            );
        }
    }
}

fn lower_x(station: &LinearStation) -> f64 {
    // Generation keeps non-integral iota positions rational. Convert that
    // exact payload once at the concrete assembly boundary; neither topology
    // nor the architecture module owns a second floating repeat calculation.
    lower_rational_iotas(station.position())
}

#[cfg(test)]
mod tests {
    use setout::Offset;
    use setout_generate::{InvocationKey, LinearDistribution, distribute_linear};

    use super::*;
    use crate::{BasilicaPremises, BasilicaSetout};

    #[test]
    fn generated_stations_lower_to_the_accepted_buttress_positions() {
        // The exact generator replaces the old floating repeat without moving
        // any buttress by more than one trillionth of a meter.
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default basilica resolves");
        let plan = setout.plan();
        let mut patterned = BuildContext::new();
        build(&mut patterned, plan, setout.buttress_stations());

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
        for (generated, accepted) in patterned.instances().iter().zip(legacy.instances()) {
            for (generated, accepted) in generated
                .placement()
                .rows
                .into_iter()
                .flatten()
                .zip(accepted.placement().rows.into_iter().flatten())
            {
                assert!((generated - accepted).abs() <= 1.0e-12);
            }
        }
    }

    #[test]
    fn semantic_endpoint_names_survive_count_growth() {
        // Growing the bay count adds a new interior identity while the two
        // physical endpoints retain names that cannot be mistaken for ranks.
        let invocation = InvocationKey::new("basilica/aisle-buttresses").unwrap();
        let fragment = distribute_linear(&LinearDistribution {
            invocation: &invocation,
            start: Offset::millimeters(1_700).unwrap(),
            end: Offset::millimeters(33_700).unwrap(),
            intervals: setout::Count::new(8),
            overrides: &[],
        })
        .unwrap();
        let keys: Vec<_> = fragment
            .items()
            .iter()
            .map(|station| buttress_instance_key("north", station.label()))
            .collect();

        assert_eq!(keys.first().unwrap(), "buttress-north-start");
        assert_eq!(keys.last().unwrap(), "buttress-north-end");
        assert!(
            keys.iter()
                .any(|key| key == "buttress-north-interior-000007")
        );
    }
}
