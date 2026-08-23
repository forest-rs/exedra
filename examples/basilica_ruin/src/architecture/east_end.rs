// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::BasilicaParams;
use crate::geometry::{
    apse_conch_recipe, apse_shell_profile, east_chancel_profile, extruded_profile_recipe,
    transverse_wall_frame,
};
use crate::names;

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let half_nave = layout.half_nave;
    // A transverse chancel wall closes the nave roof end while a broad
    // round-headed opening keeps the apse spatially continuous.
    let east_chancel = context.add_part(
        names::parts::EAST_CHANCEL_GABLE,
        extruded_profile_recipe(
            east_chancel_profile(p),
            0.5,
            transverse_wall_frame(),
            "basilica:east-chancel-gable",
        ),
        "limestone",
    );
    context.add_instance(
        names::instances::EAST_CHANCEL_GABLE,
        east_chancel,
        Placement3::translate(p.length - 0.25, -half_nave, 0.0),
        "chancel_gable",
    );

    let apse = context.add_part(
        "apse",
        extruded_profile_recipe(
            apse_shell_profile(p.apse_radius, p.apse_radius - 0.45, 16),
            8.0,
            Placement3::IDENTITY,
            "basilica:east-apse",
        ),
        "limestone",
    );
    context.add_instance(
        names::instances::EAST_APSE,
        apse,
        Placement3::translate(p.length, 0.0, 0.0),
        "apse",
    );
    let apse_roof = context.add_part(
        "apse-roof",
        apse_conch_recipe(p.apse_radius + 0.18, 2.35),
        "aged-roof-tile",
    );
    context.add_instance(
        "east-apse-roof",
        apse_roof,
        Placement3::translate(p.length, 0.0, 8.0),
        "apse_roof",
    );
}

#[cfg(test)]
mod tests {
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::ir::Placement3;
    use exedra_constructive::tessellate::EvalPolicy;
    use exedra_math::{cross, dot, sub};

    use super::{apse_conch_recipe, apse_shell_profile};
    use crate::geometry::extruded_profile_recipe;
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{BasilicaParams, instances_with_role, names, resolve_instance_path};

    const WALL_HEIGHT: f64 = 8.0;
    const CONCH_HEIGHT: f64 = 2.35;
    const WALL_THICKNESS: f64 = 0.45;
    const CONCH_OVERHANG: f64 = 0.18;

    #[test]
    fn apse_profile_has_only_the_expected_inner_and_outer_axis_crossings() {
        let p = BasilicaParams::default();
        let inner_radius = p.apse_radius - WALL_THICKNESS;
        let profile = apse_shell_profile(p.apse_radius, inner_radius, 16);
        let mut axis_vertices: Vec<f64> = profile
            .outer()
            .segs()
            .iter()
            .filter(|segment| segment.to.y.abs() < 1.0e-12 && segment.to.x > 0.0)
            .map(|segment| segment.to.x)
            .collect();
        axis_vertices.sort_by(f64::total_cmp);

        assert_eq!(axis_vertices.len(), 2);
        assert!((axis_vertices[0] - inner_radius).abs() < 1.0e-12);
        assert!((axis_vertices[1] - p.apse_radius).abs() < 1.0e-12);
        assert!(
            inner_radius > 5.2 * 0.5,
            "the 5.2m chancel opening must see into the sanctuary before meeting the curved inner wall"
        );
        assert!(profile.holes().is_empty());
    }

    #[test]
    fn chancel_axis_reaches_the_curved_inner_sanctuary_wall() {
        let p = BasilicaParams::default();
        let scenario = build_scenario();
        let item = scenario
            .render_list
            .items
            .iter()
            .find(|item| item.path.to_string() == names::instances::EAST_APSE)
            .expect("existing apse render item");
        let body = &scenario
            .compiled
            .part(item.part)
            .expect("apse part is compiled")
            .bodies[item.body as usize];
        let origin = [p.length - 0.5, 0.0, WALL_HEIGHT * 0.5];
        let direction = [1.0, 0.0, 0.0];
        let first_hit = body
            .tri
            .indices
            .chunks_exact(3)
            .filter_map(|triangle| {
                let vertices = [triangle[0], triangle[1], triangle[2]].map(|index| {
                    transform_point(
                        &item.world,
                        body.tri.positions[usize::try_from(index).expect("triangle index fits")],
                    )
                });
                ray_triangle_distance(origin, direction, vertices)
            })
            .min_by(f64::total_cmp)
            .expect("the axial ray eventually meets the curved apse wall");
        let expected = 0.5 + p.apse_radius - WALL_THICKNESS;

        assert!(
            (first_hit - expected).abs() < 1.0e-4,
            "the centerline should pass through the chancel and sanctuary before meeting the inner apse wall: hit={first_hit}, expected={expected}"
        );
        assert!(
            first_hit > 5.2 * 0.5,
            "the old filled apse hit immediately at its diameter face"
        );
    }

