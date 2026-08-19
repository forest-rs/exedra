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

pub(super) fn apse_roof_recipe(radius: f64, height: f64) -> Recipe {
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref("basilica:east-apse-roof");
    let surface = builder.material_slot("surface");
    let rings = [
        (radius, 0.0),
        (radius * 0.98, height * 0.18),
        (radius * 0.82, height * 0.48),
        (radius * 0.52, height * 0.78),
        (radius * 0.15, height),
    ];
    let sections = rings
        .into_iter()
        .map(|(ring_radius, z)| {
            let profile = builder.add_profile(half_polygon_profile(ring_radius, 16));
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
        .expect("valid apsidal half-dome loft");
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

pub(super) fn half_polygon_profile(radius: f64, arc_segments: u32) -> Profile2 {
    let mut segments = Vec::with_capacity(arc_segments as usize + 1);
    segments.push(Seg2::line((0.0, -radius)).tagged(SegTag(0)));
    for index in 1..=arc_segments {
        let angle = -core::f64::consts::FRAC_PI_2
            + core::f64::consts::PI * f64::from(index) / f64::from(arc_segments);
        segments.push(
            Seg2::line((radius * libm::cos(angle), radius * libm::sin(angle)))
                .tagged(SegTag(index)),
        );
    }
    Profile2::simple(Loop2::new(segments).expect("half polygon has distinct points"))
        .expect("half polygon winds counter-clockwise")
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

pub(super) fn apse_profile(radius: f64) -> Profile2 {
    Profile2::simple(
        Loop2::new(vec![
            Seg2::line((0.0, -radius)).tagged(SegTag(0)),
            Seg2::arc((0.0, radius), 1.0).tagged(SegTag(1)),
        ])
        .expect("valid semicircular apse"),
    )
    .expect("counter-clockwise apse profile")
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
        Placement3 {
            rows: [
                [1.0, 0.0, 0.0, start_x],
                [0.0, -cos, sin, 0.0],
                [0.0, -sin, -cos, peak],
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
