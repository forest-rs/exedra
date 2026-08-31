// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, LoftPolicy, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegTag};
use exedra_math::scale;
use setout_joiner::lower_point;

use crate::{AisleSection, LevelSection, PlanSection, RoofSection, RoofSide};

const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;

pub(super) fn extruded_profile_recipe(
    profile: Profile2,
    height: f64,
    placement: Placement3,
    source_name: &str,
) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref(source_name);
    let surface = builder.material_slot("surface");
    let profile = builder.add_profile(profile);
    let node = builder
        .with_source(source)
        .with_material(surface)
        .add(NodeKind::Extrude {
            profile,
            placement,
            height,
            caps: CapMode::Both,
        })
        .expect("constant extrusion is valid");
    builder.finish(node).expect("recipe has a valid root")
}

pub(super) fn box_recipe(size: [f64; 3], source_name: &str) -> Recipe {
    extruded_profile_recipe(
        builders::rect(size[0], size[1]).expect("positive box footprint"),
        size[2],
        Placement3::IDENTITY,
        source_name,
    )
}

pub(super) fn cylinder_recipe(
    radius: f64,
    height: f64,
    segments: u32,
    source_name: &str,
) -> Recipe {
    extruded_profile_recipe(
        polygon_profile(radius, segments),
        height,
        Placement3::IDENTITY,
        source_name,
    )
}

