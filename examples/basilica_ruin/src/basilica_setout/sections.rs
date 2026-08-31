// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Resolved exact sections consumed by the basilica's architecture systems.

use exedra_math::{cross, normalize, sub};
use setout::{Count, Length, Offset, Point3, Rational};
use setout_joiner::lower_point;

/// Side of the symmetrical nave roof.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RoofSide {
    /// North slope, positive Y.
    North,
    /// South slope, negative Y.
    South,
}

impl RoofSide {
    /// Returns `1` for north and `-1` for south.
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            Self::North => 1.0,
            Self::South => -1.0,
        }
    }
}

/// Exact resolved horizontal massing and principal station datums.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanSection {
    /// Length of the rectangular basilica before the apse springs.
    pub length: Length,
    /// Clear width between the nave wall centerlines.
    pub nave_width: Length,
    /// Half of [`PlanSection::nave_width`].
    pub half_nave: Length,
    /// Overall width across both aisles.
    pub total_width: Length,
    /// Half of [`PlanSection::total_width`].
    pub half_total: Length,
    /// Clear transverse run of either side aisle.
    pub aisle_run: Length,
    /// Common masonry wall thickness used by the primary fabric.
    pub wall_thickness: Length,
    /// East end of the rectangular nave as an X coordinate.
    pub east_end: Offset,
    /// Crossing center as an X coordinate.
    pub crossing_center: Offset,
    /// Half-width of the square crossing stage.
    pub crossing_half_width: Length,
    /// Full width of the square crossing stage.
    pub crossing_span: Length,
    /// West edge of the open crossing bay.
    pub crossing_west: Offset,
    /// East edge of the open crossing bay.
    pub crossing_east: Offset,
    /// Length of the nave segment west of the crossing.
    pub west_nave_length: Length,
    /// Length of the nave segment east of the crossing.
    pub east_nave_length: Length,
    /// Number of repeated longitudinal arcade bays.
    pub arcade_bays: Count,
}

/// Exact resolved vertical datums shared by masonry and roofs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelSection {
    /// Ground datum.
    pub ground: Offset,
    /// Top of the exterior aisle wall.
    pub aisle_wall_top: Offset,
    /// Base of the upper clerestory wall profiles.
    pub clerestory_base: Offset,
    /// Clerestory opening sill datum.
    pub clerestory_sill: Offset,
    /// Clerestory arch spring datum.
    pub clerestory_spring: Offset,
    /// Local clerestory wall height above its base.
    pub clerestory_height: Length,
    /// Local opening-sill height above the clerestory base.
    pub clerestory_sill_height: Length,
    /// Local arch-spring height above the clerestory base.
    pub clerestory_spring_height: Length,
    /// Base datum of the crossing spandrel profiles.
    pub crossing_spandrel_base: Offset,
    /// Lower datum of the faceted crossing transition webs.
    pub crossing_web_bottom: Offset,
    /// Middle datum of the faceted crossing transition webs.
    pub crossing_web_middle: Offset,
    /// Top datum of the eastern apse wall.
    pub apse_wall_top: Offset,
}

/// Exact resolved lean-to aisle-roof section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AisleSection {
    /// Horizontal bearing run between nave and aisle wall centerlines.
    pub bearing_run: Length,
    /// Horizontal eave projection beyond the aisle wall centerline.
    pub overhang: Length,
    /// Full roof run including the eave projection.
    pub roof_run: Length,
    /// Roof underside at the inner nave wall.
    pub inner_height: Offset,
    /// Roof underside where it crosses the aisle wall.
    pub bearing_height: Offset,
    /// Vertical drop across [`AisleSection::bearing_run`].
    pub bearing_drop: Length,
    /// Exact aisle-roof rise/run ratio.
    pub pitch: Rational,
    /// Vertical drop across [`AisleSection::roof_run`].
    pub roof_drop: Length,
    /// Full sloping length of the aisle roof.
    pub slope_length: Length,
    /// Modeled roof-skin thickness.
    pub roof_depth: Length,
}

