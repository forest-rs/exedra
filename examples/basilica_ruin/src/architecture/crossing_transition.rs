// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::{Placement3, Recipe};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegTag};

use super::{BuildContext, crossing_platform_base};
use crate::geometry::ruled_loft_recipe;
use crate::roof_setout::RoofSection;
use crate::{BasilicaParams, names};

const WEB_OUTER: f64 = 4.18;
const WEB_BOTTOM: f64 = 11.4;
const WEB_MIDDLE: f64 = 13.0;
const WEB_TOP_OVERLAP: f64 = 0.02;

/// Adds the four masonry webs that mediate between the square crossing and
/// the polygonal drum bearing above it.
///
/// Exedra cannot yet trim a spherical surface into true pendentive triangles,
/// so each web is honestly a restrained faceted ruled-loft solid. The lower
/// section stays over the corner pier/arch shoulders; successive sections
/// broaden inward until their upper chords overlap the drum footprint. This
/// preserves the legible load path without blocking the four crossing arches.
pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, roof: &RoofSection) {
    let web = context.add_part(
        names::parts::PENDENTIVE_WEB,
        faceted_pendentive_recipe(crossing_platform_base(roof)),
        "warm-stone",
    );
    for (key, angle) in [
        (names::instances::PENDENTIVE_NORTH_EAST, 0.0),
        (
            names::instances::PENDENTIVE_NORTH_WEST,
            core::f64::consts::FRAC_PI_2,
        ),
        (
            names::instances::PENDENTIVE_SOUTH_WEST,
            core::f64::consts::PI,
        ),
        (
            names::instances::PENDENTIVE_SOUTH_EAST,
            core::f64::consts::PI + core::f64::consts::FRAC_PI_2,
        ),
    ] {
        context.add_instance(
            key,
            web,
            Placement3::rotate_z_then_translate(angle, p.crossing_x, 0.0, 0.0),
            names::roles::PENDENTIVE,
        );
    }
}

fn faceted_pendentive_recipe(platform_base: f64) -> Recipe {
    ruled_loft_recipe(
        vec![
            (
                Placement3::translate(0.0, 0.0, WEB_BOTTOM),
                triangular_section(3.42),
            ),
            (
                Placement3::translate(0.0, 0.0, WEB_MIDDLE),
                triangular_section(2.38),
            ),
            (
                Placement3::translate(0.0, 0.0, platform_base + WEB_TOP_OVERLAP),
                triangular_section(1.42),
            ),
        ],
        "basilica:crossing-pendentive-web",
    )
}

fn triangular_section(inner: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((WEB_OUTER, WEB_OUTER)).tagged(SegTag(0)),
            Seg2::line((inner, WEB_OUTER)).tagged(SegTag(1)),
            Seg2::line((WEB_OUTER, inner)).tagged(SegTag(2)),
        ])
        .expect("pendentive section has three distinct corners"),
    )
    .expect("pendentive section winds counter-clockwise")
}

#[cfg(test)]
mod tests {
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::tessellate::EvalPolicy;

    use super::super::crossing_platform_base;
    use super::{WEB_BOTTOM, WEB_OUTER, WEB_TOP_OVERLAP, faceted_pendentive_recipe};
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{BasilicaParams, BasilicaRoofSetout, instance_id_at, names, role_instances};

    #[test]
    fn one_pendentive_is_a_clean_outward_oriented_solid() {
        let p = BasilicaParams::default();
        // The transition must meet the same roof-controlled bearing datum as
        // production geometry, rather than reconstructing it from parameters.
        let roof = BasilicaRoofSetout::new(&p).expect("default roof resolves");
        let platform_base = crossing_platform_base(roof.section());
        let evaluated = evaluate(
            &faceted_pendentive_recipe(platform_base),
            &EvalPolicy::default(),
        )
        .expect("fixed pendentive loft evaluates");
        assert!(evaluated.report.diagnostics.is_empty());
        assert_eq!(evaluated.bodies.len(), 1);
        let mesh = &evaluated.bodies[0].body.mesh;
        assert!(mesh.validate_fast().is_empty());
        assert!(mesh.validate_deep().is_empty());

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in mesh.vertices() {
            let position = mesh
                .vertex_position(vertex)
                .expect("live vertex has position");
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        assert!((f64::from(min[2]) - WEB_BOTTOM).abs() < 1.0e-5);
        assert!((f64::from(max[2]) - platform_base - WEB_TOP_OVERLAP).abs() < 1.0e-5);
        assert!((f64::from(max[0]) - WEB_OUTER).abs() < 1.0e-5);
        assert!((f64::from(max[1]) - WEB_OUTER).abs() < 1.0e-5);
    }

    #[test]
    fn four_named_webs_share_one_part_and_semantic_role() {
        let scenario = build_scenario();
        let pendentives = role_instances(&scenario.assembly, names::roles::PENDENTIVE);
        assert_eq!(pendentives.len(), 4);
        let shared_part = scenario
            .assembly
            .part_by_key(names::parts::PENDENTIVE_WEB)
            .expect("stable pendentive part key resolves");
        for path in [
            names::instances::PENDENTIVE_NORTH_EAST,
            names::instances::PENDENTIVE_NORTH_WEST,
            names::instances::PENDENTIVE_SOUTH_WEST,
            names::instances::PENDENTIVE_SOUTH_EAST,
        ] {
            let instance = instance_id_at(&scenario.assembly, path)
                .unwrap_or_else(|| panic!("missing named pendentive {path}"));
            assert_eq!(
                scenario.assembly.instance(instance).unwrap().part(),
                shared_part
            );
        }
    }

    #[test]
    fn webs_contact_the_bearing_datum_without_entering_the_crossing_floor() {
        let p = BasilicaParams::default();
        // This assertion guards the interface between setting-out and the
        // independently tessellated crossing platform.
        let roof = BasilicaRoofSetout::new(&p).expect("default roof resolves");
        let platform_base = crossing_platform_base(roof.section());
        let scenario = build_scenario();
        let (platform_min, _) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::CROSSING_PLATFORM,
        );
        assert!((platform_min[2] - platform_base).abs() < 1.0e-5);

        for path in [
            names::instances::PENDENTIVE_NORTH_EAST,
            names::instances::PENDENTIVE_NORTH_WEST,
            names::instances::PENDENTIVE_SOUTH_WEST,
            names::instances::PENDENTIVE_SOUTH_EAST,
        ] {
            let (min, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, path);
            assert!(
                (min[2] - WEB_BOTTOM).abs() < 1.0e-5,
                "{path} starts at its arch-shoulder section: {min:?}"
            );
            assert!(
                max[2] > platform_min[2] && max[2] < platform_min[2] + 0.1,
                "{path} must overlap only the underside of the bearing datum: {max:?}"
            );
        }

        let (north_east_min, _) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::PENDENTIVE_NORTH_EAST,
        );
        assert!(
            north_east_min[0] > p.crossing_x && north_east_min[1] > 0.0,
            "north-east web must leave the crossing center clear: {north_east_min:?}"
        );
        let (_, south_west_max) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::PENDENTIVE_SOUTH_WEST,
        );
        assert!(
            south_west_max[0] < p.crossing_x && south_west_max[1] < 0.0,
            "south-west web must leave the crossing center clear: {south_west_max:?}"
        );
    }
}
