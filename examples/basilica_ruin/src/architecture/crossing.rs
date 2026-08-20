// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_constructive::ir::Placement3;

use super::{
    BuildContext, CROSSING_PLATFORM_HEIGHT, Layout, crossing_drum_base, crossing_platform_base,
};
use crate::BasilicaParams;
use crate::geometry::{
    arcaded_wall_profile, box_recipe, centered_vertical_wall_frame, cylinder_recipe, dome_recipe,
    drum_panel_profile, extruded_profile_recipe, square_polygon_ring_profile,
    transverse_wall_frame, vertical_wall_frame,
};
use crate::names;

pub(super) fn build(context: &mut BuildContext, p: &BasilicaParams, layout: Layout) {
    let Layout {
        crossing_west,
        crossing_east,
        ..
    } = layout;

    // The crossing stage is a visible load path: four ground-bearing piers
    // carry upper spandrel beams and an open square bearing ring, which in
    // turn bears the polygonal drum above the nave roof ridge. The separate
    // crossing-transition system fills the four corners below that ring.
    let crossing_span = crossing_east - crossing_west;
    let half_crossing = crossing_span * 0.5;
    let drum_base = crossing_drum_base(p);
    let platform_base = crossing_platform_base(p);
    let pier_size = 1.15;
    let crossing_pier = context.add_part(
        "crossing-pier",
        box_recipe(
            [pier_size, pier_size, platform_base],
            "basilica:crossing-pier",
        ),
        "warm-stone",
    );
    for (key, x, y) in [
        ("crossing-pier-south-west", crossing_west, -half_crossing),
        (
            "crossing-pier-north-west",
            crossing_west,
            half_crossing - pier_size,
        ),
        (
            "crossing-pier-south-east",
            crossing_east - pier_size,
            -half_crossing,
        ),
        (
            "crossing-pier-north-east",
            crossing_east - pier_size,
            half_crossing - pier_size,
        ),
    ] {
        context.add_instance(
            key,
            crossing_pier,
            Placement3::translate(x, y, 0.0),
            names::roles::CROSSING_PIER,
        );
    }
    let spandrel_base = 9.6;
    let spandrel_height = platform_base - spandrel_base;
    let crossing_spandrel_long = context.add_part(
        "crossing-spandrel-long",
        extruded_profile_recipe(
            arcaded_wall_profile(crossing_span, spandrel_height, 1, 5.4, 0.12, 1.88, None),
            0.65,
            vertical_wall_frame(),
            "basilica:crossing-spandrel-long",
        ),
        "warm-stone",
    );
    context.add_instance(
        "crossing-spandrel-south",
        crossing_spandrel_long,
        Placement3::translate(crossing_west, -half_crossing + 0.65, spandrel_base),
        "crossing_spandrel",
    );
    context.add_instance(
        "crossing-spandrel-north",
        crossing_spandrel_long,
        Placement3::translate(crossing_west, half_crossing, spandrel_base),
        "crossing_spandrel",
    );
    let crossing_spandrel_short = context.add_part(
        "crossing-spandrel-short",
        extruded_profile_recipe(
            arcaded_wall_profile(
                crossing_span - 1.3,
                spandrel_height,
                1,
                4.4,
                0.12,
                1.78,
                None,
            ),
            0.65,
            transverse_wall_frame(),
            "basilica:crossing-spandrel-short",
        ),
        "warm-stone",
    );
    context.add_instance(
        "crossing-spandrel-west",
        crossing_spandrel_short,
        Placement3::translate(crossing_west, -half_crossing + 0.65, spandrel_base),
        "crossing_spandrel",
    );
    context.add_instance(
        "crossing-spandrel-east",
        crossing_spandrel_short,
        Placement3::translate(crossing_east - 0.65, -half_crossing + 0.65, spandrel_base),
        "crossing_spandrel",
    );
    let crossing_platform = context.add_part(
        "crossing-platform",
        extruded_profile_recipe(
            square_polygon_ring_profile(half_crossing, p.drum_radius - 0.48, 12),
            CROSSING_PLATFORM_HEIGHT,
            Placement3::IDENTITY,
            "basilica:crossing-platform",
        ),
        "warm-stone",
    );
    context.add_instance(
        names::instances::CROSSING_PLATFORM,
        crossing_platform,
        Placement3::translate(p.crossing_x, 0.0, platform_base),
        "crossing_platform",
    );

    let drum_faces = 12_u32;
    let half_angle = core::f64::consts::PI / f64::from(drum_faces);
    let drum_chord = 2.0 * p.drum_radius * libm::sin(half_angle);
    let drum_apothem = p.drum_radius * libm::cos(half_angle);
    let drum_thickness = 0.36;
    let drum_window = context.add_part(
        names::parts::DRUM_WINDOW_PANEL,
        extruded_profile_recipe(
            drum_panel_profile(drum_chord, p.drum_height, true),
            drum_thickness,
            centered_vertical_wall_frame(drum_chord, drum_thickness),
            "basilica:crossing-drum-window-panel",
        ),
        "warm-stone",
    );
    let drum_solid = context.add_part(
        "crossing-drum-solid-panel",
        extruded_profile_recipe(
            drum_panel_profile(drum_chord, p.drum_height, false),
            drum_thickness,
            centered_vertical_wall_frame(drum_chord, drum_thickness),
            "basilica:crossing-drum-solid-panel",
        ),
        "warm-stone",
    );
    for face in 0..drum_faces {
        let angle = core::f64::consts::TAU * f64::from(face) / f64::from(drum_faces);
        let part = if face % 2 == 0 {
            drum_window
        } else {
            drum_solid
        };
        context.add_instance(
            &format!("crossing-drum-panel-{face:02}"),
            part,
            Placement3::rotate_z_then_translate(
                angle + core::f64::consts::FRAC_PI_2,
                p.crossing_x + drum_apothem * libm::cos(angle),
                drum_apothem * libm::sin(angle),
                drum_base,
            ),
            if face % 2 == 0 {
                names::roles::DRUM_WINDOW
            } else {
                "drum_solid"
            },
        );
    }
    let drum_cornice = context.add_part(
        "crossing-drum-cornice",
        cylinder_recipe(
            p.drum_radius + 0.2,
            0.22,
            drum_faces,
            "basilica:crossing-drum-cornice",
        ),
        "warm-stone",
    );
    context.add_instance(
        "crossing-drum-cornice-base",
        drum_cornice,
        Placement3::translate(p.crossing_x, 0.0, drum_base - 0.1),
        "drum_cornice",
    );
    context.add_instance(
        "crossing-drum-cornice-top",
        drum_cornice,
        Placement3::translate(p.crossing_x, 0.0, drum_base + p.drum_height - 0.1),
        "drum_cornice",
    );

    let dome = context.add_part(
        names::parts::CROSSING_DOME,
        dome_recipe(p.drum_radius + 0.18, p.dome_height),
        "oxidized-lead",
    );
    context.add_instance(
        names::instances::CROSSING_DOME,
        dome,
        Placement3::translate(p.crossing_x, 0.0, drum_base + p.drum_height),
        "dome",
    );
}
