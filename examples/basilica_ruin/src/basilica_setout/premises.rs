// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use setout::{Count, Length};

/// Exact authored premises controlling the example's architectural massing.
///
/// These values are physical sizes or station distances, never world-space
/// coordinates. [`super::BasilicaSetout`](crate::BasilicaSetout) derives signed
/// coordinates and all dependent datums from the western ground origin.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BasilicaPremises {
    /// Length of the rectangular nave before the apse springs.
    pub length: Length,
    /// Clear width of the high central nave.
    pub nave_width: Length,
    /// Overall width across both side aisles.
    pub total_width: Length,
    /// Height of the masonry nave wall head below the timber wall plate.
    pub nave_wall_height: Length,
    /// Height of the exterior aisle arcade walls.
    pub aisle_wall_height: Length,
    /// Rise from the wall-plate bearing datum to the roof ridge.
    pub roof_rise: Length,
    /// Station distance from the west datum to the crossing center.
    pub crossing_station: Length,
    /// Circumradius of the polygonal drum.
    pub drum_radius: Length,
    /// Height of the drum walls between cornices.
    pub drum_height: Length,
    /// Rise of the shallow dome above the drum.
    pub dome_height: Length,
    /// Radius of the semicircular eastern apse.
    pub apse_radius: Length,
    /// Number of repeated arcade bays along each longitudinal wall.
    pub arcade_bays: Count,
}

impl Default for BasilicaPremises {
    fn default() -> Self {
        Self {
            length: millimeters(36_000),
            nave_width: millimeters(9_000),
            total_width: millimeters(18_000),
            nave_wall_height: millimeters(11_000),
            aisle_wall_height: millimeters(5_200),
            roof_rise: millimeters(3_200),
            crossing_station: millimeters(26_000),
            drum_radius: millimeters(4_100),
            drum_height: millimeters(2_600),
            dome_height: millimeters(3_100),
            apse_radius: millimeters(4_500),
            arcade_bays: Count::new(7),
        }
    }
}

fn millimeters(value: u64) -> Length {
    Length::millimeters(value).expect("default basilica dimensions are positive and bounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_premises_are_exact_and_dimensionally_valid() {
        // The public authoring surface must contain neither floating roots nor
        // a signed coordinate disguised as a physical size.
        let premises = BasilicaPremises::default();
        assert_eq!(premises.length, Length::meters(36).unwrap());
        assert_eq!(premises.drum_radius, Length::millimeters(4_100).unwrap());
        assert_eq!(premises.arcade_bays.get(), 7);
    }
}