pub(super) fn ruled_loft_recipe(
    sections: Vec<(Placement3, Profile2)>,
    source_name: &str,
) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref(source_name);
    let surface = builder.material_slot("surface");
    let sections = sections
        .into_iter()
        .map(|(placement, profile)| (placement, builder.add_profile(profile)))
        .collect();
    let node = builder
        .with_source(source)
        .with_material(surface)
        .add(NodeKind::Loft {
            sections,
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .expect("fixed-correspondence ruled loft is valid");
    builder.finish(node).expect("recipe has a valid root")
}

pub(super) fn dome_recipe(radius: f64, height: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("basilica:crossing-dome");
    let surface = builder.material_slot("surface");
    let rings = [
        (radius, 0.0),
        (radius * 0.98, height * 0.16),
        (radius * 0.82, height * 0.52),
        (radius * 0.5, height * 0.82),
        (radius * 0.13, height),
    ];
    let sections = rings
        .into_iter()
        .map(|(ring_radius, z)| {
            let profile = builder.add_profile(polygon_profile(ring_radius, 24));
            (Placement3::translate(0.0, 0.0, z), profile)
        })
        .collect();
    let node = builder
        .with_source(source)
        .with_material(surface)
        .add(NodeKind::Loft {
            sections,
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .expect("valid shallow dome loft");
    builder.finish(node).expect("recipe has a valid root")
}

pub(super) fn apse_conch_recipe(radius: f64, height: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("basilica:east-apse-roof");
    let surface = builder.material_slot("surface");
    let rings = [
        (radius, radius - 0.38, 0.0),
        (radius * 0.98, radius * 0.98 - 0.37, height * 0.18),
        (radius * 0.82, radius * 0.82 - 0.34, height * 0.48),
        (radius * 0.52, radius * 0.52 - 0.28, height * 0.78),
        (radius * 0.32, radius * 0.13, height * 0.90),
        (radius * 0.15, 0.01, height),
    ];
    let sections = rings
        .into_iter()
        .map(|(outer_radius, inner_radius, z)| {
            let profile = builder.add_profile(apse_shell_profile(outer_radius, inner_radius, 16));
            (Placement3::translate(0.0, 0.0, z), profile)
        })
        .collect();
    let node = builder
        .with_source(source)
        .with_material(surface)
        .add(NodeKind::Loft {
            sections,
            policy: LoftPolicy::Ruled,
            caps: CapMode::Both,
        })
        .expect("valid apsidal conch loft");
    builder.finish(node).expect("recipe has a valid root")
}

pub(super) fn polygon_profile(radius: f64, segments: u32) -> Profile2 {
    let points = (0..segments)
        .map(|index| {
            let angle = core::f64::consts::TAU * f64::from(index) / f64::from(segments);
            Seg2::line((radius * libm::cos(angle), radius * libm::sin(angle))).tagged(SegTag(index))
        })
        .collect();
    Profile2::simple(Loop2::new(points).expect("regular polygon has distinct points"))
        .expect("regular polygon winds counter-clockwise")
}

pub(super) fn square_polygon_ring_profile(
    half_extent: f64,
    inner_radius: f64,
    inner_segments: u32,
) -> Profile2 {
    let outer = Loop2::new(vec![
        Seg2::line((half_extent, -half_extent)).tagged(SegTag(0)),
        Seg2::line((half_extent, half_extent)).tagged(SegTag(1)),
        Seg2::line((-half_extent, half_extent)).tagged(SegTag(2)),
        Seg2::line((-half_extent, -half_extent)).tagged(SegTag(3)),
    ])
    .expect("square bearing perimeter has distinct corners");
    let inner = Loop2::new(
        (0..inner_segments)
            .map(|index| {
                let angle = -core::f64::consts::TAU * f64::from(index) / f64::from(inner_segments);
                Seg2::line((
                    inner_radius * libm::cos(angle),
                    inner_radius * libm::sin(angle),
                ))
                .tagged(SegTag(index))
            })
            .collect(),
    )
    .expect("polygonal bearing opening has distinct corners");
    Profile2::new(outer, vec![inner]).expect("bearing opening lies inside square perimeter")
}

pub(super) fn arcaded_wall_profile(
    length: f64,
    height: f64,
    bays: u32,
    opening_width: f64,
    sill: f64,
    spring: f64,
    ruin_notch: Option<(f64, f64, f64)>,
) -> Profile2 {
    let outer = match ruin_notch {
        Some((left, right, depth)) => Loop2::new(vec![
            Seg2::line((length, 0.0)).tagged(SegTag(0)),
            Seg2::line((length, height)).tagged(SegTag(1)),
            Seg2::line((right, height)).tagged(SegTag(2)),
            Seg2::line((right, height - depth)).tagged(SegTag(3)),
            Seg2::line((left, height - depth)).tagged(SegTag(4)),
            Seg2::line((left, height)).tagged(SegTag(5)),
            Seg2::line((0.0, height)).tagged(SegTag(6)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(7)),
        ])
        .expect("valid ruined wall perimeter"),
        None => Loop2::new(vec![
            Seg2::line((length, 0.0)).tagged(SegTag(0)),
            Seg2::line((length, height)).tagged(SegTag(1)),
            Seg2::line((0.0, height)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("valid wall perimeter"),
    };
    let pitch = (length - 4.0) / f64::from(bays);
    let holes = (0..bays)
        .map(|bay| {
            let center = 2.0 + (f64::from(bay) + 0.5) * pitch;
            round_head_hole(center, opening_width, sill, spring)
        })
        .collect();
    Profile2::new(outer, holes).expect("arcade holes stay inside the wall")
}

pub(super) fn round_head_hole(center: f64, width: f64, sill: f64, spring: f64) -> Loop2 {
    let left = center - width * 0.5;
    let right = center + width * 0.5;
    Loop2::new(vec![
        Seg2::line((left, spring)).tagged(SegTag(0)),
        Seg2::arc((right, spring), -1.0).tagged(SegTag(1)),
        Seg2::line((right, sill)).tagged(SegTag(2)),
        Seg2::line((left, sill)).tagged(SegTag(3)),
    ])
    .expect("valid round-headed opening")
}

pub(super) fn drum_panel_profile(width: f64, height: f64, window: bool) -> Profile2 {
    let outer = builders::rect(width, height).expect("positive drum panel");
    if !window {
        return outer;
    }
    let hole = round_head_hole(width * 0.5, width * 0.38, 0.45, 1.45);
    Profile2::new(outer.outer().clone(), vec![hole]).expect("drum window lies inside panel")
}

pub(super) fn west_facade_profile(
    plan: &PlanSection,
    levels: &LevelSection,
    roof: &RoofSection,
) -> Profile2 {
    let half_nave = roof.half_span.as_meters();
    let half_total = plan.half_total.as_meters();
    let aisle_wall_top = levels.aisle_wall_top.as_meters();
    let wall_plate_top = roof.wall_plate_top.as_meters();
    let peak = roof.ridge_height.as_meters();
    let outer = Loop2::new(vec![
        Seg2::line((half_total, 0.0)).tagged(SegTag(0)),
        Seg2::line((half_total, aisle_wall_top)).tagged(SegTag(1)),
        Seg2::line((half_nave, aisle_wall_top)).tagged(SegTag(2)),
        Seg2::line((half_nave, wall_plate_top)).tagged(SegTag(3)),
        Seg2::line((0.0, peak)).tagged(SegTag(4)),
        Seg2::line((-half_nave, wall_plate_top)).tagged(SegTag(5)),
        Seg2::line((-half_nave, aisle_wall_top)).tagged(SegTag(6)),
        Seg2::line((-half_total, aisle_wall_top)).tagged(SegTag(7)),
        Seg2::line((-half_total, 0.0)).tagged(SegTag(8)),
    ])
    .expect("valid stepped basilica facade");
    let entrance = round_head_hole(0.0, 3.0, 0.2, 3.6);
    Profile2::new(outer, vec![entrance]).expect("entrance lies inside the facade")
}

pub(super) fn gable_profile(width: f64, rise: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((width, 0.0)).tagged(SegTag(0)),
            Seg2::line((width * 0.5, rise)).tagged(SegTag(1)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(2)),
        ])
        .expect("valid crossing gable"),
    )
    .expect("crossing gable winds counter-clockwise")
}

pub(super) fn east_chancel_profile(roof: &RoofSection) -> Profile2 {
    let width = roof.span.as_meters();
    let wall_plate_top = roof.wall_plate_top.as_meters();
    let outer = Loop2::new(vec![
        Seg2::line((width, 0.0)).tagged(SegTag(0)),
        Seg2::line((width, wall_plate_top)).tagged(SegTag(1)),
        Seg2::line((width * 0.5, roof.ridge_height.as_meters())).tagged(SegTag(2)),
        Seg2::line((0.0, wall_plate_top)).tagged(SegTag(3)),
        Seg2::line((0.0, 0.0)).tagged(SegTag(4)),
    ])
    .expect("valid chancel gable");
    let opening = round_head_hole(width * 0.5, 5.2, 0.18, 5.8);
    Profile2::new(outer, vec![opening]).expect("apse opening lies inside chancel gable")
}

pub(super) fn aisle_end_profile(width: f64, start_top: f64, end_top: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((width, 0.0)).tagged(SegTag(0)),
            Seg2::line((width, end_top)).tagged(SegTag(1)),
            Seg2::line((0.0, start_top)).tagged(SegTag(2)),
            Seg2::line((0.0, 0.0)).tagged(SegTag(3)),
        ])
        .expect("valid sloped aisle end"),
    )
    .expect("aisle end winds counter-clockwise")
}

