// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use cambium::assembly::{LinearOccurrence, NamedAssemblyPattern, instantiate_named};
use exedra_assembly::{Assembly, PartId};
use exedra_constructive::ir::{Placement3, Recipe};

use crate::{BasilicaPremises, BasilicaSetout, names};

mod aisles;
mod buttresses;
mod crossing;
mod crossing_transition;
mod east_end;
mod interior_arcades;
mod nave;
mod nave_trusses;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Inventory {
    pub(crate) nave_walls: u32,
    pub(crate) nave_wall_plates: u32,
    pub(crate) aisles: u32,
    pub(crate) aisle_roofs: u32,
    pub(crate) round_head_openings: u32,
    pub(crate) interior_arcades: u32,
    pub(crate) interior_arcade_openings: u32,
    pub(crate) buttresses: u32,
    pub(crate) crossing_piers: u32,
    pub(crate) crossing_stages: u32,
    pub(crate) pendentives: u32,
    pub(crate) drum_windows: u32,
    pub(crate) cornice_bands: u32,
    pub(crate) chancel_openings: u32,
    pub(crate) apses: u32,
    pub(crate) drums: u32,
    pub(crate) domes: u32,
    pub(crate) ruined_bays: u32,
    pub(crate) nave_trusses: u32,
    pub(crate) nave_truss_members: u32,
    pub(crate) omitted_nave_trusses: u32,
}

/// Example-local assembly wiring shared by the architectural systems.
///
/// This is deliberately not a modeling DSL: it only makes the scenario's
/// required surface slot, material binding, and semantic role metadata
/// impossible to forget.
struct BuildContext {
    assembly: Assembly,
}

impl BuildContext {
    fn new() -> Self {
        Self {
            assembly: Assembly::new(),
        }
    }

    fn add_part(&mut self, key: &str, recipe: Recipe, material: &str) -> PartId {
        let part = self
            .assembly
            .add_recipe_part(key, recipe)
            .expect("unique constant part key");
        self.assembly
            .set_part_material(part, "surface", material)
            .expect("surface slot exists");
        part
    }

    fn add_instance(&mut self, key: &str, part: PartId, placement: Placement3, role: &str) {
        let instance = self
            .assembly
            .add_instance(None, key, part, placement)
            .expect("unique constant instance key");
        self.assembly
            .set_metadata(instance, names::ARCHITECTURAL_ROLE, role)
            .expect("fresh instance exists");
    }

    fn instantiate_pattern(
        &mut self,
        pattern: &NamedAssemblyPattern<'_>,
        occurrences: &[LinearOccurrence],
    ) {
        instantiate_named(&mut self.assembly, pattern, occurrences)
            .expect("scenario pattern must satisfy the accepted assembly contract");
    }

    fn finish(self) -> Assembly {
        self.assembly
    }
}

pub(crate) fn build_assembly(premises: &BasilicaPremises) -> (Assembly, Inventory) {
    let mut context = BuildContext::new();
    let setout = BasilicaSetout::new(premises).expect("basilica premises must set out");
    let plan = setout.plan();
    let levels = setout.levels();
    let aisle = setout.aisle();
    let roof = setout.roof();
    let crossing = setout.crossing();
    let east_end = setout.east_end();

    // Architectural systems join the scenario here in deterministic assembly
    // order. A new detail system should be one focused module and one call in
    // the architectural sequence, not another capability on `BuildContext`.
    nave::build(&mut context, plan, levels, roof);
    aisles::build(&mut context, plan, levels, aisle);
    interior_arcades::build(&mut context, plan, levels);
    east_end::build(&mut context, plan, east_end, roof);
    crossing::build(&mut context, plan, levels, crossing);
    crossing_transition::build(&mut context, plan, levels, crossing);
    buttresses::build(&mut context, plan, setout.buttress_stations());
    // Append interior timber detail after the accepted primary fabric. Stable
    // identities survive the new continuous wall plates even though the old
    // artifact's numeric instance prefix intentionally grows.
    nave_trusses::build(&mut context, plan, &setout);

    let arcade_bays = u32::try_from(plan.arcade_bays.get())
        .expect("the basilica arcade bay count fits the assembly inventory");

    let inventory = Inventory {
        nave_walls: 4,
        nave_wall_plates: 5,
        aisles: 2,
        aisle_roofs: 2,
        round_head_openings: arcade_bays * 2 + 12 + 12 + 1,
        interior_arcades: 4,
        interior_arcade_openings: 12,
        buttresses: u32::try_from(setout.buttress_stations().items().len())
            .expect("bounded generated buttress count fits u32")
            * 2,
        crossing_piers: 4,
        crossing_stages: 1,
        pendentives: 4,
        drum_windows: 6,
        cornice_bands: 2,
        chancel_openings: 1,
        apses: 1,
        drums: 1,
        domes: 1,
        ruined_bays: 1,
        nave_trusses: 6,
        nave_truss_members: 42,
        omitted_nave_trusses: 1,
    };
    (context.finish(), inventory)
}

#[cfg(test)]
mod tests {
    use crate::output::build_scenario;

    use super::Inventory;

    #[test]
    fn architectural_inventory_is_explicit_and_restrained() {
        // The gallery inventory is a deliberate public review surface; the
        // generated key adds exactly one visible truss member per station.
        let scenario = build_scenario();
        assert_eq!(
            scenario.inventory,
            Inventory {
                nave_walls: 4,
                nave_wall_plates: 5,
                aisles: 2,
                aisle_roofs: 2,
                round_head_openings: 39,
                interior_arcades: 4,
                interior_arcade_openings: 12,
                buttresses: 16,
                crossing_piers: 4,
                crossing_stages: 1,
                pendentives: 4,
                drum_windows: 6,
                cornice_bands: 2,
                chancel_openings: 1,
                apses: 1,
                drums: 1,
                domes: 1,
                ruined_bays: 1,
                nave_trusses: 6,
                nave_truss_members: 42,
                omitted_nave_trusses: 1,
            }
        );
        let paths: Vec<String> = scenario
            .render_list
            .items
            .iter()
            .map(|item| item.path.to_string())
            .collect();
        for required in [
            "nave-wall-north-west",
            "nave-wall-south-west-broken",
            "nave-wall-north-east",
            "nave-wall-south-east",
            "aisle-wall-north",
            "aisle-wall-south",
            "aisle-roof-north",
            "aisle-roof-south",
            "interior-arcade-north-west",
            "interior-arcade-south-west",
            "interior-arcade-north-east",
            "interior-arcade-south-east",
            "west-facade",
            "east-chancel-gable",
            "east-apse",
            "crossing-platform",
            "crossing-pendentive-north-east",
            "crossing-pier-south-west",
            "crossing-drum-panel-00",
            "crossing-drum-cornice-base",
            "crossing-dome",
            "nave-truss-west-00-tie-beam",
            "nave-truss-west-05-king-post",
            "nave-truss-east-00-diagonal-brace-south",
        ] {
            assert!(
                paths.iter().any(|path| path == required),
                "missing {required}"
            );
        }
    }
}
