// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use cambium::assembly::{
    InstanceTemplate, LinearRepeat, MetadataEntry, NamedAssemblyPattern, repeat_linear,
};
use exedra_assembly::PartId;
use exedra_constructive::ir::Placement3;

use super::{BuildContext, Layout};
use crate::geometry::box_recipe;
use crate::{BasilicaParams, names};

const WEST_BAYS: u32 = 5;
const OMITTED_WEST_SLOT: u32 = 2;
const TIE_WIDTH: f64 = 0.30;
const TIE_DEPTH: f64 = 0.34;
const TIE_BASE: f64 = 10.90;
const RAFTER_WIDTH: f64 = 0.26;
const RAFTER_DEPTH: f64 = 0.24;
// Keep the structural pair visibly separate from the modeled roof skin. A
// near-flush 35 mm gap made the near-side rafter disappear in oblique views.
const ROOF_CLEARANCE: f64 = 0.12;
const KING_POST_WIDTH: f64 = 0.26;
const KING_POST_BASE: f64 = 11.10;
const KING_POST_RAFTER_OVERLAP: f64 = 0.025;
const BRACE_RUN: f64 = 2.40;
const BRACE_WIDTH: f64 = 0.20;
const BRACE_DEPTH: f64 = 0.18;
const BRACE_BASE: f64 = 11.22;

struct MemberParts {
    tie: PartId,
    rafter: PartId,
    king_post: PartId,
    brace: PartId,
}

struct TrussGeometry {
    roof_sin: f64,
    roof_cos: f64,
    roof_peak: f64,
    rafter_length: f64,
    king_post_height: f64,
    brace_length: f64,
    brace_cos: f64,
    brace_sin: f64,
}

/// Adds the open timber frames that visibly carry the two nave roof slopes.
///
/// The tie beam also acts as a wall plate: the existing roof planes use their
/// exterior overhang as the slope run, leaving their underside slightly above
/// the nave wall heads. Its depth bridges that gap while tying the walls
/// against rafter spread. The missing west slot repeats the authored south-west
/// clerestory and roof loss rather than placing intact structure beneath it.
pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let geometry = TrussGeometry::from_params(p, layout);
    let parts = add_member_parts(context, p, &geometry);
    let west_pitch = (layout.crossing_west - 4.0) / f64::from(WEST_BAYS);
    let role = [MetadataEntry {
        key: names::ARCHITECTURAL_ROLE,
        value: names::roles::NAVE_TRUSS_MEMBER,
    }];
    let members = member_templates(&parts, &geometry, layout, &role);
    let west_occurrences = repeat_linear(&LinearRepeat {
        count: WEST_BAYS + 1,
        start: [2.0, 0.0, 0.0],
        step: [west_pitch, 0.0, 0.0],
        omitted: &[OMITTED_WEST_SLOT],
    })
    .expect("accepted west truss repeat must be finite");
    context.instantiate_pattern(
        &NamedAssemblyPattern {
            parent: None,
            key_prefix: "nave-truss-west",
            ordinal_width: 2,
            members: &members,
        },
        &west_occurrences,
    );

    let east_x = (layout.crossing_east + p.length) * 0.5;
    let east_occurrences = repeat_linear(&LinearRepeat {
        count: 1,
        start: [east_x, 0.0, 0.0],
        step: [0.0, 0.0, 0.0],
        omitted: &[],
    })
    .expect("accepted east truss occurrence must be finite");
    context.instantiate_pattern(
        &NamedAssemblyPattern {
            parent: None,
            key_prefix: "nave-truss-east",
            ordinal_width: 2,
            members: &members,
        },
        &east_occurrences,
    );
}

