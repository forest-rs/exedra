// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, LoftPolicy, NodeKind, Placement3, Recipe, RecipeBuilder};
use exedra_constructive::profile::{Loop2, Profile2, Seg2, SegTag};

use crate::BasilicaParams;

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

pub(super) fn west_facade_profile(p: &BasilicaParams) -> Profile2 {
    let half_nave = p.nave_width * 0.5;
    let half_total = p.total_width * 0.5;
    let peak = p.nave_wall_height + p.roof_rise;
    let outer = Loop2::new(vec![
        Seg2::line((half_total, 0.0)).tagged(SegTag(0)),
        Seg2::line((half_total, p.aisle_wall_height)).tagged(SegTag(1)),
        Seg2::line((half_nave, p.aisle_wall_height)).tagged(SegTag(2)),
        Seg2::line((half_nave, p.nave_wall_height)).tagged(SegTag(3)),
        Seg2::line((0.0, peak)).tagged(SegTag(4)),
        Seg2::line((-half_nave, p.nave_wall_height)).tagged(SegTag(5)),
        Seg2::line((-half_nave, p.aisle_wall_height)).tagged(SegTag(6)),
        Seg2::line((-half_total, p.aisle_wall_height)).tagged(SegTag(7)),
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

pub(super) fn east_chancel_profile(p: &BasilicaParams) -> Profile2 {
    let width = p.nave_width;
    let outer = Loop2::new(vec![
        Seg2::line((width, 0.0)).tagged(SegTag(0)),
        Seg2::line((width, p.nave_wall_height)).tagged(SegTag(1)),
        Seg2::line((width * 0.5, p.nave_wall_height + p.roof_rise)).tagged(SegTag(2)),
        Seg2::line((0.0, p.nave_wall_height)).tagged(SegTag(3)),
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
    Placement3 {
        rows: [
            [0.0, 0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
        ],
    }
}

pub(super) fn centered_vertical_wall_frame(width: f64, thickness: f64) -> Placement3 {
    Placement3 {
        rows: [
            [1.0, 0.0, 0.0, -width * 0.5],
            [0.0, 0.0, -1.0, thickness * 0.5],
            [0.0, 1.0, 0.0, 0.0],
        ],
    }
}

pub(super) fn roof_panel_frame(
    start_x: f64,
    run: f64,
    p: &BasilicaParams,
    north: bool,
) -> Placement3 {
    let angle = libm::atan2(p.roof_rise, run);
    let (sin, cos) = (libm::sin(angle), libm::cos(angle));
    let peak = p.nave_wall_height + p.roof_rise;
    if north {
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, start_x],
                [0.0, cos, sin, 0.0],
                [0.0, -sin, cos, peak],
            ],
        }
    } else {
        // Parameterize the south slope from eave to ridge so local +Z points
        // outward/up, mirroring the north slab instead of thickening inward
        // into the nave.
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, start_x],
                [0.0, cos, -sin, -run],
                [0.0, sin, cos, p.nave_wall_height],
            ],
        }
    }
}

pub(super) fn aisle_roof_section(p: &BasilicaParams) -> (f64, f64, f64) {
    let half_nave = p.nave_width * 0.5;
    let half_total = p.total_width * 0.5;
    let bearing_run = half_total - half_nave;
    // Keep the complete roof build-up below the 6.0 clerestory sill, while
    // letting its underside overlap the outer wall head by a small bearing.
    let inner_height = p.aisle_wall_height + 0.52;
    let bearing_height = p.aisle_wall_height - 0.08;
    let total_run = bearing_run + 0.3;
    let total_drop = (inner_height - bearing_height) * total_run / bearing_run;
    (total_run, total_drop, inner_height)
}

pub(super) fn aisle_roof_frame(
    p: &BasilicaParams,
    run: f64,
    drop: f64,
    inner_height: f64,
    north: bool,
) -> Placement3 {
    let half_nave = p.nave_width * 0.5;
    let angle = libm::atan2(drop, run);
    let (sin, cos) = (libm::sin(angle), libm::cos(angle));
    if north {
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, cos, sin, half_nave],
                [0.0, -sin, cos, inner_height],
            ],
        }
    } else {
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, -cos, sin, -half_nave],
                [0.0, -sin, -cos, inner_height],
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nave_roof_frames_preserve_the_baseline_and_point_thickness_outward() {
        const THICKNESS: f64 = 0.28;

        let p = BasilicaParams::default();
        let run = p.nave_width * 0.5 + 0.35;
        let slope = libm::sqrt(run * run + p.roof_rise * p.roof_rise);
        let roof_sin = p.roof_rise / slope;
        let roof_cos = run / slope;
        let peak = p.nave_wall_height + p.roof_rise;
        let start_x = 10.75;
        let segment_length = 7.15;
        let north = roof_panel_frame(start_x, run, &p, true);
        let south = roof_panel_frame(start_x, run, &p, false);

        assert_close(rotation_determinant(&north), 1.0);
        assert_close(rotation_determinant(&south), 1.0);

        let north_ridge = transform_point(&north, [0.0, 0.0, 0.0]);
        let north_eave = transform_point(&north, [0.0, slope, 0.0]);
        let south_eave = transform_point(&south, [0.0, 0.0, 0.0]);
        let south_ridge = transform_point(&south, [0.0, slope, 0.0]);
        assert_point_close(north_ridge, [start_x, 0.0, peak]);
        assert_point_close(north_eave, [start_x, run, p.nave_wall_height]);
        assert_point_close(south_eave, [start_x, -run, p.nave_wall_height]);
        assert_point_close(south_ridge, [start_x, 0.0, peak]);

        for (frame, baseline, normal_y) in [
            (&north, north_ridge, roof_sin),
            (&south, south_eave, -roof_sin),
        ] {
            let outer = transform_point(frame, [0.0, 0.0, THICKNESS]);
            let signed_thickness =
                normal_y * (outer[1] - baseline[1]) + roof_cos * (outer[2] - baseline[2]);
            assert_close(signed_thickness, THICKNESS);
        }

        for frame in [&north, &south] {
            let x0 = transform_point(frame, [0.0, 0.0, 0.0])[0];
            let x1 = transform_point(frame, [segment_length, 0.0, 0.0])[0];
            assert_close(x0, start_x);
            assert_close(x1, start_x + segment_length);
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
