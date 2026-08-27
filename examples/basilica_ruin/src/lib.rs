// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable integration scenario for the Byzantine basilica worked example.

use exedra_assembly::Assembly;

mod architecture;
mod geometry;
mod output;
mod roof_setout;

pub use roof_setout::{
    BasilicaRoofSetout, RoofReconfiguration, RoofSection, RoofSetoutError, RoofSide,
};

#[derive(Copy, Clone, Debug, PartialEq)]
/// Parameters controlling the example's architectural massing.
///
/// Coordinates are metres in a Z-up frame: the nave runs along positive X,
/// width spans Y, and height is positive Z.
pub struct BasilicaParams {
    /// Length of the rectangular nave before the apse springs.
    pub length: f64,
    /// Clear width of the high central nave.
    pub nave_width: f64,
    /// Overall width across both side aisles.
    pub total_width: f64,
    /// Height of the masonry nave wall head below the timber wall plate.
    pub nave_wall_height: f64,
    /// Height of the exterior aisle arcade walls.
    pub aisle_wall_height: f64,
    /// Rise from the wall-plate bearing datum to the roof ridge.
    pub roof_rise: f64,
    /// Longitudinal center of the crossing and dome.
    pub crossing_x: f64,
    /// Circumradius of the polygonal drum.
    pub drum_radius: f64,
    /// Height of the drum walls between cornices.
    pub drum_height: f64,
    /// Rise of the shallow dome above the drum.
    pub dome_height: f64,
    /// Radius of the semicircular eastern apse.
    pub apse_radius: f64,
    /// Number of repeated arcade bays along each longitudinal wall.
    pub arcade_bays: u32,
}

impl Default for BasilicaParams {
    fn default() -> Self {
        Self {
            length: 36.0,
            nave_width: 9.0,
            total_width: 18.0,
            nave_wall_height: 11.0,
            aisle_wall_height: 5.2,
            roof_rise: 3.2,
            crossing_x: 26.0,
            drum_radius: 4.1,
            drum_height: 2.6,
            dome_height: 3.1,
            apse_radius: 4.5,
            arcade_bays: 7,
        }
    }
}

/// Runs the example CLI and writes its deterministic OBJ and diagnostic glTF.
///
/// This keeps compilation and export products internal: consumers that need
/// to edit the building should call [`build_basilica_assembly`] and compile
/// only after their mutations are complete.
pub fn run_cli() {
    output::run_cli();
}

/// Stable vocabulary for addressing major parts, instances, and roles.
///
/// The string values are part of this example's identity contract. OBJ group
/// names and glTF node metadata retain the same instance addresses and part keys.
pub mod names {
    /// Metadata key storing an instance's architectural role.
    pub const ARCHITECTURAL_ROLE: &str = "architectural_role";