impl TrussGeometry {
    fn from_params(p: &BasilicaParams, layout: Layout) -> Self {
        let roof_run = layout.half_nave + 0.35;
        let roof_slope = libm::sqrt(roof_run * roof_run + p.roof_rise * p.roof_rise);
        let roof_cos = roof_run / roof_slope;
        let roof_sin = p.roof_rise / roof_slope;
        let roof_peak = p.nave_wall_height + p.roof_rise;
        let rafter_length = layout.half_nave / roof_cos;
        let rafter_lower_ridge = roof_peak - roof_cos * (RAFTER_DEPTH + ROOF_CLEARANCE);
        let king_post_height = rafter_lower_ridge + KING_POST_RAFTER_OVERLAP - KING_POST_BASE;

        let brace_target_x = (BRACE_RUN + roof_sin * (RAFTER_DEPTH + ROOF_CLEARANCE)) / roof_cos;
        let brace_target_z =
            roof_peak - roof_sin * brace_target_x - roof_cos * (RAFTER_DEPTH + ROOF_CLEARANCE);
        let brace_rise = brace_target_z - BRACE_BASE;
        let brace_length = libm::sqrt(BRACE_RUN * BRACE_RUN + brace_rise * brace_rise);

        Self {
            roof_sin,
            roof_cos,
            roof_peak,
            rafter_length,
            king_post_height,
            brace_length,
            brace_cos: BRACE_RUN / brace_length,
            brace_sin: brace_rise / brace_length,
        }
    }
}

fn add_member_parts(
    context: &mut BuildContext,
    p: &BasilicaParams,
    geometry: &TrussGeometry,
) -> MemberParts {
    MemberParts {
        tie: context.add_part(
            names::parts::NAVE_TRUSS_TIE_BEAM,
            box_recipe(
                [p.nave_width, TIE_WIDTH, TIE_DEPTH],
                "basilica:nave-truss-tie-beam",
            ),
            "aged-timber",
        ),
        rafter: context.add_part(
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
            box_recipe(
                [geometry.rafter_length, RAFTER_WIDTH, RAFTER_DEPTH],
                "basilica:nave-truss-principal-rafter",
            ),
            "aged-timber",
        ),
        king_post: context.add_part(
            names::parts::NAVE_TRUSS_KING_POST,
            box_recipe(
                [KING_POST_WIDTH, KING_POST_WIDTH, geometry.king_post_height],
                "basilica:nave-truss-king-post",
            ),
            "aged-timber",
        ),
        brace: context.add_part(
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
            box_recipe(
                [geometry.brace_length, BRACE_WIDTH, BRACE_DEPTH],
                "basilica:nave-truss-diagonal-brace",
            ),
            "aged-timber",
        ),
    }
}

fn member_templates<'metadata>(
    parts: &MemberParts,
    geometry: &TrussGeometry,
    layout: Layout,
    metadata: &'metadata [MetadataEntry<'metadata>],
) -> [InstanceTemplate<'metadata>; 6] {
    [
        InstanceTemplate {
            key_suffix: "tie-beam",
            part: parts.tie,
            placement: tie_frame(layout.half_nave),
            bindings: &[],
            metadata,
        },
        InstanceTemplate {
            key_suffix: "principal-rafter-north",
            part: parts.rafter,
            placement: rafter_frame(geometry, true),
            bindings: &[],
            metadata,
        },
        InstanceTemplate {
            key_suffix: "principal-rafter-south",
            part: parts.rafter,
            placement: rafter_frame(geometry, false),
            bindings: &[],
            metadata,
        },
        InstanceTemplate {
            key_suffix: "king-post",
            part: parts.king_post,
            placement: Placement3::translate(
                -KING_POST_WIDTH * 0.5,
                -KING_POST_WIDTH * 0.5,
                KING_POST_BASE,
            ),
            bindings: &[],
            metadata,
        },
        InstanceTemplate {
            key_suffix: "diagonal-brace-north",
            part: parts.brace,
            placement: brace_frame(geometry, true),
            bindings: &[],
            metadata,
        },
        InstanceTemplate {
            key_suffix: "diagonal-brace-south",
            part: parts.brace,
            placement: brace_frame(geometry, false),
            bindings: &[],
            metadata,
        },
    ]
}

