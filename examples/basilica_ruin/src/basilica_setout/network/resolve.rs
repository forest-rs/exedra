// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Strict projection from an evaluation into architecture-facing sections.

use setout::Evaluation;

use super::{
    AisleSection, BasilicaQuantities, CrossingSection, EastEndSection, LevelSection, PlanSection,
    RoofSection,
};

pub(super) fn resolve_plan_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<PlanSection, setout::AccessError> {
    Ok(PlanSection {
        length: evaluation.exact(&quantities.length)?,
        nave_width: evaluation.exact(&quantities.span)?,
        half_nave: evaluation.exact(&quantities.half_span)?,
        total_width: evaluation.exact(&quantities.total_width)?,
        half_total: evaluation.exact(&quantities.half_total)?,
        aisle_run: evaluation.exact(&quantities.aisle_run)?,
        wall_thickness: evaluation.exact(&quantities.wall_thickness)?,
        east_end: evaluation.exact(&quantities.east_end)?,
        crossing_center: evaluation.exact(&quantities.crossing_center)?,
        crossing_half_width: evaluation.exact(&quantities.crossing_half_width)?,
        crossing_span: evaluation.exact(&quantities.crossing_span)?,
        crossing_west: evaluation.exact(&quantities.crossing_west)?,
        crossing_east: evaluation.exact(&quantities.crossing_east)?,
        west_nave_length: evaluation.exact(&quantities.west_nave_length)?,
        east_nave_length: evaluation.exact(&quantities.east_nave_length)?,
        arcade_bays: evaluation.exact(&quantities.arcade_bays)?,
        nave_truss_bays: evaluation.exact(&quantities.nave_truss_bays)?,
        buttress_start: evaluation.exact(&quantities.buttress_start)?,
        buttress_end: evaluation.exact(&quantities.buttress_end)?,
        nave_truss_west_start: evaluation.exact(&quantities.nave_truss_west_start)?,
        nave_truss_west_end: evaluation.exact(&quantities.nave_truss_west_end)?,
        nave_truss_east: evaluation.exact(&quantities.nave_truss_east)?,
    })
}

pub(super) fn resolve_level_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<LevelSection, setout::AccessError> {
    Ok(LevelSection {
        ground: evaluation.exact(&quantities.origin)?,
        aisle_wall_top: evaluation.exact(&quantities.aisle_wall_top)?,
        clerestory_base: evaluation.exact(&quantities.clerestory_base)?,
        clerestory_sill: evaluation.exact(&quantities.clerestory_sill)?,
        clerestory_spring: evaluation.exact(&quantities.clerestory_spring)?,
        clerestory_height: evaluation.exact(&quantities.clerestory_height)?,
        clerestory_sill_height: evaluation.exact(&quantities.clerestory_sill_height)?,
        clerestory_spring_height: evaluation.exact(&quantities.clerestory_spring_height)?,
        crossing_spandrel_base: evaluation.exact(&quantities.crossing_spandrel_base)?,
        crossing_web_bottom: evaluation.exact(&quantities.crossing_web_bottom)?,
        crossing_web_middle: evaluation.exact(&quantities.crossing_web_middle)?,
        apse_wall_top: evaluation.exact(&quantities.apse_wall_top)?,
    })
}

pub(super) fn resolve_aisle_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<AisleSection, setout::AccessError> {
    Ok(AisleSection {
        bearing_run: evaluation.exact(&quantities.aisle_run)?,
        overhang: evaluation.exact(&quantities.aisle_roof_overhang)?,
        roof_run: evaluation.exact(&quantities.aisle_roof_run)?,
        inner_height: evaluation.exact(&quantities.aisle_roof_inner_height)?,
        bearing_height: evaluation.exact(&quantities.aisle_roof_bearing_height)?,
        bearing_drop: evaluation.exact(&quantities.aisle_roof_bearing_drop)?,
        pitch: evaluation.exact(&quantities.aisle_roof_pitch)?,
        roof_drop: evaluation.exact(&quantities.aisle_roof_drop)?,
        slope_length: evaluation.exact(&quantities.aisle_roof_slope_length)?,
        roof_depth: evaluation.exact(&quantities.aisle_roof_depth)?,
    })
}

pub(super) fn resolve_crossing_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<CrossingSection, setout::AccessError> {
    Ok(CrossingSection {
        drum_radius: evaluation.exact(&quantities.drum_radius)?,
        platform_inner_radius: evaluation.exact(&quantities.platform_inner_radius)?,
        cornice_radius: evaluation.exact(&quantities.cornice_radius)?,
        dome_radius: evaluation.exact(&quantities.dome_radius)?,
        drum_height: evaluation.exact(&quantities.drum_height)?,
        dome_height: evaluation.exact(&quantities.dome_height)?,
        platform_base: evaluation.exact(&quantities.platform_base)?,
        platform_height: evaluation.exact(&quantities.platform_height)?,
        drum_base: evaluation.exact(&quantities.drum_base)?,
        drum_top: evaluation.exact(&quantities.drum_top)?,
        dome_top: evaluation.exact(&quantities.dome_top)?,
    })
}

pub(super) fn resolve_east_end_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<EastEndSection, setout::AccessError> {
    Ok(EastEndSection {
        apse_radius: evaluation.exact(&quantities.apse_radius)?,
        apse_inner_radius: evaluation.exact(&quantities.apse_inner_radius)?,
        apse_wall_height: evaluation.exact(&quantities.apse_wall_height)?,
        conch_radius: evaluation.exact(&quantities.conch_radius)?,
        conch_height: evaluation.exact(&quantities.conch_height)?,
    })
}

pub(super) fn resolve_roof_section(
    evaluation: &Evaluation,
    quantities: &BasilicaQuantities,
) -> Result<RoofSection, setout::AccessError> {
    Ok(RoofSection {
        span: evaluation.exact(&quantities.span)?,
        half_span: evaluation.exact(&quantities.half_span)?,
        wall_head: evaluation.exact(&quantities.wall_head)?,
        wall_plate_height: evaluation.exact(&quantities.wall_plate_height)?,
        wall_plate_top: evaluation.exact(&quantities.wall_plate_top)?,
        rise: evaluation.exact(&quantities.rise)?,
        ridge_height: evaluation.exact(&quantities.ridge_height)?,
        pitch: evaluation.exact(&quantities.pitch)?,
        rafter_length: evaluation.exact(&quantities.rafter_length)?,
        overhang: evaluation.exact(&quantities.overhang)?,
        overhang_drop: evaluation.exact(&quantities.overhang_drop)?,
        roof_run: evaluation.exact(&quantities.roof_run)?,
        roof_eave_height: evaluation.exact(&quantities.roof_eave_height)?,
        roof_slope_length: evaluation.exact(&quantities.roof_slope_length)?,
        roof_skin_depth: evaluation.exact(&quantities.roof_skin_depth)?,
        wall_plate_width: evaluation.exact(&quantities.wall_plate_width)?,
        principal_rafter_width: evaluation.exact(&quantities.principal_rafter_width)?,
        principal_rafter_depth: evaluation.exact(&quantities.principal_rafter_depth)?,
        principal_rafter_reveal: evaluation.exact(&quantities.principal_rafter_reveal)?,
        north_wall_seat: evaluation.exact(&quantities.north_wall_seat)?,
        south_wall_seat: evaluation.exact(&quantities.south_wall_seat)?,
        ridge_point: evaluation.exact(&quantities.ridge_point)?,
        north_roof_eave: evaluation.exact(&quantities.north_roof_eave)?,
        south_roof_eave: evaluation.exact(&quantities.south_roof_eave)?,
    })
}