    /// Stable part registration keys.
    pub mod parts {
        /// The intact west clerestory segment, reused on the north side.
        pub const NAVE_CLERESTORY_WEST: &str = "nave-clerestory-west";
        /// The short clerestory segment east of the open crossing.
        pub const NAVE_CLERESTORY_EAST: &str = "nave-clerestory-east";
        /// Continuous north wall plate west of the crossing.
        pub const NAVE_WALL_PLATE_WEST: &str = "nave-wall-plate-west";
        /// Surviving west portion of the broken south wall plate.
        pub const NAVE_WALL_PLATE_RUIN_A: &str = "nave-wall-plate-ruin-a";
        /// Surviving east portion of the broken south wall plate.
        pub const NAVE_WALL_PLATE_RUIN_B: &str = "nave-wall-plate-ruin-b";
        /// Wall plate reused on both sides east of the crossing.
        pub const NAVE_WALL_PLATE_EAST: &str = "nave-wall-plate-east";
        /// The long lower nave-to-aisle arcade segment west of the crossing.
        pub const INTERIOR_ARCADE_WEST: &str = "interior-arcade-west";
        /// The short lower nave-to-aisle arcade segment east of the crossing.
        pub const INTERIOR_ARCADE_EAST: &str = "interior-arcade-east";
        /// The constructive part carrying the crossing dome.
        pub const CROSSING_DOME: &str = "crossing-dome";
        /// The faceted ruled-loft web reused at all four crossing corners.
        pub const PENDENTIVE_WEB: &str = "crossing-pendentive-web";
        /// The pierced panel reused around alternating drum faces.
        pub const DRUM_WINDOW_PANEL: &str = "crossing-drum-window-panel";
        /// The exterior aisle buttress reused on both elevations.
        pub const AISLE_BUTTRESS: &str = "aisle-buttress";
        /// The east chancel gable containing the apse opening.
        pub const EAST_CHANCEL_GABLE: &str = "east-chancel-gable";
        /// The transverse timber tying the nave walls in each roof truss.
        pub const NAVE_TRUSS_TIE_BEAM: &str = "nave-truss-tie-beam";
        /// The sloping timber reused on both sides of every nave roof truss.
        pub const NAVE_TRUSS_PRINCIPAL_RAFTER: &str = "nave-truss-principal-rafter";
        /// The vertical timber joining each tie beam to the roof apex.
        pub const NAVE_TRUSS_KING_POST: &str = "nave-truss-king-post";
        /// The diagonal timber reused on both sides of every nave roof truss.
        pub const NAVE_TRUSS_DIAGONAL_BRACE: &str = "nave-truss-diagonal-brace";
    }

    /// Stable root-instance names for major architectural elements.
    pub mod instances {
        /// Path of the north-west nave clerestory segment.
        pub const NAVE_WALL_NORTH_WEST: &str = "nave-wall-north-west";
        /// Path of the ruined south-west nave clerestory segment.
        pub const NAVE_WALL_SOUTH_WEST_RUIN: &str = "nave-wall-south-west-broken";
        /// Path of the north-east nave clerestory segment.
        pub const NAVE_WALL_NORTH_EAST: &str = "nave-wall-north-east";
        /// Path of the continuous north-west nave wall plate.
        pub const NAVE_WALL_PLATE_NORTH_WEST: &str = "nave-wall-plate-north-west";
        /// Path of the surviving west part of the south-west wall plate.
        pub const NAVE_WALL_PLATE_SOUTH_WEST_A: &str = "nave-wall-plate-south-west-a";
        /// Path of the surviving east part of the south-west wall plate.
        pub const NAVE_WALL_PLATE_SOUTH_WEST_B: &str = "nave-wall-plate-south-west-b";
        /// Path of the north-east nave wall plate.
        pub const NAVE_WALL_PLATE_NORTH_EAST: &str = "nave-wall-plate-north-east";
        /// Path of the south-east nave wall plate.
        pub const NAVE_WALL_PLATE_SOUTH_EAST: &str = "nave-wall-plate-south-east";
        /// Path of the north-west nave-to-aisle arcade segment.
        pub const INTERIOR_ARCADE_NORTH_WEST: &str = "interior-arcade-north-west";
        /// Path of the south-west nave-to-aisle arcade segment.
        pub const INTERIOR_ARCADE_SOUTH_WEST: &str = "interior-arcade-south-west";
        /// Path of the north-east nave-to-aisle arcade segment.
        pub const INTERIOR_ARCADE_NORTH_EAST: &str = "interior-arcade-north-east";
        /// Path of the south-east nave-to-aisle arcade segment.
        pub const INTERIOR_ARCADE_SOUTH_EAST: &str = "interior-arcade-south-east";
        /// Path of the crossing dome instance.
        pub const CROSSING_DOME: &str = "crossing-dome";
        /// Path of the open square bearing ring that supports the drum.
        pub const CROSSING_PLATFORM: &str = "crossing-platform";
        /// Path of the north-east crossing pendentive.
        pub const PENDENTIVE_NORTH_EAST: &str = "crossing-pendentive-north-east";
        /// Path of the north-west crossing pendentive.
        pub const PENDENTIVE_NORTH_WEST: &str = "crossing-pendentive-north-west";
        /// Path of the south-west crossing pendentive.
        pub const PENDENTIVE_SOUTH_WEST: &str = "crossing-pendentive-south-west";
        /// Path of the south-east crossing pendentive.
        pub const PENDENTIVE_SOUTH_EAST: &str = "crossing-pendentive-south-east";
        /// Path of the east chancel gable instance.
        pub const EAST_CHANCEL_GABLE: &str = "east-chancel-gable";
        /// Path of the eastern apse instance.
        pub const EAST_APSE: &str = "east-apse";
    }