fn tie_frame(half_nave: f64) -> Placement3 {
    Placement3::from_axes(
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [TIE_WIDTH * 0.5, -half_nave, TIE_BASE],
    )
}

fn rafter_frame(geometry: &TrussGeometry, north: bool) -> Placement3 {
    let offset = RAFTER_DEPTH + ROOF_CLEARANCE;
    if north {
        Placement3::from_axes(
            [0.0, geometry.roof_cos, -geometry.roof_sin],
            [-1.0, 0.0, 0.0],
            [0.0, geometry.roof_sin, geometry.roof_cos],
            [
                RAFTER_WIDTH * 0.5,
                -geometry.roof_sin * offset,
                geometry.roof_peak - geometry.roof_cos * offset,
            ],
        )
    } else {
        Placement3::from_axes(
            [0.0, -geometry.roof_cos, -geometry.roof_sin],
            [1.0, 0.0, 0.0],
            [0.0, -geometry.roof_sin, geometry.roof_cos],
            [
                -RAFTER_WIDTH * 0.5,
                geometry.roof_sin * offset,
                geometry.roof_peak - geometry.roof_cos * offset,
            ],
        )
    }
}

fn brace_frame(geometry: &TrussGeometry, north: bool) -> Placement3 {
    if north {
        Placement3::from_axes(
            [0.0, geometry.brace_cos, geometry.brace_sin],
            [-1.0, 0.0, 0.0],
            [0.0, -geometry.brace_sin, geometry.brace_cos],
            [BRACE_WIDTH * 0.5, 0.0, BRACE_BASE],
        )
    } else {
        Placement3::from_axes(
            [0.0, -geometry.brace_cos, geometry.brace_sin],
            [1.0, 0.0, 0.0],
            [0.0, geometry.brace_sin, geometry.brace_cos],
            [-BRACE_WIDTH * 0.5, 0.0, BRACE_BASE],
        )
    }
}

#[cfg(test)]
mod tests {
    use exedra_assembly::{PartSource, assembly_fingerprint};
    use exedra_constructive::evaluate::evaluate;
    use exedra_constructive::tessellate::EvalPolicy;

    use super::*;
    use crate::output::{bounds_for_path, build_scenario};
    use crate::{instances_with_role, resolve_instance_path};

    const FRAME_PREFIXES: [&str; 6] = [
        "nave-truss-west-00",
        "nave-truss-west-01",
        "nave-truss-west-03",
        "nave-truss-west-04",
        "nave-truss-west-05",
        "nave-truss-east-00",
    ];

    #[test]
    fn named_patterns_match_the_accepted_station_loops() {
        let p = BasilicaParams::default();
        let layout = Layout::from_params(&p);
        let geometry = TrussGeometry::from_params(&p, layout);

        let mut patterned = BuildContext::new();
        build(&mut patterned, &p, layout);

        let mut legacy = BuildContext::new();
        let parts = add_member_parts(&mut legacy, &p, &geometry);
        let west_pitch = (layout.crossing_west - 4.0) / f64::from(WEST_BAYS);
        for slot in 0..=WEST_BAYS {
            if slot != OMITTED_WEST_SLOT {
                add_legacy_frame(
                    &mut legacy,
                    &parts,
                    &geometry,
                    layout,
                    "west",
                    slot,
                    2.0 + f64::from(slot) * west_pitch,
                );
            }
        }
        add_legacy_frame(
            &mut legacy,
            &parts,
            &geometry,
            layout,
            "east",
            0,
            (layout.crossing_east + p.length) * 0.5,
        );

        let patterned = patterned.finish();
        let legacy = legacy.finish();
        assert_eq!(patterned.instances().len(), 36);
        assert_eq!(
            assembly_fingerprint(&patterned),
            assembly_fingerprint(&legacy)
        );
    }