pub(super) fn apse_shell_profile(
    outer_radius: f64,
    inner_radius: f64,
    arc_segments: u32,
) -> Profile2 {
    assert!(
        outer_radius > inner_radius && inner_radius > 0.0,
        "apse shell radii must be positive and nested"
    );
    assert!(
        arc_segments >= 2,
        "apse shell needs at least two arc segments"
    );

    let mut segments = Vec::with_capacity(arc_segments as usize * 2 + 2);
    segments.push(Seg2::line((0.0, -outer_radius)).tagged(SegTag(0)));
    for index in 1..=arc_segments {
        let angle = -core::f64::consts::FRAC_PI_2
            + core::f64::consts::PI * f64::from(index) / f64::from(arc_segments);
        segments.push(
            Seg2::line((
                outer_radius * libm::cos(angle),
                outer_radius * libm::sin(angle),
            ))
            .tagged(SegTag(index)),
        );
    }
    segments.push(Seg2::line((0.0, inner_radius)).tagged(SegTag(arc_segments.saturating_add(1))));
    for index in 1..=arc_segments {
        let angle = core::f64::consts::FRAC_PI_2
            - core::f64::consts::PI * f64::from(index) / f64::from(arc_segments);
        segments.push(
            Seg2::line((
                inner_radius * libm::cos(angle),
                inner_radius * libm::sin(angle),
            ))
            .tagged(SegTag(arc_segments.saturating_add(1).saturating_add(index))),
        );
    }

    Profile2::simple(Loop2::new(segments).expect("valid open semicircular apse shell"))
        .expect("counter-clockwise apse shell profile")
}

