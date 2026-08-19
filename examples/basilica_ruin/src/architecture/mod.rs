// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_assembly::{Assembly, PartId};
use exedra_constructive::ir::{Placement3, Recipe};

use crate::{BasilicaParams, names};

mod aisles;
mod buttresses;
mod crossing;
mod east_end;
mod interior_arcades;
mod nave;

const CLERESTORY_BASE: f64 = 5.75;

#[derive(Copy, Clone)]
struct Layout {
    wall_thickness: f64,
    half_nave: f64,
    half_total: f64,
    crossing_west: f64,
    crossing_east: f64,
}

impl Layout {
    fn from_params(p: &BasilicaParams) -> Self {
        Self {
            wall_thickness: 0.45,
            half_nave: p.nave_width * 0.5,
            half_total: p.total_width * 0.5,
            crossing_west: p.crossing_x - p.drum_radius - 0.6,
            crossing_east: p.crossing_x + p.drum_radius + 0.6,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct Inventory {
    pub(crate) nave_walls: u32,
    pub(crate) aisles: u32,
    pub(crate) aisle_roofs: u32,
    pub(crate) round_head_openings: u32,
    pub(crate) interior_arcades: u32,
    pub(crate) interior_arcade_openings: u32,
    pub(crate) buttresses: u32,
    pub(crate) crossing_piers: u32,
    pub(crate) crossing_stages: u32,
    pub(crate) drum_windows: u32,
    pub(crate) cornice_bands: u32,
    pub(crate) chancel_openings: u32,
    pub(crate) apses: u32,
    pub(crate) drums: u32,
    pub(crate) domes: u32,
    pub(crate) ruined_bays: u32,
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
            .set_default_slot(part, "surface")
            .expect("every scenario recipe declares surface");
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

    fn finish(self) -> Assembly {
        self.assembly
    }
}

pub(crate) fn build_assembly(p: &BasilicaParams) -> (Assembly, Inventory) {
    let mut context = BuildContext::new();
    let layout = Layout::from_params(p);

    // Architectural systems join the scenario here in deterministic assembly
    // order. A new detail system should be one focused module and one call in
    // the architectural sequence, not another capability on `BuildContext`.
    nave::build(&mut context, p, layout);
    aisles::build(&mut context, p, layout);
    interior_arcades::build(&mut context, p, layout);
    east_end::build(&mut context, p, layout);
    crossing::build(&mut context, p, layout);
    buttresses::build(&mut context, p, layout);

    let inventory = Inventory {
        nave_walls: 4,
        aisles: 2,
        aisle_roofs: 2,
        round_head_openings: p.arcade_bays * 2 + 12 + 12 + 1,
        interior_arcades: 4,
        interior_arcade_openings: 12,
        buttresses: (p.arcade_bays + 1) * 2,
        crossing_piers: 4,
        crossing_stages: 1,
        drum_windows: 6,
        cornice_bands: 2,
        chancel_openings: 1,
        apses: 1,
        drums: 1,
        domes: 1,
        ruined_bays: 1,
    };
    (context.finish(), inventory)
}

#[cfg(test)]
mod tests {
    use crate::output::build_scenario;

    use super::Inventory;

    #[test]
    fn architectural_inventory_is_explicit_and_restrained() {
        let scenario = build_scenario();
        assert_eq!(
            scenario.inventory,
            Inventory {
                nave_walls: 4,
                aisles: 2,
                aisle_roofs: 2,
                round_head_openings: 39,
                interior_arcades: 4,
                interior_arcade_openings: 12,
                buttresses: 16,
                crossing_piers: 4,
                crossing_stages: 1,
                drum_windows: 6,
                cornice_bands: 2,
                chancel_openings: 1,
                apses: 1,
                drums: 1,
                domes: 1,
                ruined_bays: 1,
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
            "crossing-pier-south-west",
            "crossing-drum-panel-00",
            "crossing-drum-cornice-base",
            "crossing-dome",
        ] {
            assert!(
                paths.iter().any(|path| path == required),
                "missing {required}"
            );
        }
    }
}