    /// Stable architectural-role values used for semantic selection.
    pub mod roles {
        /// Intact segments of the high nave clerestory.
        pub const NAVE_CLERESTORY: &str = "nave_clerestory";
        /// The one deliberately broken high nave wall segment.
        pub const NAVE_CLERESTORY_RUIN: &str = "nave_clerestory_ruin";
        /// Lower pierced walls communicating between the nave and side aisles.
        pub const INTERIOR_ARCADE: &str = "interior_arcade";
        /// All exterior aisle buttresses.
        pub const AISLE_BUTTRESS: &str = "aisle_buttress";
        /// Pierced faces of the polygonal drum.
        pub const DRUM_WINDOW: &str = "drum_window";
        /// Ground-bearing piers supporting the crossing stage.
        pub const CROSSING_PIER: &str = "crossing_pier";
        /// Faceted webs mediating between the square crossing and drum.
        pub const PENDENTIVE: &str = "pendentive";
        /// Independently addressable structural members of the nave roof trusses.
        pub const NAVE_TRUSS_MEMBER: &str = "nave_truss_member";
        /// Continuous timber bearing between the roof and clerestory wall.
        pub const NAVE_WALL_PLATE: &str = "nave_wall_plate";
    }
}

/// Builds a mutable, Addressable-ready basilica assembly without compiling it.
///
/// Consumers may find construction-time parts through [`Assembly::part_by_key`].
/// Once authoring is complete, bind the result with
/// [`Assembly::into_addressable`] to resolve exact addresses, run typed
/// metadata or part queries, inspect effective-material evidence, and commit
/// later authoring under one revision clock. Compile after authoring is
/// complete.
///
/// # Panics
///
/// Panics when parameters do not describe positive, nested building extents
/// that can contain the authored openings. [`BasilicaParams::default`] is the
/// validated reference configuration.
#[must_use]
pub fn build_basilica_assembly(params: &BasilicaParams) -> Assembly {
    architecture::build_assembly(params).0
}

#[cfg(test)]
pub(crate) fn instance_id_at(
    assembly: &Assembly,
    name: &str,
) -> Option<exedra_assembly::InstanceId> {
    use exedra_assembly::InstanceAddress;

    let address = InstanceAddress::parse(&format!("/{name}")).ok()?;
    assembly.instance_by_address(&address)
}