pub(super) fn vertical_wall_frame() -> Placement3 {
    Placement3::euler_xyz_then_translate(HALF_PI, 0.0, 0.0, [0.0, 0.0, 0.0])
}

pub(super) fn transverse_wall_frame() -> Placement3 {
    Placement3::from_axes([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0; 3])
}

pub(super) fn centered_vertical_wall_frame(width: f64, thickness: f64) -> Placement3 {
    Placement3::from_axes(
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [-width * 0.5, thickness * 0.5, 0.0],
    )
}

pub(super) fn roof_panel_frame(start_x: f64, roof: &RoofSection, north: bool) -> Placement3 {
    let x_axis = [1.0, 0.0, 0.0];
    if north {
        let mut ridge = lower_point(roof.ridge_point);
        ridge[0] = start_x;
        // The north skin is parameterized ridge-to-eave. Negating the exact
        // inward structural slope preserves the same pitch through the
        // overhang without re-running trigonometry in the geometry layer.
        let ridge_to_eave = scale(roof.inward_slope(RoofSide::North), -1.0);
        Placement3::from_axes(
            x_axis,
            ridge_to_eave,
            roof.outward_normal(RoofSide::North),
            ridge,
        )
    } else {
        let mut eave = lower_point(roof.roof_eave(RoofSide::South));
        eave[0] = start_x;
        // The south skin runs eave-to-ridge so X×Y remains the outward roof
        // normal. This keeps both frames right-handed while they mirror.
        Placement3::from_axes(
            x_axis,
            roof.inward_slope(RoofSide::South),
            roof.outward_normal(RoofSide::South),
            eave,
        )
    }
}