    fn add_legacy_frame(
        context: &mut BuildContext,
        parts: &MemberParts,
        geometry: &TrussGeometry,
        layout: Layout,
        segment: &str,
        slot: u32,
        x: f64,
    ) {
        let prefix = format!("nave-truss-{segment}-{slot:02}");
        for (suffix, part, placement) in [
            (
                "tie-beam",
                parts.tie,
                legacy_station_placement(x, tie_frame(layout.half_nave)),
            ),
            (
                "principal-rafter-north",
                parts.rafter,
                legacy_station_placement(x, rafter_frame(geometry, true)),
            ),
            (
                "principal-rafter-south",
                parts.rafter,
                legacy_station_placement(x, rafter_frame(geometry, false)),
            ),
            (
                "king-post",
                parts.king_post,
                Placement3::translate(
                    x - KING_POST_WIDTH * 0.5,
                    -KING_POST_WIDTH * 0.5,
                    KING_POST_BASE,
                ),
            ),
            (
                "diagonal-brace-north",
                parts.brace,
                legacy_station_placement(x, brace_frame(geometry, true)),
            ),
            (
                "diagonal-brace-south",
                parts.brace,
                legacy_station_placement(x, brace_frame(geometry, false)),
            ),
        ] {
            context.add_instance(
                &format!("{prefix}-{suffix}"),
                part,
                placement,
                names::roles::NAVE_TRUSS_MEMBER,
            );
        }
    }

    fn legacy_station_placement(x: f64, mut placement: Placement3) -> Placement3 {
        placement.rows[0][3] += x;
        placement
    }

    #[test]
    fn named_members_share_four_parts_and_one_role() {
        let scenario = build_scenario();
        let members = instances_with_role(&scenario.assembly, names::roles::NAVE_TRUSS_MEMBER);
        assert_eq!(members.len(), 36);

        for (key, expected_count) in [
            (names::parts::NAVE_TRUSS_TIE_BEAM, 6),
            (names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER, 12),
            (names::parts::NAVE_TRUSS_KING_POST, 6),
            (names::parts::NAVE_TRUSS_DIAGONAL_BRACE, 12),
        ] {
            let part = scenario
                .assembly
                .part_by_key(key)
                .unwrap_or_else(|| panic!("missing stable truss part {key}"));
            assert_eq!(
                members
                    .iter()
                    .filter(|&&id| scenario.assembly.instance(id).unwrap().part() == part)
                    .count(),
                expected_count
            );
        }

        for id in members {
            let instance = scenario.assembly.instance(id).unwrap();
            let part = scenario.assembly.part(instance.part()).unwrap();
            let surface = part.slot_index("surface").expect("truss surface slot");
            assert_eq!(
                scenario.assembly.resolved_material(id, surface),
                Some("aged-timber")
            );
        }
    }