#[cfg(test)]
pub(crate) fn role_instances(assembly: &Assembly, role: &str) -> Vec<exedra_assembly::InstanceId> {
    use addressable::{Query, SpaceId};
    use addressable_tree::TreeRuntime;
    use exedra_assembly::{AssemblyAxis, AssemblyPredicate, AssemblySpace, AssemblyView};

    let space = TreeRuntime::new(SpaceId::<AssemblySpace>::new(1), assembly);
    space
        .query_many(
            &Query::many(space.root_locator(AssemblyView::Instances))
                .traverse(AssemblyAxis::Descendants)
                .filter(AssemblyPredicate::metadata_equals(
                    names::ARCHITECTURAL_ROLE,
                    role,
                )),
        )
        .expect("Basilica role query succeeds")
        .items()
        .iter()
        .map(|location| {
            *space
                .resolved_handle(location)
                .expect("queried instances carry handles")
                .handle()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use addressable::{Query, Resolution, SpaceId};
    use exedra_assembly::{
        AssemblyAxis, AssemblyPredicate, AssemblySpace, InstanceAddress, assembly_fingerprint,
    };

    use super::*;
    use crate::output::{bounds, build_scenario, byte_signature, export_obj, scene_stats};

    #[test]
    fn public_names_resolve_addresses_parts_and_roles() {
        let assembly = build_basilica_assembly(&BasilicaParams::default());
        let space = assembly
            .clone()
            .into_addressable(SpaceId::<AssemblySpace>::new(1));

        let dome_part = assembly
            .part_by_key(names::parts::CROSSING_DOME)
            .expect("stable dome part key resolves");
        let dome_address =
            InstanceAddress::parse(&format!("/{}", names::instances::CROSSING_DOME)).unwrap();
        let Resolution::Resolved(dome) =
            space.resolve(&space.locator(&dome_address).expect("current dome address"))
        else {
            panic!("stable dome address resolves");
        };
        let dome_instance = *space
            .resolved_handle(&dome)
            .expect("dome has a runtime handle")
            .handle();
        assert_eq!(assembly.instance(dome_instance).unwrap().part(), dome_part);
        assert_eq!(
            dome.address(),
            &dome_address,
            "resolved location retains the stored address"
        );

        let pendentive_part = assembly
            .part_by_key(names::parts::PENDENTIVE_WEB)
            .expect("stable pendentive part key resolves");
        let pendentive_address =
            InstanceAddress::parse(&format!("/{}", names::instances::PENDENTIVE_NORTH_EAST))
                .unwrap();
        let Resolution::Resolved(pendentive) = space.resolve(
            &space
                .locator(&pendentive_address)
                .expect("current pendentive address"),
        ) else {
            panic!("stable pendentive address resolves");
        };
        let pendentive_instance = *space
            .resolved_handle(&pendentive)
            .expect("pendentive has a runtime handle")
            .handle();
        assert_eq!(
            assembly.instance(pendentive_instance).unwrap().part(),
            pendentive_part
        );

        let select_role = |role| {
            space
                .query_many(
                    &Query::many(space.root_locator())
                        .traverse(AssemblyAxis::Descendants)
                        .filter(AssemblyPredicate::metadata_equals(
                            names::ARCHITECTURAL_ROLE,
                            role,
                        )),
                )
                .expect("role query succeeds")
        };
        let buttresses = select_role(names::roles::AISLE_BUTTRESS);
        let windows = select_role(names::roles::DRUM_WINDOW);
        let intact_nave_walls = select_role(names::roles::NAVE_CLERESTORY);
        let ruined_nave_walls = select_role(names::roles::NAVE_CLERESTORY_RUIN);
        let pendentives = select_role(names::roles::PENDENTIVE);
        let truss_members = select_role(names::roles::NAVE_TRUSS_MEMBER);
        assert_eq!(buttresses.items().len(), 16);
        assert_eq!(windows.items().len(), 6);
        assert_eq!(intact_nave_walls.items().len(), 3);
        assert_eq!(ruined_nave_walls.items().len(), 1);
        assert_eq!(pendentives.items().len(), 4);
        assert_eq!(truss_members.items().len(), 36);
        assert!(buttresses.items().iter().all(|location| {
            location.referent().as_part().map(|key| key.as_str())
                == Some(names::parts::AISLE_BUTTRESS)
        }));
        assert!(windows.items().iter().all(|location| {
            location.referent().as_part().map(|key| key.as_str())
                == Some(names::parts::DRUM_WINDOW_PANEL)
        }));
    }

    #[test]
    fn public_name_vocabulary_is_stable() {
        assert_eq!(names::ARCHITECTURAL_ROLE, "architectural_role");
        assert_eq!(names::parts::CROSSING_DOME, "crossing-dome");
        assert_eq!(names::parts::PENDENTIVE_WEB, "crossing-pendentive-web");
        assert_eq!(names::parts::NAVE_CLERESTORY_WEST, "nave-clerestory-west");
        assert_eq!(names::parts::NAVE_CLERESTORY_EAST, "nave-clerestory-east");
        assert_eq!(names::parts::INTERIOR_ARCADE_WEST, "interior-arcade-west");
        assert_eq!(names::parts::INTERIOR_ARCADE_EAST, "interior-arcade-east");
        assert_eq!(names::parts::NAVE_TRUSS_TIE_BEAM, "nave-truss-tie-beam");
        assert_eq!(
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
            "nave-truss-principal-rafter"
        );
        assert_eq!(names::parts::NAVE_TRUSS_KING_POST, "nave-truss-king-post");
        assert_eq!(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
            "nave-truss-diagonal-brace"
        );
        assert_eq!(names::instances::CROSSING_DOME, "crossing-dome");
        assert_eq!(
            names::instances::NAVE_WALL_NORTH_WEST,
            "nave-wall-north-west"
        );
        assert_eq!(
            names::instances::NAVE_WALL_SOUTH_WEST_RUIN,
            "nave-wall-south-west-broken"
        );
        assert_eq!(names::instances::CROSSING_PLATFORM, "crossing-platform");
        assert_eq!(
            names::instances::PENDENTIVE_NORTH_EAST,
            "crossing-pendentive-north-east"
        );
        assert_eq!(names::instances::EAST_CHANCEL_GABLE, "east-chancel-gable");
        assert_eq!(names::instances::EAST_APSE, "east-apse");
        assert_eq!(names::roles::AISLE_BUTTRESS, "aisle_buttress");
        assert_eq!(names::roles::INTERIOR_ARCADE, "interior_arcade");
        assert_eq!(names::roles::DRUM_WINDOW, "drum_window");
        assert_eq!(names::roles::PENDENTIVE, "pendentive");
        assert_eq!(names::roles::NAVE_TRUSS_MEMBER, "nave_truss_member");
    }

    #[test]
    fn silhouette_has_basilica_proportions() {
        let scenario = build_scenario();
        let (min, max) = bounds(&scenario.compiled, &scenario.render_list);
        let extents = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        assert!(extents.iter().all(|value| value.is_finite()));
        assert!(extents[0] > extents[1] * 2.0, "longitudinal: {extents:?}");
        assert!(extents[2] > 16.0, "dome dominates skyline: {extents:?}");
        assert!(max[0] > 40.0, "apse projects eastward: {min:?}..{max:?}");
        assert!(
            min[1] < -9.0 && max[1] > 9.0,
            "paired aisles: {min:?}..{max:?}"
        );
    }

    #[test]
    fn evaluation_and_export_are_deterministic() {
        let a = build_scenario();
        let b = build_scenario();
        let obj_a = export_obj(&a.compiled, &a.render_list);
        let obj_b = export_obj(&b.compiled, &b.render_list);
        let gltf_a = exedra_gltf::export_gltf_with_options(
            &a.assembly,
            &a.compiled,
            &a.render_list,
            exedra_gltf::GltfExportOptions::z_up_to_y_up(),
        )
        .expect("matching assembly");
        let gltf_b = exedra_gltf::export_gltf_with_options(
            &b.assembly,
            &b.compiled,
            &b.render_list,
            exedra_gltf::GltfExportOptions::z_up_to_y_up(),
        )
        .expect("matching assembly");
        assert_eq!(scene_stats(&a), scene_stats(&b));
        assert_eq!(a.diagnostics, 0);
        assert_eq!(b.diagnostics, 0);
        assert_eq!(
            assembly_fingerprint(&a.assembly),
            assembly_fingerprint(&b.assembly)
        );
        assert_eq!(
            assembly_fingerprint(&a.assembly),
            0x9a6f_3c0f_4ad4_9a83_0a23_96a3_27e7_fadd
        );
        assert_eq!(obj_a, obj_b);
        assert_eq!(gltf_a.json, gltf_b.json);
        // The assembly fingerprint above is the cross-platform semantic
        // golden. OBJ bytes are required to repeat exactly on the same target.
        assert_eq!(
            byte_signature(obj_a.as_bytes()),
            byte_signature(obj_b.as_bytes())
        );
    }
}
