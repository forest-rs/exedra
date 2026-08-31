// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact quantity declarations and the ordered basilica relation graph.

use setout::{
    BuildError, ComposePoint, Count, Length, NetworkBuilder, NetworkDef, Offset, OffsetByLength,
    OffsetDirection, Pitch, Point3, Pythagorean, QuantityPolicy, Rational, ScaleLength, Sum,
};

use super::BasilicaQuantities;

pub(super) fn build_definition() -> Result<(NetworkDef, BasilicaQuantities), BuildError> {
    let mut network = NetworkBuilder::new();
    let origin =
        network.declare::<Offset>("basilica/roof/origin", QuantityPolicy::unrestricted())?;
    let length = network.declare::<Length>("basilica/plan/length", QuantityPolicy::positive())?;
    let east_end =
        network.declare::<Offset>("basilica/plan/east-end", QuantityPolicy::positive())?;
    let span = network.declare::<Length>("basilica/roof/span", QuantityPolicy::positive())?;
    let half_span =
        network.declare::<Length>("basilica/roof/half-span", QuantityPolicy::positive())?;
    let north_half_span = network.declare::<Offset>(
        "basilica/roof/north-half-span",
        QuantityPolicy::unrestricted(),
    )?;
    let south_half_span = network.declare::<Offset>(
        "basilica/roof/south-half-span",
        QuantityPolicy::unrestricted(),
    )?;
    let total_width =
        network.declare::<Length>("basilica/plan/total-width", QuantityPolicy::positive())?;
    let half_total =
        network.declare::<Length>("basilica/plan/half-total", QuantityPolicy::positive())?;
    let side_aisles_width = network.declare::<Length>(
        "basilica/plan/side-aisles-width",
        QuantityPolicy::positive(),
    )?;
    let aisle_run =
        network.declare::<Length>("basilica/plan/aisle-run", QuantityPolicy::positive())?;
    let wall_thickness =
        network.declare::<Length>("basilica/plan/wall-thickness", QuantityPolicy::positive())?;
    let north_total = network.declare::<Offset>(
        "basilica/plan/north-total-edge",
        QuantityPolicy::unrestricted(),
    )?;
    let south_total = network.declare::<Offset>(
        "basilica/plan/south-total-edge",
        QuantityPolicy::unrestricted(),
    )?;
    let crossing_station =
        network.declare::<Length>("basilica/plan/crossing-station", QuantityPolicy::positive())?;
    let crossing_center =
        network.declare::<Offset>("basilica/plan/crossing-center", QuantityPolicy::positive())?;
    let crossing_clearance = network.declare::<Length>(
        "basilica/plan/crossing-clearance",
        QuantityPolicy::positive(),
    )?;
    let crossing_half_width = network.declare::<Length>(
        "basilica/plan/crossing-half-width",
        QuantityPolicy::positive(),
    )?;
    let crossing_span =
        network.declare::<Length>("basilica/plan/crossing-span", QuantityPolicy::positive())?;
    let crossing_west =
        network.declare::<Offset>("basilica/plan/crossing-west", QuantityPolicy::positive())?;
    let crossing_east =
        network.declare::<Offset>("basilica/plan/crossing-east", QuantityPolicy::positive())?;
    let west_nave_length =
        network.declare::<Length>("basilica/plan/west-nave-length", QuantityPolicy::positive())?;
    let east_nave_length =
        network.declare::<Length>("basilica/plan/east-nave-length", QuantityPolicy::positive())?;
    let arcade_bays =
        network.declare::<Count>("basilica/plan/arcade-bays", QuantityPolicy::positive())?;
    let nave_truss_bays =
        network.declare::<Count>("basilica/plan/nave-truss-bays", QuantityPolicy::positive())?;
    let buttress_west_inset = network.declare::<Length>(
        "basilica/plan/buttress-west-inset",
        QuantityPolicy::positive(),
    )?;
    let buttress_east_inset = network.declare::<Length>(
        "basilica/plan/buttress-east-inset",
        QuantityPolicy::positive(),
    )?;
    let buttress_start =
        network.declare::<Offset>("basilica/plan/buttress-start", QuantityPolicy::positive())?;
    let buttress_end =
        network.declare::<Offset>("basilica/plan/buttress-end", QuantityPolicy::positive())?;
    let nave_truss_end_clearance = network.declare::<Length>(
        "basilica/plan/nave-truss-end-clearance",
        QuantityPolicy::positive(),
    )?;
    let nave_truss_west_start = network.declare::<Offset>(
        "basilica/plan/nave-truss-west-start",
        QuantityPolicy::positive(),
    )?;
    let nave_truss_west_end = network.declare::<Offset>(
        "basilica/plan/nave-truss-west-end",
        QuantityPolicy::positive(),
    )?;
    let nave_truss_east_half_length = network.declare::<Length>(
        "basilica/plan/nave-truss-east-half-length",
        QuantityPolicy::positive(),
    )?;
    let nave_truss_east =
        network.declare::<Offset>("basilica/plan/nave-truss-east", QuantityPolicy::positive())?;
    let nave_wall_height = network.declare::<Length>(
        "basilica/levels/nave-wall-height",
        QuantityPolicy::positive(),
    )?;
    let wall_head =
        network.declare::<Offset>("basilica/roof/wall-head", QuantityPolicy::positive())?;
    let aisle_wall_height = network.declare::<Length>(
        "basilica/levels/aisle-wall-height",
        QuantityPolicy::positive(),
    )?;
    let aisle_wall_top =
        network.declare::<Offset>("basilica/levels/aisle-wall-top", QuantityPolicy::positive())?;
    let clerestory_base_height = network.declare::<Length>(
        "basilica/levels/clerestory-base-height",
        QuantityPolicy::positive(),
    )?;
    let clerestory_base = network.declare::<Offset>(
        "basilica/levels/clerestory-base",
        QuantityPolicy::positive(),
    )?;
    let clerestory_sill_height_from_ground = network.declare::<Length>(
        "basilica/levels/clerestory-sill-height",
        QuantityPolicy::positive(),
    )?;
    let clerestory_sill = network.declare::<Offset>(
        "basilica/levels/clerestory-sill",
        QuantityPolicy::positive(),
    )?;
    let clerestory_spring_height_from_ground = network.declare::<Length>(
        "basilica/levels/clerestory-spring-height",
        QuantityPolicy::positive(),
    )?;
    let clerestory_spring = network.declare::<Offset>(
        "basilica/levels/clerestory-spring",
        QuantityPolicy::positive(),
    )?;
    let clerestory_height = network.declare::<Length>(
        "basilica/levels/clerestory-local-height",
        QuantityPolicy::positive(),
    )?;
    let clerestory_sill_height = network.declare::<Length>(
        "basilica/levels/clerestory-local-sill",
        QuantityPolicy::positive(),
    )?;
    let clerestory_spring_height = network.declare::<Length>(
        "basilica/levels/clerestory-local-spring",
        QuantityPolicy::positive(),
    )?;
    let crossing_spandrel_base_height = network.declare::<Length>(
        "basilica/levels/crossing-spandrel-base-height",
        QuantityPolicy::positive(),
    )?;
    let crossing_spandrel_base = network.declare::<Offset>(
        "basilica/levels/crossing-spandrel-base",
        QuantityPolicy::positive(),
    )?;
    let crossing_web_bottom_height = network.declare::<Length>(
        "basilica/levels/crossing-web-bottom-height",
        QuantityPolicy::positive(),
    )?;
    let crossing_web_bottom = network.declare::<Offset>(
        "basilica/levels/crossing-web-bottom",
        QuantityPolicy::positive(),
    )?;
    let crossing_web_middle_height = network.declare::<Length>(
        "basilica/levels/crossing-web-middle-height",
        QuantityPolicy::positive(),
    )?;
    let crossing_web_middle = network.declare::<Offset>(
        "basilica/levels/crossing-web-middle",
        QuantityPolicy::positive(),
    )?;
    let apse_wall_height = network.declare::<Length>(
        "basilica/east-end/apse-wall-height",
        QuantityPolicy::positive(),
    )?;
    let apse_wall_top = network.declare::<Offset>(
        "basilica/east-end/apse-wall-top",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_overhang =
        network.declare::<Length>("basilica/aisle/roof-overhang", QuantityPolicy::positive())?;
    let aisle_roof_run =
        network.declare::<Length>("basilica/aisle/roof-run", QuantityPolicy::positive())?;
    let aisle_roof_inner_lift =
        network.declare::<Length>("basilica/aisle/roof-inner-lift", QuantityPolicy::positive())?;
    let aisle_roof_inner_height = network.declare::<Offset>(
        "basilica/aisle/roof-inner-height",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_bearing_inset = network.declare::<Length>(
        "basilica/aisle/roof-bearing-inset",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_bearing_height = network.declare::<Offset>(
        "basilica/aisle/roof-bearing-height",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_bearing_drop = network.declare::<Length>(
        "basilica/aisle/roof-bearing-drop",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_pitch =
        network.declare::<Rational>("basilica/aisle/roof-pitch", QuantityPolicy::unrestricted())?;
    let aisle_roof_drop =
        network.declare::<Length>("basilica/aisle/roof-drop", QuantityPolicy::positive())?;
    let aisle_roof_slope_length = network.declare::<Length>(
        "basilica/aisle/roof-slope-length",
        QuantityPolicy::positive(),
    )?;
    let aisle_roof_depth =
        network.declare::<Length>("basilica/aisle/roof-depth", QuantityPolicy::positive())?;
    let wall_plate_height = network.declare::<Length>(
        "basilica/roof/wall-plate-height",
        QuantityPolicy::positive(),
    )?;
    let wall_plate_top =
        network.declare::<Offset>("basilica/roof/wall-plate-top", QuantityPolicy::positive())?;
    let rise = network.declare::<Length>("basilica/roof/rise", QuantityPolicy::positive())?;
    let ridge_height =
        network.declare::<Offset>("basilica/roof/ridge-height", QuantityPolicy::positive())?;
    let pitch =
        network.declare::<Rational>("basilica/roof/pitch", QuantityPolicy::unrestricted())?;
    let rafter_length = network.declare::<Length>(
        "basilica/roof/principal-rafter-length",
        QuantityPolicy::positive(),
    )?;
    let overhang =
        network.declare::<Length>("basilica/roof/overhang", QuantityPolicy::positive())?;
    let overhang_drop =
        network.declare::<Length>("basilica/roof/overhang-drop", QuantityPolicy::positive())?;
    let roof_run =
        network.declare::<Length>("basilica/roof/full-run", QuantityPolicy::positive())?;
    let north_roof_run = network.declare::<Offset>(
        "basilica/roof/north-full-run",
        QuantityPolicy::unrestricted(),
    )?;
    let south_roof_run = network.declare::<Offset>(
        "basilica/roof/south-full-run",
        QuantityPolicy::unrestricted(),
    )?;
    let roof_total_rise =
        network.declare::<Length>("basilica/roof/full-rise", QuantityPolicy::positive())?;
    let roof_eave_height =
        network.declare::<Offset>("basilica/roof/eave-height", QuantityPolicy::positive())?;
    let roof_slope_length = network.declare::<Length>(
        "basilica/roof/skin-slope-length",
        QuantityPolicy::positive(),
    )?;
    let roof_skin_depth =
        network.declare::<Length>("basilica/roof/skin-depth", QuantityPolicy::positive())?;
    let wall_plate_width =
        network.declare::<Length>("basilica/roof/wall-plate-width", QuantityPolicy::positive())?;
    let principal_rafter_width = network.declare::<Length>(
        "basilica/roof/principal-rafter-width",
        QuantityPolicy::positive(),
    )?;
    let principal_rafter_depth = network.declare::<Length>(
        "basilica/roof/principal-rafter-depth",
        QuantityPolicy::positive(),
    )?;
    let principal_rafter_reveal = network.declare::<Offset>(
        "basilica/roof/principal-rafter-reveal",
        QuantityPolicy::non_negative(),
    )?;
    let drum_radius =
        network.declare::<Length>("basilica/crossing/drum-radius", QuantityPolicy::positive())?;
    let crossing_platform_inset = network.declare::<Length>(
        "basilica/crossing/platform-inset",
        QuantityPolicy::positive(),
    )?;
    let platform_inner_radius = network.declare::<Length>(
        "basilica/crossing/platform-inner-radius",
        QuantityPolicy::positive(),
    )?;
    let cornice_overhang = network.declare::<Length>(
        "basilica/crossing/cornice-overhang",
        QuantityPolicy::positive(),
    )?;
    let cornice_radius = network.declare::<Length>(
        "basilica/crossing/cornice-radius",
        QuantityPolicy::positive(),
    )?;
    let dome_overhang = network.declare::<Length>(
        "basilica/crossing/dome-overhang",
        QuantityPolicy::positive(),
    )?;
    let dome_radius =
        network.declare::<Length>("basilica/crossing/dome-radius", QuantityPolicy::positive())?;
    let drum_height =
        network.declare::<Length>("basilica/crossing/drum-height", QuantityPolicy::positive())?;
    let dome_height =
        network.declare::<Length>("basilica/crossing/dome-height", QuantityPolicy::positive())?;
    let drum_base_above_ridge = network.declare::<Length>(
        "basilica/crossing/drum-base-above-ridge",
        QuantityPolicy::positive(),
    )?;
    let drum_base =
        network.declare::<Offset>("basilica/crossing/drum-base", QuantityPolicy::positive())?;
    let platform_height = network.declare::<Length>(
        "basilica/crossing/platform-height",
        QuantityPolicy::positive(),
    )?;
    let platform_base = network.declare::<Offset>(
        "basilica/crossing/platform-base",
        QuantityPolicy::positive(),
    )?;
    let drum_top =
        network.declare::<Offset>("basilica/crossing/drum-top", QuantityPolicy::positive())?;
    let dome_top =
        network.declare::<Offset>("basilica/crossing/dome-top", QuantityPolicy::positive())?;
    let apse_radius =
        network.declare::<Length>("basilica/east-end/apse-radius", QuantityPolicy::positive())?;
    let apse_inner_radius = network.declare::<Length>(
        "basilica/east-end/apse-inner-radius",
        QuantityPolicy::positive(),
    )?;
    let conch_overhang = network.declare::<Length>(
        "basilica/east-end/conch-overhang",
        QuantityPolicy::positive(),
    )?;
    let conch_radius =
        network.declare::<Length>("basilica/east-end/conch-radius", QuantityPolicy::positive())?;
    let conch_height =
        network.declare::<Length>("basilica/east-end/conch-height", QuantityPolicy::positive())?;
    let north_wall_seat = network.declare::<Point3>(
        "basilica/roof/north-wall-seat",
        QuantityPolicy::unrestricted(),
    )?;
    let south_wall_seat = network.declare::<Point3>(
        "basilica/roof/south-wall-seat",
        QuantityPolicy::unrestricted(),
    )?;
    let ridge_point =
        network.declare::<Point3>("basilica/roof/ridge-point", QuantityPolicy::unrestricted())?;
    let north_roof_eave = network.declare::<Point3>(
        "basilica/roof/north-eave-point",
        QuantityPolicy::unrestricted(),
    )?;
    let south_roof_eave = network.declare::<Point3>(
        "basilica/roof/south-eave-point",
        QuantityPolicy::unrestricted(),
    )?;

    // Relation keys carry alphabetic phase prefixes only to make the intended
    // closed-form frontier obvious in diagnostics. Readiness, not the prefix,
    // remains the actual direction selector.
    network.relate(OffsetByLength::new(
        "basilica/plan/a-east-end",
        origin.clone(),
        length.clone(),
        east_end.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(ScaleLength::new(
        "basilica/plan/b-half-total",
        total_width.clone(),
        half_total.clone(),
        Rational::new(1, 2).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(Sum::new(
        "basilica/plan/c-side-aisles",
        span.clone(),
        side_aisles_width.clone(),
        total_width.clone(),
    )?)?;
    network.relate(ScaleLength::new(
        "basilica/plan/d-aisle-run",
        side_aisles_width.clone(),
        aisle_run.clone(),
        Rational::new(1, 2).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/e-north-total",
        origin.clone(),
        half_total.clone(),
        north_total.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/f-south-total",
        origin.clone(),
        half_total.clone(),
        south_total.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/g-crossing-center",
        origin.clone(),
        crossing_station.clone(),
        crossing_center.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(Sum::new(
        "basilica/plan/h-crossing-half-width",
        drum_radius.clone(),
        crossing_clearance.clone(),
        crossing_half_width.clone(),
    )?)?;
    network.relate(ScaleLength::new(
        "basilica/plan/i-crossing-span",
        crossing_half_width.clone(),
        crossing_span.clone(),
        Rational::new(2, 1).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/j-crossing-west",
        crossing_center.clone(),
        crossing_half_width.clone(),
        crossing_west.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/k-crossing-east",
        crossing_center.clone(),
        crossing_half_width.clone(),
        crossing_east.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/l-west-nave",
        origin.clone(),
        west_nave_length.clone(),
        crossing_west.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/m-east-nave",
        crossing_east.clone(),
        east_nave_length.clone(),
        east_end.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/n-buttress-start",
        origin.clone(),
        buttress_west_inset.clone(),
        buttress_start.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/o-buttress-end",
        east_end.clone(),
        buttress_east_inset.clone(),
        buttress_end.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/p-nave-truss-west-start",
        origin.clone(),
        nave_truss_end_clearance.clone(),
        nave_truss_west_start.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/q-nave-truss-west-end",
        crossing_west.clone(),
        nave_truss_end_clearance.clone(),
        nave_truss_west_end.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(ScaleLength::new(
        "basilica/plan/r-nave-truss-east-half-length",
        east_nave_length.clone(),
        nave_truss_east_half_length.clone(),
        Rational::new(1, 2).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/plan/s-nave-truss-east",
        crossing_east.clone(),
        nave_truss_east_half_length.clone(),
        nave_truss_east.clone(),
        OffsetDirection::Positive,
    )?)?;
    for (key, height, datum) in [
        (
            "basilica/levels/a-wall-head",
            nave_wall_height.clone(),
            wall_head.clone(),
        ),
        (
            "basilica/levels/b-aisle-wall-top",
            aisle_wall_height.clone(),
            aisle_wall_top.clone(),
        ),
        (
            "basilica/levels/c-clerestory-base",
            clerestory_base_height.clone(),
            clerestory_base.clone(),
        ),
        (
            "basilica/levels/d-clerestory-sill",
            clerestory_sill_height_from_ground.clone(),
            clerestory_sill.clone(),
        ),
        (
            "basilica/levels/e-clerestory-spring",
            clerestory_spring_height_from_ground.clone(),
            clerestory_spring.clone(),
        ),
        (
            "basilica/levels/f-crossing-spandrel-base",
            crossing_spandrel_base_height.clone(),
            crossing_spandrel_base.clone(),
        ),
        (
            "basilica/levels/g-crossing-web-bottom",
            crossing_web_bottom_height.clone(),
            crossing_web_bottom.clone(),
        ),
        (
            "basilica/levels/h-crossing-web-middle",
            crossing_web_middle_height.clone(),
            crossing_web_middle.clone(),
        ),
        (
            "basilica/levels/i-apse-wall-top",
            apse_wall_height.clone(),
            apse_wall_top.clone(),
        ),
    ] {
        network.relate(OffsetByLength::new(
            key,
            origin.clone(),
            height,
            datum,
            OffsetDirection::Positive,
        )?)?;
    }
    network.relate(OffsetByLength::new(
        "basilica/levels/j-clerestory-height",
        clerestory_base.clone(),
        clerestory_height.clone(),
        wall_head.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/levels/k-clerestory-local-sill",
        clerestory_base.clone(),
        clerestory_sill_height.clone(),
        clerestory_sill.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/levels/l-clerestory-local-spring",
        clerestory_base.clone(),
        clerestory_spring_height.clone(),
        clerestory_spring.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(Sum::new(
        "basilica/aisle/a-roof-run",
        aisle_run.clone(),
        aisle_roof_overhang.clone(),
        aisle_roof_run.clone(),
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/aisle/b-inner-height",
        aisle_wall_top.clone(),
        aisle_roof_inner_lift.clone(),
        aisle_roof_inner_height.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/aisle/c-bearing-height",
        aisle_wall_top.clone(),
        aisle_roof_bearing_inset.clone(),
        aisle_roof_bearing_height.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(Sum::new(
        "basilica/aisle/d-bearing-drop",
        aisle_roof_inner_lift.clone(),
        aisle_roof_bearing_inset.clone(),
        aisle_roof_bearing_drop.clone(),
    )?)?;
    network.relate(Pitch::new(
        "basilica/aisle/e-pitch",
        aisle_run.clone(),
        aisle_roof_bearing_drop.clone(),
        aisle_roof_pitch.clone(),
    )?)?;
    network.relate(Pitch::new(
        "basilica/aisle/f-full-drop",
        aisle_roof_run.clone(),
        aisle_roof_drop.clone(),
        aisle_roof_pitch.clone(),
    )?)?;
    network.relate(Pythagorean::new(
        "basilica/aisle/g-slope-length",
        aisle_roof_run.clone(),
        aisle_roof_drop.clone(),
        aisle_roof_slope_length.clone(),
    )?)?;
    network.relate(ScaleLength::new(
        "basilica/roof/a-half-span",
        span.clone(),
        half_span.clone(),
        Rational::new(1, 2).map_err(|_| BuildError::InvalidRelation)?,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/b-north-half-span",
        origin.clone(),
        half_span.clone(),
        north_half_span.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/c-south-half-span",
        origin.clone(),
        half_span.clone(),
        south_half_span.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/d-wall-plate-top",
        wall_head.clone(),
        wall_plate_height.clone(),
        wall_plate_top.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/e-ridge-height",
        wall_plate_top.clone(),
        rise.clone(),
        ridge_height.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(Pitch::new(
        "basilica/roof/f-pitch",
        half_span.clone(),
        rise.clone(),
        pitch.clone(),
    )?)?;
    network.relate(Pythagorean::new(
        "basilica/roof/g-principal-rafter",
        half_span.clone(),
        rise.clone(),
        rafter_length.clone(),
    )?)?;
    network.relate(Pitch::new(
        "basilica/roof/h-overhang-drop",
        overhang.clone(),
        overhang_drop.clone(),
        pitch.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/roof/i-full-run",
        half_span.clone(),
        overhang.clone(),
        roof_run.clone(),
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/j-north-full-run",
        origin.clone(),
        roof_run.clone(),
        north_roof_run.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/k-south-full-run",
        origin.clone(),
        roof_run.clone(),
        south_roof_run.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/roof/l-eave-height",
        wall_plate_top.clone(),
        overhang_drop.clone(),
        roof_eave_height.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(Sum::new(
        "basilica/roof/m-full-rise",
        rise.clone(),
        overhang_drop.clone(),
        roof_total_rise.clone(),
    )?)?;
    network.relate(Pythagorean::new(
        "basilica/roof/n-skin-slope",
        roof_run.clone(),
        roof_total_rise.clone(),
        roof_slope_length.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/o-north-wall-seat",
        origin.clone(),
        north_half_span.clone(),
        wall_plate_top.clone(),
        north_wall_seat.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/p-south-wall-seat",
        origin.clone(),
        south_half_span.clone(),
        wall_plate_top.clone(),
        south_wall_seat.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/q-ridge-point",
        origin.clone(),
        origin.clone(),
        ridge_height.clone(),
        ridge_point.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/r-north-eave",
        origin.clone(),
        north_roof_run.clone(),
        roof_eave_height.clone(),
        north_roof_eave.clone(),
    )?)?;
    network.relate(ComposePoint::new(
        "basilica/roof/s-south-eave",
        origin.clone(),
        south_roof_run.clone(),
        roof_eave_height.clone(),
        south_roof_eave.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/crossing/a-platform-inner-radius",
        platform_inner_radius.clone(),
        crossing_platform_inset.clone(),
        drum_radius.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/crossing/b-cornice-radius",
        drum_radius.clone(),
        cornice_overhang.clone(),
        cornice_radius.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/crossing/c-dome-radius",
        drum_radius.clone(),
        dome_overhang.clone(),
        dome_radius.clone(),
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/crossing/d-drum-base",
        ridge_height.clone(),
        drum_base_above_ridge.clone(),
        drum_base.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/crossing/e-platform-base",
        drum_base.clone(),
        platform_height.clone(),
        platform_base.clone(),
        OffsetDirection::Negative,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/crossing/f-drum-top",
        drum_base.clone(),
        drum_height.clone(),
        drum_top.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(OffsetByLength::new(
        "basilica/crossing/g-dome-top",
        drum_top.clone(),
        dome_height.clone(),
        dome_top.clone(),
        OffsetDirection::Positive,
    )?)?;
    network.relate(Sum::new(
        "basilica/east-end/a-inner-radius",
        apse_inner_radius.clone(),
        wall_thickness.clone(),
        apse_radius.clone(),
    )?)?;
    network.relate(Sum::new(
        "basilica/east-end/b-conch-radius",
        apse_radius.clone(),
        conch_overhang.clone(),
        conch_radius.clone(),
    )?)?;

    let quantities = BasilicaQuantities {
        origin,
        length,
        east_end,
        span,
        half_span,
        total_width,
        half_total,
        aisle_run,
        wall_thickness,
        crossing_station,
        crossing_center,
        crossing_clearance,
        crossing_half_width,
        crossing_span,
        crossing_west,
        crossing_east,
        west_nave_length,
        east_nave_length,
        arcade_bays,
        nave_truss_bays,
        buttress_west_inset,
        buttress_east_inset,
        buttress_start,
        buttress_end,
        nave_truss_end_clearance,
        nave_truss_west_start,
        nave_truss_west_end,
        nave_truss_east,
        nave_wall_height,
        wall_head,
        aisle_wall_height,
        aisle_wall_top,
        clerestory_base_height,
        clerestory_base,
        clerestory_sill_height_from_ground,
        clerestory_sill,
        clerestory_spring_height_from_ground,
        clerestory_spring,
        clerestory_height,
        clerestory_sill_height,
        clerestory_spring_height,
        crossing_spandrel_base_height,
        crossing_spandrel_base,
        crossing_web_bottom_height,
        crossing_web_bottom,
        crossing_web_middle_height,
        crossing_web_middle,
        apse_wall_height,
        apse_wall_top,
        aisle_roof_overhang,
        aisle_roof_run,
        aisle_roof_inner_lift,
        aisle_roof_inner_height,
        aisle_roof_bearing_inset,
        aisle_roof_bearing_height,
        aisle_roof_bearing_drop,
        aisle_roof_pitch,
        aisle_roof_drop,
        aisle_roof_slope_length,
        aisle_roof_depth,
        wall_plate_height,
        wall_plate_top,
        rise,
        ridge_height,
        pitch,
        rafter_length,
        overhang,
        overhang_drop,
        roof_run,
        roof_eave_height,
        roof_slope_length,
        roof_skin_depth,
        wall_plate_width,
        principal_rafter_width,
        principal_rafter_depth,
        principal_rafter_reveal,
        drum_radius,
        crossing_platform_inset,
        platform_inner_radius,
        cornice_overhang,
        cornice_radius,
        dome_overhang,
        dome_radius,
        drum_height,
        dome_height,
        drum_base_above_ridge,
        drum_base,
        platform_height,
        platform_base,
        drum_top,
        dome_top,
        apse_radius,
        apse_inner_radius,
        conch_overhang,
        conch_radius,
        conch_height,
        north_wall_seat,
        south_wall_seat,
        ridge_point,
        north_roof_eave,
        south_roof_eave,
    };
    Ok((network.finish()?, quantities))
}
