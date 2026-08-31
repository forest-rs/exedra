// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact authored roots for the basilica network.

use setout::{
    ArithmeticError, Knowledge, Length, NetworkDef, Offset, RootClaimSet, RootClaimSetBuilder,
};

use super::{BasilicaPremises, BasilicaQuantities, BasilicaSetoutError};

const WALL_PLATE_HEIGHT_MM: u64 = 180;
const WALL_PLATE_WIDTH_MM: u64 = 300;
const ROOF_OVERHANG_MM: u64 = 350;
const ROOF_SKIN_DEPTH_MM: u64 = 280;
const PRINCIPAL_RAFTER_WIDTH_MM: u64 = 260;
const PRINCIPAL_RAFTER_DEPTH_MM: u64 = 240;
const PRINCIPAL_RAFTER_REVEAL_MM: i64 = 120;
const BUTTRESS_WEST_INSET_MM: u64 = 1_700;
const BUTTRESS_EAST_INSET_MM: u64 = 2_300;

pub(super) fn build_roots(
    definition: &NetworkDef,
    quantities: &BasilicaQuantities,
    premises: &BasilicaPremises,
) -> Result<RootClaimSet, BasilicaSetoutError> {
    let mut roots = RootClaimSetBuilder::new(definition);
    roots
        .author(
            "basilica/root/roof-zero",
            &quantities.origin,
            Knowledge::exact(Offset::ZERO),
        )?
        .author(
            "basilica/root/length",
            &quantities.length,
            Knowledge::exact(premises.length),
        )?
        .author(
            "basilica/root/nave-span",
            &quantities.span,
            Knowledge::exact(premises.nave_width),
        )?
        .author(
            "basilica/root/total-width",
            &quantities.total_width,
            Knowledge::exact(premises.total_width),
        )?
        .author(
            "basilica/root/crossing-station",
            &quantities.crossing_station,
            Knowledge::exact(premises.crossing_station),
        )?
        .author(
            "basilica/root/arcade-bays",
            &quantities.arcade_bays,
            Knowledge::exact(premises.arcade_bays),
        )?
        .author(
            "basilica/root/buttress-west-inset",
            &quantities.buttress_west_inset,
            Knowledge::exact(exact_length_millimeters(BUTTRESS_WEST_INSET_MM)?),
        )?
        .author(
            "basilica/root/buttress-east-inset",
            &quantities.buttress_east_inset,
            Knowledge::exact(exact_length_millimeters(BUTTRESS_EAST_INSET_MM)?),
        )?
        .author(
            "basilica/root/nave-wall-height",
            &quantities.nave_wall_height,
            Knowledge::exact(premises.nave_wall_height),
        )?
        .author(
            "basilica/root/aisle-wall-height",
            &quantities.aisle_wall_height,
            Knowledge::exact(premises.aisle_wall_height),
        )?
        .author(
            "basilica/root/roof-rise",
            &quantities.rise,
            Knowledge::exact(premises.roof_rise),
        )?
        .author(
            "basilica/root/drum-radius",
            &quantities.drum_radius,
            Knowledge::exact(premises.drum_radius),
        )?
        .author(
            "basilica/root/drum-height",
            &quantities.drum_height,
            Knowledge::exact(premises.drum_height),
        )?
        .author(
            "basilica/root/dome-height",
            &quantities.dome_height,
            Knowledge::exact(premises.dome_height),
        )?
        .author(
            "basilica/root/apse-radius",
            &quantities.apse_radius,
            Knowledge::exact(premises.apse_radius),
        )?
        .author(
            "basilica/root/wall-thickness",
            &quantities.wall_thickness,
            Knowledge::exact(exact_length_millimeters(450)?),
        )?
        .author(
            "basilica/root/crossing-clearance",
            &quantities.crossing_clearance,
            Knowledge::exact(exact_length_millimeters(600)?),
        )?
        .author(
            "basilica/root/clerestory-base",
            &quantities.clerestory_base_height,
            Knowledge::exact(exact_length_millimeters(5_750)?),
        )?
        .author(
            "basilica/root/clerestory-sill",
            &quantities.clerestory_sill_height_from_ground,
            Knowledge::exact(exact_length_millimeters(6_550)?),
        )?
        .author(
            "basilica/root/clerestory-spring",
            &quantities.clerestory_spring_height_from_ground,
            Knowledge::exact(exact_length_millimeters(7_900)?),
        )?
        .author(
            "basilica/root/crossing-spandrel-base",
            &quantities.crossing_spandrel_base_height,
            Knowledge::exact(exact_length_millimeters(9_600)?),
        )?
        .author(
            "basilica/root/crossing-web-bottom",
            &quantities.crossing_web_bottom_height,
            Knowledge::exact(exact_length_millimeters(11_400)?),
        )?
        .author(
            "basilica/root/crossing-web-middle",
            &quantities.crossing_web_middle_height,
            Knowledge::exact(exact_length_millimeters(13_000)?),
        )?
        .author(
            "basilica/root/apse-wall-height",
            &quantities.apse_wall_height,
            Knowledge::exact(exact_length_millimeters(8_000)?),
        )?
        .author(
            "basilica/root/aisle-roof-overhang",
            &quantities.aisle_roof_overhang,
            Knowledge::exact(exact_length_millimeters(300)?),
        )?
        .author(
            "basilica/root/aisle-roof-inner-lift",
            &quantities.aisle_roof_inner_lift,
            Knowledge::exact(exact_length_millimeters(520)?),
        )?
        .author(
            "basilica/root/aisle-roof-bearing-inset",
            &quantities.aisle_roof_bearing_inset,
            Knowledge::exact(exact_length_millimeters(80)?),
        )?
        .author(
            "basilica/root/aisle-roof-depth",
            &quantities.aisle_roof_depth,
            Knowledge::exact(exact_length_millimeters(220)?),
        )?
        .author(
            "basilica/root/wall-plate-height",
            &quantities.wall_plate_height,
            Knowledge::exact(exact_length_millimeters(WALL_PLATE_HEIGHT_MM)?),
        )?
        .author(
            "basilica/root/wall-plate-width",
            &quantities.wall_plate_width,
            Knowledge::exact(exact_length_millimeters(WALL_PLATE_WIDTH_MM)?),
        )?
        .author(
            "basilica/root/roof-overhang",
            &quantities.overhang,
            Knowledge::exact(exact_length_millimeters(ROOF_OVERHANG_MM)?),
        )?
        .author(
            "basilica/root/roof-skin-depth",
            &quantities.roof_skin_depth,
            Knowledge::exact(exact_length_millimeters(ROOF_SKIN_DEPTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-width",
            &quantities.principal_rafter_width,
            Knowledge::exact(exact_length_millimeters(PRINCIPAL_RAFTER_WIDTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-depth",
            &quantities.principal_rafter_depth,
            Knowledge::exact(exact_length_millimeters(PRINCIPAL_RAFTER_DEPTH_MM)?),
        )?
        .author(
            "basilica/root/principal-rafter-reveal",
            &quantities.principal_rafter_reveal,
            Knowledge::exact(exact_offset_millimeters(PRINCIPAL_RAFTER_REVEAL_MM)?),
        )?
        .author(
            "basilica/root/crossing-platform-inset",
            &quantities.crossing_platform_inset,
            Knowledge::exact(exact_length_millimeters(480)?),
        )?
        .author(
            "basilica/root/cornice-overhang",
            &quantities.cornice_overhang,
            Knowledge::exact(exact_length_millimeters(200)?),
        )?
        .author(
            "basilica/root/dome-overhang",
            &quantities.dome_overhang,
            Knowledge::exact(exact_length_millimeters(180)?),
        )?
        .author(
            "basilica/root/drum-base-above-ridge",
            &quantities.drum_base_above_ridge,
            Knowledge::exact(exact_length_millimeters(550)?),
        )?
        .author(
            "basilica/root/crossing-platform-height",
            &quantities.platform_height,
            Knowledge::exact(exact_length_millimeters(220)?),
        )?
        .author(
            "basilica/root/conch-overhang",
            &quantities.conch_overhang,
            Knowledge::exact(exact_length_millimeters(180)?),
        )?
        .author(
            "basilica/root/conch-height",
            &quantities.conch_height,
            Knowledge::exact(exact_length_millimeters(2_350)?),
        )?;
    Ok(roots.finish()?)
}

fn exact_length_millimeters(value: u64) -> Result<Length, ArithmeticError> {
    Length::millimeters(value).ok_or(ArithmeticError::Overflow)
}

fn exact_offset_millimeters(value: i64) -> Result<Offset, ArithmeticError> {
    Offset::millimeters(value).ok_or(ArithmeticError::Overflow)
}