pub(super) fn aisle_roof_frame(
    plan: &PlanSection,
    aisle: &AisleSection,
    north: bool,
) -> Placement3 {
    let half_nave = plan.half_nave.as_meters();
    let run = aisle.roof_run.as_meters();
    let drop = aisle.roof_drop.as_meters();
    let inner_height = aisle.inner_height.as_meters();
    let angle = libm::atan2(drop, run);
    let (sin, cos) = (libm::sin(angle), libm::cos(angle));
    if north {
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, cos, -sin],
            [0.0, sin, cos],
            [0.0, half_nave, inner_height],
        )
    } else {
        // Parameterize the south skin from its eave back to its inner
        // bearing. Running both skins inner-to-eave would force the south
        // frame's right-handed thickness axis inward and down, placing that
        // roof one skin depth below its north-side mirror.
        Placement3::from_axes(
            [1.0, 0.0, 0.0],
            [0.0, cos, sin],
            [0.0, -sin, cos],
            [0.0, -half_nave - run, inner_height - drop],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BasilicaPremises, BasilicaSetout};

    #[test]
    fn nave_roof_frames_preserve_the_baseline_and_point_thickness_outward() {
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default roof resolves");
        let roof = setout.roof();
        let slope = roof.roof_slope_length.as_meters();
        let thickness = roof.roof_skin_depth.as_meters();
        let start_x = 10.75;
        let segment_length = 7.15;
        let north = roof_panel_frame(start_x, roof, true);
        let south = roof_panel_frame(start_x, roof, false);

        assert_close(rotation_determinant(&north), 1.0);
        assert_close(rotation_determinant(&south), 1.0);

        let north_ridge = transform_point(&north, [0.0, 0.0, 0.0]);
        let north_eave = transform_point(&north, [0.0, slope, 0.0]);
        let south_eave = transform_point(&south, [0.0, 0.0, 0.0]);
        let south_ridge = transform_point(&south, [0.0, slope, 0.0]);
        let with_x = |point: setout::Point3| {
            let mut point = lower_point(point);
            point[0] = start_x;
            point
        };
        assert_point_close(north_ridge, with_x(roof.ridge_point));
        assert_point_close(north_eave, with_x(roof.north_roof_eave));
        assert_point_close(south_eave, with_x(roof.south_roof_eave));
        assert_point_close(south_ridge, with_x(roof.ridge_point));

        for (frame, baseline, normal) in [
            (&north, north_ridge, roof.outward_normal(RoofSide::North)),
            (&south, south_eave, roof.outward_normal(RoofSide::South)),
        ] {
            // Project the displaced face back onto the resolved outward
            // normal: roof thickness must not intrude into the nave volume.
            let outer = transform_point(frame, [0.0, 0.0, thickness]);
            let signed_thickness = normal[0] * (outer[0] - baseline[0])
                + normal[1] * (outer[1] - baseline[1])
                + normal[2] * (outer[2] - baseline[2]);
            assert_close(signed_thickness, thickness);
        }

        for frame in [&north, &south] {
            let x0 = transform_point(frame, [0.0, 0.0, 0.0])[0];
            let x1 = transform_point(frame, [segment_length, 0.0, 0.0])[0];
            assert_close(x0, start_x);
            assert_close(x1, start_x + segment_length);
        }
    }

    #[test]
    fn nave_roof_bears_on_the_wall_plate_at_the_wall_line() {
        // This is the seam the old model only approximated: its roof slope was
        // drawn from the outer overhang to the ridge, so it passed above the
        // wall line without being derived from the wall plate. The roof may
        // overhang beyond this point, but its underside must pass exactly
        // through the plate top where the load enters the clerestory wall.
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default roof resolves");
        let roof = setout.roof();
        let frame = roof_panel_frame(0.0, roof, true);
        // The structural rafter length is exactly the ridge-to-seat distance,
        // so this samples the skin at the actual load-transfer line.
        let wall_line = transform_point(&frame, [0.0, roof.rafter_length.as_meters(), 0.0]);

        assert_point_close(wall_line, lower_point(roof.north_wall_seat));
    }

    #[test]
    fn aisle_roof_frames_are_proper_mirrors_with_outward_thickness() {
        // The two lean-to skins use opposite parameter directions so both
        // transforms remain right-handed and both thickness axes point out
        // of the building. This pins the old south-side defect where the
        // baseline was mirrored but the entire skin thickened inward/down.
        let p = BasilicaPremises::default();
        let setout = BasilicaSetout::new(&p).expect("default aisle roof resolves");
        let plan = setout.plan();
        let aisle = setout.aisle();
        let half_nave = plan.half_nave.as_meters();
        let run = aisle.roof_run.as_meters();
        let drop = aisle.roof_drop.as_meters();
        let inner_height = aisle.inner_height.as_meters();
        let slope = aisle.slope_length.as_meters();
        let thickness = aisle.roof_depth.as_meters();
        let north = aisle_roof_frame(plan, aisle, true);
        let south = aisle_roof_frame(plan, aisle, false);

        assert_close(rotation_determinant(&north), 1.0);
        assert_close(rotation_determinant(&south), 1.0);

        let north_inner = transform_point(&north, [0.0, 0.0, 0.0]);
        let north_eave = transform_point(&north, [0.0, slope, 0.0]);
        let south_eave = transform_point(&south, [0.0, 0.0, 0.0]);
        let south_inner = transform_point(&south, [0.0, slope, 0.0]);
        assert_point_close(north_inner, [0.0, half_nave, inner_height]);
        assert_point_close(north_eave, [0.0, half_nave + run, inner_height - drop]);
        assert_point_close(south_eave, [0.0, -half_nave - run, inner_height - drop]);
        assert_point_close(south_inner, [0.0, -half_nave, inner_height]);

        for (frame, baseline, outward_y) in [(&north, north_inner, 1.0), (&south, south_eave, -1.0)]
        {
            let outer = transform_point(frame, [0.0, 0.0, thickness]);
            assert!(
                outward_y * (outer[1] - baseline[1]) > 0.0,
                "roof thickness must move away from the nave"
            );
            assert!(
                outer[2] > baseline[2],
                "roof thickness must move above its underside"
            );
        }
    }

    fn transform_point(placement: &Placement3, point: [f64; 3]) -> [f64; 3] {
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

    fn assert_point_close(actual: [f64; 3], expected: [f64; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert_close(actual, expected);
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1.0e-9, "{actual} != {expected}");
    }
}