    #[test]
    fn member_recipes_are_clean_solids_and_placements_are_rigid() {
        let scenario = build_scenario();
        for key in [
            names::parts::NAVE_TRUSS_TIE_BEAM,
            names::parts::NAVE_TRUSS_PRINCIPAL_RAFTER,
            names::parts::NAVE_TRUSS_KING_POST,
            names::parts::NAVE_TRUSS_DIAGONAL_BRACE,
        ] {
            let part = scenario
                .assembly
                .part(
                    scenario
                        .assembly
                        .part_by_key(key)
                        .expect("stable truss part exists"),
                )
                .unwrap();
            let PartSource::Recipe(recipe) = part.source() else {
                panic!("truss members stay recipe-backed");
            };
            let evaluated = evaluate(recipe, &EvalPolicy::default()).expect("box recipe evaluates");
            assert!(evaluated.report.diagnostics.is_empty());
            assert_eq!(evaluated.bodies.len(), 1);
            assert!(evaluated.bodies[0].body.mesh.validate_deep().is_empty());
        }

        for id in instances_with_role(&scenario.assembly, names::roles::NAVE_TRUSS_MEMBER) {
            let placement = scenario.assembly.instance(id).unwrap().placement();
            assert!(
                placement
                    .rows
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite()),
                "truss placement must be finite"
            );
            assert!(
                (rotation_determinant(placement) - 1.0).abs() < 1.0e-12,
                "truss placement must be proper rigid: {:?}",
                placement.rows
            );
        }
    }

    #[test]
    fn rafters_clear_the_roof_underside_and_bear_through_the_ties() {
        let p = BasilicaParams::default();
        let scenario = build_scenario();
        let roof_run = p.nave_width * 0.5 + 0.35;
        let roof_slope = libm::sqrt(roof_run * roof_run + p.roof_rise * p.roof_rise);
        let roof_sin = p.roof_rise / roof_slope;
        let roof_cos = roof_run / roof_slope;
        let roof_peak = p.nave_wall_height + p.roof_rise;

        for prefix in FRAME_PREFIXES {
            let tie_path = format!("{prefix}-tie-beam");
            let (tie_min, tie_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &tie_path);
            assert_close(tie_min[1], -p.nave_width * 0.5);
            assert_close(tie_max[1], p.nave_width * 0.5);
            assert!(tie_min[2] < p.nave_wall_height && tie_max[2] > p.nave_wall_height);

            for side in ["north", "south"] {
                let path = format!("{prefix}-principal-rafter-{side}");
                let item = render_item(&scenario, &path);
                let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
                let normal_y = if side == "north" { roof_sin } else { -roof_sin };
                let max_plane_distance = body
                    .tri
                    .positions
                    .iter()
                    .map(|&position| transform_point(&item.world, position))
                    .map(|position| {
                        normal_y * position[1] + roof_cos * position[2] - roof_cos * roof_peak
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                assert!(
                    max_plane_distance <= -ROOF_CLEARANCE + 1.0e-5,
                    "{path} protrudes into the roof: {max_plane_distance}"
                );
                assert!(
                    (max_plane_distance + ROOF_CLEARANCE).abs() < 1.0e-5,
                    "{path} must retain the designed roof clearance: {max_plane_distance}"
                );
                let (rafter_min, _) =
                    bounds_for_path(&scenario.compiled, &scenario.render_list, &path);
                assert!(
                    rafter_min[2] < tie_max[2] && rafter_min[2] > tie_min[2],
                    "{path} must overlap the tie bearing: {rafter_min:?} vs {tie_min:?}..{tie_max:?}"
                );
            }

            let (king_min, king_max) = bounds_for_path(
                &scenario.compiled,
                &scenario.render_list,
                &format!("{prefix}-king-post"),
            );
            assert!(king_min[2] < tie_max[2]);
            assert!(king_max[2] > roof_peak - roof_cos * (RAFTER_DEPTH + ROOF_CLEARANCE));

            for side in ["north", "south"] {
                let (brace_min, brace_max) = bounds_for_path(
                    &scenario.compiled,
                    &scenario.render_list,
                    &format!("{prefix}-diagonal-brace-{side}"),
                );
                assert!(brace_min[2] < tie_max[2]);
                assert!(brace_max[2] > BRACE_BASE + 1.0);
            }
        }
    }

    #[test]
    fn every_intact_frame_has_a_mirrored_rafter_pair_with_visible_roof_reveal() {
        const MIN_VISIBLE_ROOF_REVEAL: f64 = 0.10;
        const MEMBER_SUFFIXES: [&str; 6] = [
            "tie-beam",
            "principal-rafter-north",
            "principal-rafter-south",
            "king-post",
            "diagonal-brace-north",
            "diagonal-brace-south",
        ];

        let p = BasilicaParams::default();
        let scenario = build_scenario();
        let roof_run = p.nave_width * 0.5 + 0.35;
        let roof_slope = libm::sqrt(roof_run * roof_run + p.roof_rise * p.roof_rise);
        let roof_sin = p.roof_rise / roof_slope;
        let roof_cos = roof_run / roof_slope;
        let roof_peak = p.nave_wall_height + p.roof_rise;

        for prefix in FRAME_PREFIXES {
            let north_path = format!("{prefix}-principal-rafter-north");
            let south_path = format!("{prefix}-principal-rafter-south");
            let north_id = resolve_instance_path(&scenario.assembly, &north_path)
                .unwrap_or_else(|| panic!("missing paired north rafter {north_path}"));
            let south_id = resolve_instance_path(&scenario.assembly, &south_path)
                .unwrap_or_else(|| panic!("missing paired south rafter {south_path}"));
            assert_eq!(
                scenario.assembly.instance(north_id).unwrap().part(),
                scenario.assembly.instance(south_id).unwrap().part(),
                "{prefix} rafter pair must share the principal-rafter part"
            );

            let (north_min, north_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &north_path);
            let (south_min, south_max) =
                bounds_for_path(&scenario.compiled, &scenario.render_list, &south_path);
            assert_close(north_min[0], south_min[0]);
            assert_close(north_max[0], south_max[0]);
            assert_close(north_min[1], -south_max[1]);
            assert_close(north_max[1], -south_min[1]);
            assert_close(north_min[2], south_min[2]);
            assert_close(north_max[2], south_max[2]);

            for (path, normal_y) in [(&north_path, roof_sin), (&south_path, -roof_sin)] {
                let item = render_item(&scenario, path);
                let body = &scenario.compiled.part(item.part).unwrap().bodies[item.body as usize];
                let max_plane_distance = body
                    .tri
                    .positions
                    .iter()
                    .map(|&position| transform_point(&item.world, position))
                    .map(|position| {
                        normal_y * position[1] + roof_cos * position[2] - roof_cos * roof_peak
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                let reveal = -max_plane_distance;
                assert!(
                    reveal >= MIN_VISIBLE_ROOF_REVEAL,
                    "{path} is present but visually swallowed by the roof: reveal={reveal}m"
                );
                assert_close(reveal, ROOF_CLEARANCE);
            }
        }

        for suffix in MEMBER_SUFFIXES {
            let omitted_path = format!("nave-truss-west-02-{suffix}");
            assert!(
                resolve_instance_path(&scenario.assembly, &omitted_path).is_none(),
                "only the authored west-02 frame may be omitted: {omitted_path}"
            );
        }
    }

    #[test]
    fn truss_stations_preserve_the_ruin_and_crossing_voids() {
        let p = BasilicaParams::default();
        let crossing_west = p.crossing_x - p.drum_radius - 0.6;
        let crossing_east = p.crossing_x + p.drum_radius + 0.6;
        let scenario = build_scenario();
        assert!(resolve_instance_path(&scenario.assembly, "nave-truss-west-02-tie-beam").is_none());

        for item in &scenario.render_list.items[71..] {
            let path = item.path.to_string();
            let (min, max) = bounds_for_path(&scenario.compiled, &scenario.render_list, &path);
            if path.starts_with("nave-truss-west") {
                assert!(
                    max[0] <= 7.15 || min[0] >= 10.75,
                    "{path} enters the authored roof ruin: {min:?}..{max:?}"
                );
                assert!(max[0] < crossing_west, "{path} enters the crossing");
            } else {
                assert!(path.starts_with("nave-truss-east-00"));
                assert!(min[0] > crossing_east, "{path} enters the crossing");
            }
        }
    }

    fn render_item<'a>(
        scenario: &'a crate::output::Scenario,
        path: &str,
    ) -> &'a exedra_assembly::RenderItem {
        scenario
            .render_list
            .items
            .iter()
            .find(|item| item.path.to_string() == path)
            .unwrap_or_else(|| panic!("missing render item {path}"))
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

    fn rotation_determinant(placement: &Placement3) -> f64 {
        let r = &placement.rows;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
    }
}
