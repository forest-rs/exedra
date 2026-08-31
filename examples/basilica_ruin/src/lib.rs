// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable integration scenario for the Byzantine basilica worked example.

use exedra_assembly::{Assembly, InstanceId, InstancePath};
use setout_generate::ItemLabel;

mod architecture;
mod basilica_setout;
mod geometry;
mod output;

fn buttress_instance_key(side: &str, label: &ItemLabel) -> String {
    // Generated labels use `/` as a semantic path separator. Assembly root
    // keys use `-`, so this is the one explicit identity adapter between the
    // two systems rather than a second ordinal-labeling policy.
    format!("buttress-{side}-{}", label.as_str().replace('/', "-"))
}

pub use basilica_setout::{
    AisleSection, BasilicaPremises, BasilicaReconfiguration, BasilicaSetout, BasilicaSetoutError,
    CrossingSection, EastEndSection, LevelSection, PlanSection, RoofSection, RoofSide,
};

/// Runs the example CLI and writes its deterministic OBJ and binary glTF.
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
/// names and glTF node metadata retain the same instance paths and part keys.
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
        /// The canonical north-side principal-rafter part.
        pub const NAVE_TRUSS_PRINCIPAL_RAFTER: &str = "nave-truss-principal-rafter";
        /// The mirror-composed south-side principal-rafter part.
        pub const NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH: &str = "nave-truss-principal-rafter-south";
        /// The vertical timber joining each tie beam to the roof apex.
        pub const NAVE_TRUSS_KING_POST: &str = "nave-truss-king-post";
        /// The transverse key suspending each tie from its king post.
        pub const NAVE_TRUSS_KING_POST_KEY: &str = "nave-truss-king-post-key";
        /// The canonical north-side diagonal-brace part.
        pub const NAVE_TRUSS_DIAGONAL_BRACE: &str = "nave-truss-diagonal-brace";
        /// The mirror-composed south-side diagonal-brace part.
        pub const NAVE_TRUSS_DIAGONAL_BRACE_SOUTH: &str = "nave-truss-diagonal-brace-south";
    }

    /// Stable root-instance paths for major architectural elements.
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

/// Builds a mutable, name-addressable basilica assembly without compiling it.
///
/// Consumers may find parts through [`Assembly::part_by_key`], resolve stable
/// instance paths with [`resolve_instance_path`], or select semantic groups
/// through [`instances_with_role`]. Compile only after all edits are applied.
///
/// # Panics
///
/// Panics when parameters do not describe positive, nested building extents
/// that can contain the authored openings. [`BasilicaPremises::default`] is the
/// validated reference configuration.
#[must_use]
pub fn build_basilica_assembly(premises: &BasilicaPremises) -> Assembly {
    architecture::build_assembly(premises).0
}

/// Resolves a slash-separated stable instance path.
///
/// Returns `None` for an empty path, empty path segments, or an unknown path.
#[must_use]
pub fn resolve_instance_path(assembly: &Assembly, path: &str) -> Option<InstanceId> {
    if path.is_empty() || path.split('/').any(str::is_empty) {
        return None;
    }
    let segments: Vec<&str> = path.split('/').collect();
    assembly.resolve_path(&InstancePath::from_segments(&segments))
}

/// Collects instance handles whose architectural role equals `role`.
///
/// Results preserve deterministic assembly insertion order.
#[must_use]
pub fn instances_with_role(assembly: &Assembly, role: &str) -> Vec<InstanceId> {
    assembly
        .instances_with_ids()
        .filter_map(|(id, instance)| {
            let matches = instance
                .metadata()
                .iter()
                .any(|(key, value)| key == names::ARCHITECTURAL_ROLE && value == role);
            matches.then_some(id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use exedra_assembly::assembly_fingerprint;

    use super::*;
    use crate::output::{bounds, build_scenario, byte_signature, export_obj, scene_stats};

    #[test]
    fn public_names_resolve_parts_paths_and_roles() {
        // Stable selectors must still resolve after the fitted trusses add a
        // generated key and distinct handed member recipes behind one role.
        let assembly = build_basilica_assembly(&BasilicaPremises::default());

        let dome_part = assembly
            .part_by_key(names::parts::CROSSING_DOME)
            .expect("stable dome part key resolves");
        let dome_instance = resolve_instance_path(&assembly, names::instances::CROSSING_DOME)
            .expect("stable dome instance path resolves");
        assert_eq!(assembly.instance(dome_instance).unwrap().part(), dome_part);
        assert_eq!(
            assembly.path_of(dome_instance).unwrap().to_string(),
            names::instances::CROSSING_DOME
        );

        let pendentive_part = assembly
            .part_by_key(names::parts::PENDENTIVE_WEB)
            .expect("stable pendentive part key resolves");
        let pendentive_instance =
            resolve_instance_path(&assembly, names::instances::PENDENTIVE_NORTH_EAST)
                .expect("stable pendentive instance path resolves");
        assert_eq!(
            assembly.instance(pendentive_instance).unwrap().part(),
            pendentive_part
        );

        for key in [
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH,
            names::parts::NAVE_TRUSS_KING_POST_KEY,
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH,
        ] {
            assert!(
                assembly.part_by_key(key).is_some(),
                "new fitted member part is publicly addressable: {key}"
            );
        }

        let buttresses = instances_with_role(&assembly, names::roles::AISLE_BUTTRESS);
        let windows = instances_with_role(&assembly, names::roles::DRUM_WINDOW);
        let intact_nave_walls = instances_with_role(&assembly, names::roles::NAVE_CLERESTORY);
        let ruined_nave_walls = instances_with_role(&assembly, names::roles::NAVE_CLERESTORY_RUIN);
        let pendentives = instances_with_role(&assembly, names::roles::PENDENTIVE);
        let truss_members = instances_with_role(&assembly, names::roles::NAVE_TRUSS_MEMBER);
        assert_eq!(buttresses.len(), 16);
        assert_eq!(windows.len(), 6);
        assert_eq!(intact_nave_walls.len(), 3);
        assert_eq!(ruined_nave_walls.len(), 1);
        assert_eq!(pendentives.len(), 4);
        assert_eq!(truss_members.len(), 42);
        assert!(buttresses.iter().all(|&id| {
            assembly.instance(id).unwrap().part()
                == assembly.part_by_key(names::parts::AISLE_BUTTRESS).unwrap()
        }));
        assert!(windows.iter().all(|&id| {
            assembly.instance(id).unwrap().part()
                == assembly
                    .part_by_key(names::parts::DRUM_WINDOW_PANEL)
                    .unwrap()
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
        assert_eq!(
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER_SOUTH,
            "nave-truss-principal-rafter-south"
        );
        assert_eq!(names::parts::NAVE_TRUSS_KING_POST, "nave-truss-king-post");
        assert_eq!(
            names::parts::NAVE_TRUSS_KING_POST_KEY,
            "nave-truss-king-post-key"
        );
        assert_eq!(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
            "nave-truss-diagonal-brace"
        );
        assert_eq!(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE_SOUTH,
            "nave-truss-diagonal-brace-south"
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
        assert!(resolve_instance_path(&Assembly::new(), "").is_none());
        assert!(resolve_instance_path(&Assembly::new(), "root//child").is_none());
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
        // Rebuilding the complete gallery twice must reproduce its assembly,
        // fitted timber meshes, OBJ text, and glTF payload exactly.
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
            0x3a0a_91c3_dadb_fc87_5d1c_9163_1f20_4c94
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