    #[test]
    fn apse_wall_and_conch_are_clean_outward_oriented_solids() {
        let p = BasilicaParams::default();
        let recipes = [
            extruded_profile_recipe(
                apse_shell_profile(p.apse_radius, p.apse_radius - WALL_THICKNESS, 16),
                WALL_HEIGHT,
                Placement3::IDENTITY,
                "test:basilica:east-apse",
            ),
            apse_conch_recipe(p.apse_radius + CONCH_OVERHANG, CONCH_HEIGHT),
        ];

        for recipe in recipes {
            let evaluated = evaluate(&recipe, &EvalPolicy::default())
                .expect("fixed-correspondence apse recipe evaluates");
            assert!(evaluated.report.diagnostics.is_empty());
            assert_eq!(evaluated.bodies.len(), 1);
            let mesh = &evaluated.bodies[0].body.mesh;
            assert!(mesh.validate_fast().is_empty());
            assert!(mesh.validate_deep().is_empty());
        }
    }

    #[test]
    fn apse_shells_preserve_the_exterior_and_meet_at_the_wall_head() {
        let p = BasilicaParams::default();
        let scenario = build_scenario();
        let (wall_min, wall_max) = bounds_for_path(
            &scenario.compiled,
            &scenario.render_list,
            names::instances::EAST_APSE,
        );
        let (conch_min, conch_max) =
            bounds_for_path(&scenario.compiled, &scenario.render_list, "east-apse-roof");

        assert_close(wall_min, [p.length, -p.apse_radius, 0.0]);
        assert_close(
            wall_max,
            [p.length + p.apse_radius, p.apse_radius, WALL_HEIGHT],
        );
        let conch_radius = p.apse_radius + CONCH_OVERHANG;
        assert_close(conch_min, [p.length, -conch_radius, WALL_HEIGHT]);
        assert_close(
            conch_max,
            [
                p.length + conch_radius,
                conch_radius,
                WALL_HEIGHT + CONCH_HEIGHT,
            ],
        );
        assert!((wall_max[2] - conch_min[2]).abs() < 1.0e-5);
        assert!(
            p.apse_radius > conch_radius - 0.38,
            "the wall and conch shells need radial bearing overlap"
        );
    }

    #[test]
    fn existing_apse_names_roles_and_parts_still_resolve() {
        let scenario = build_scenario();
        let apse = resolve_instance_path(&scenario.assembly, names::instances::EAST_APSE)
            .expect("existing apse path resolves");
        let conch = resolve_instance_path(&scenario.assembly, "east-apse-roof")
            .expect("existing apse roof path resolves");
        let apse_part = scenario
            .assembly
            .part_by_key("apse")
            .expect("existing apse part key resolves");
        let conch_part = scenario
            .assembly
            .part_by_key("apse-roof")
            .expect("existing apse roof part key resolves");

        assert_eq!(scenario.assembly.instance(apse).unwrap().part(), apse_part);
        assert_eq!(
            scenario.assembly.instance(conch).unwrap().part(),
            conch_part
        );
        assert_eq!(instances_with_role(&scenario.assembly, "apse"), [apse]);
        assert_eq!(
            instances_with_role(&scenario.assembly, "apse_roof"),
            [conch]
        );
    }

    fn assert_close(actual: [f64; 3], expected: [f64; 3]) {
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() < 1.0e-5,
                "axis {axis}: {actual:?} != {expected:?}"
            );
        }
    }

    fn transform_point(placement: &Placement3, point: [f32; 3]) -> [f64; 3] {
        let point = point.map(f64::from);
        let rows = &placement.rows;
        [
            rows[0][0] * point[0] + rows[0][1] * point[1] + rows[0][2] * point[2] + rows[0][3],
            rows[1][0] * point[0] + rows[1][1] * point[1] + rows[1][2] * point[2] + rows[1][3],
            rows[2][0] * point[0] + rows[2][1] * point[1] + rows[2][2] * point[2] + rows[2][3],
        ]
    }

    fn ray_triangle_distance(
        origin: [f64; 3],
        direction: [f64; 3],
        triangle: [[f64; 3]; 3],
    ) -> Option<f64> {
        let edge_a = sub(triangle[1], triangle[0]);
        let edge_b = sub(triangle[2], triangle[0]);
        let perpendicular = cross(direction, edge_b);
        let determinant = dot(edge_a, perpendicular);
        if determinant.abs() < 1.0e-10 {
            return None;
        }

        let inverse = determinant.recip();
        let from_vertex = sub(origin, triangle[0]);
        let u = inverse * dot(from_vertex, perpendicular);
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let second_perpendicular = cross(from_vertex, edge_a);
        let v = inverse * dot(direction, second_perpendicular);
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let distance = inverse * dot(edge_b, second_perpendicular);
        (distance > 1.0e-8).then_some(distance)
    }
}