/// Exact resolved crossing-stage, drum, and dome massing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossingSection {
    /// Circumradius of the polygonal drum.
    pub drum_radius: Length,
    /// Radius of the platform opening beneath the drum.
    pub platform_inner_radius: Length,
    /// Radius of the drum cornices.
    pub cornice_radius: Length,
    /// Radius of the dome shell at its spring.
    pub dome_radius: Length,
    /// Height of the drum wall panels.
    pub drum_height: Length,
    /// Rise of the dome above the drum.
    pub dome_height: Length,
    /// Underside datum of the crossing bearing platform.
    pub platform_base: Offset,
    /// Thickness of the crossing bearing platform.
    pub platform_height: Length,
    /// Base datum of the drum walls.
    pub drum_base: Offset,
    /// Top datum of the drum walls and spring of the dome.
    pub drum_top: Offset,
    /// Apex datum of the dome.
    pub dome_top: Offset,
}

/// Exact resolved eastern apse and conch massing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EastEndSection {
    /// Outer radius of the masonry apse wall.
    pub apse_radius: Length,
    /// Inner radius of the masonry apse wall.
    pub apse_inner_radius: Length,
    /// Height of the masonry apse wall.
    pub apse_wall_height: Length,
    /// Outer radius of the roof conch.
    pub conch_radius: Length,
    /// Rise of the roof conch.
    pub conch_height: Length,
}

/// Exact resolved roof section shared by the ruin and structure laboratory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoofSection {
    /// Clear nave span between wall centerlines.
    pub span: Length,
    /// Half of [`RoofSection::span`].
    pub half_span: Length,
    /// Masonry wall head before the timber wall plate.
    pub wall_head: Offset,
    /// Continuous wall-plate height.
    pub wall_plate_height: Length,
    /// Top bearing datum of the wall plate.
    pub wall_plate_top: Offset,
    /// Rise from the wall-plate bearing datum to the ridge.
    pub rise: Length,
    /// Exact ridge datum.
    pub ridge_height: Offset,
    /// Exact rise/run ratio.
    pub pitch: Rational,
    /// Principal-rafter length selected to the nearest iota with a root certificate.
    pub rafter_length: Length,
    /// Horizontal roof overhang beyond the wall centerline.
    pub overhang: Length,
    /// Vertical fall across the overhang at the shared pitch.
    pub overhang_drop: Length,
    /// Full horizontal run from ridge to outer eave.
    pub roof_run: Length,
    /// Underside datum at the outer eave.
    pub roof_eave_height: Offset,
    /// Full sloping roof-skin length from ridge to outer eave.
    pub roof_slope_length: Length,
    /// Modeled roof-skin thickness.
    pub roof_skin_depth: Length,
    /// Wall-plate width across the wall.
    pub wall_plate_width: Length,
    /// Principal-rafter width along the nave.
    pub principal_rafter_width: Length,
    /// Principal-rafter depth normal to the slope.
    pub principal_rafter_depth: Length,
    /// Deliberate visual reveal between principal rafters and roof skin.
    pub principal_rafter_reveal: Offset,
    /// North wall-plate seat point at X=0.
    pub north_wall_seat: Point3,
    /// South wall-plate seat point at X=0.
    pub south_wall_seat: Point3,
    /// Ridge point at X=0.
    pub ridge_point: Point3,
    /// North outer-eave underside point at X=0.
    pub north_roof_eave: Point3,
    /// South outer-eave underside point at X=0.
    pub south_roof_eave: Point3,
}

impl RoofSection {
    /// Returns the exact structural wall seat on one side.
    #[must_use]
    pub const fn wall_seat(&self, side: RoofSide) -> Point3 {
        match side {
            RoofSide::North => self.north_wall_seat,
            RoofSide::South => self.south_wall_seat,
        }
    }

    /// Returns the exact outer roof-skin eave point on one side.
    #[must_use]
    pub const fn roof_eave(&self, side: RoofSide) -> Point3 {
        match side {
            RoofSide::North => self.north_roof_eave,
            RoofSide::South => self.south_roof_eave,
        }
    }

    /// Returns the inward/up unit slope from a wall plate to the ridge.
    #[must_use]
    pub fn inward_slope(&self, side: RoofSide) -> [f64; 3] {
        normalize(sub(
            lower_point(self.ridge_point),
            lower_point(self.wall_seat(side)),
        ))
        .expect("resolved roof seats never coincide with the ridge")
    }

    /// Returns the outward/up unit normal of one roof slope.
    #[must_use]
    pub fn outward_normal(&self, side: RoofSide) -> [f64; 3] {
        let x_axis = [1.0, 0.0, 0.0];
        match side {
            RoofSide::North => cross(self.inward_slope(side), x_axis),
            RoofSide::South => cross(x_axis, self.inward_slope(side)),
        }
    }
}
