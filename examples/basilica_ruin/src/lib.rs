// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Executable integration scenario for the Byzantine basilica worked example.

use exedra_assembly::{Assembly, InstanceId, InstancePath};

mod architecture;
mod geometry;
mod output;

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
    /// Height of the nave clerestory walls at the eaves.
    pub nave_wall_height: f64,
    /// Height of the exterior aisle arcade walls.
    pub aisle_wall_height: f64,
    /// Rise from the nave eaves to its roof ridge.
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

    /// Stable root-instance paths for major architectural elements.
    pub mod instances {
        /// Path of the north-west nave clerestory segment.
        pub const NAVE_WALL_NORTH_WEST: &str = "nave-wall-north-west";
        /// Path of the ruined south-west nave clerestory segment.
        pub const NAVE_WALL_SOUTH_WEST_RUIN: &str = "nave-wall-south-west-broken";
        /// Path of the north-east nave clerestory segment.
        pub const NAVE_WALL_NORTH_EAST: &str = "nave-wall-north-east";
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
/// that can contain the authored openings. [`BasilicaParams::default`] is the
/// validated reference configuration.
#[must_use]
pub fn build_basilica_assembly(params: &BasilicaParams) -> Assembly {
    architecture::build_assembly(params).0
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
        let assembly = build_basilica_assembly(&BasilicaParams::default());

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
        assert_eq!(truss_members.len(), 36);
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
            0xcb8f_38b3_2198_594d_b662_d171_ec8d_2f18
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
