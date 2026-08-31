// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Quantity-to-element invalidation for basilica consumers.

use setout::QuantityKey;
use setout_generate::LinearFragment;
use setout_joiner::{BindingIndex, DirtyChannel};

use super::BasilicaQuantities;
use crate::{
    NAVE_TRUSS_EAST_STATION_KEY, NAVE_TRUSS_MEMBER_SUFFIXES, buttress_instance_key,
    truss_member_instance_key, west_truss_station_key,
};

pub(super) fn build_binding_index(
    quantities: &BasilicaQuantities,
    buttress_stations: &LinearFragment,
    west_truss_stations: &LinearFragment,
) -> BindingIndex {
    let mut bindings = BindingIndex::new();

    // These groups mirror actual architecture-module inputs. They are kept at
    // stable element-name granularity so a future incremental assembly builder
    // can rebuild no more—and no less—than the exact setout delta requires.
    bind_elements(
        &mut bindings,
        &[
            "nave-wall-north-east",
            "nave-wall-south-east",
            "nave-wall-plate-north-east",
            "nave-wall-plate-south-east",
            "nave-roof-north-east",
            "nave-roof-south-east",
            "aisle-wall-north",
            "aisle-wall-south",
            "aisle-roof-north",
            "aisle-roof-south",
            "aisle-eave-north",
            "aisle-eave-south",
            "east-aisle-end-north",
            "east-aisle-end-south",
            "interior-arcade-north-east",
            "interior-arcade-south-east",
            "east-chancel-gable",
            "east-apse",
            "east-apse-roof",
        ],
        &[
            quantities.length.key().clone(),
            quantities.east_end.key().clone(),
            quantities.east_nave_length.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );
    for side in ["north", "south"] {
        // The binding inventory consumes the same generated labels as assembly
        // construction. It never recreates topology or assumes that a current
        // ordinal is durable identity.
        for station in buttress_stations.items() {
            bindings.bind(
                buttress_instance_key(side, station.label()),
                [
                    quantities.length.key().clone(),
                    quantities.arcade_bays.key().clone(),
                    quantities.buttress_start.key().clone(),
                    quantities.buttress_end.key().clone(),
                    quantities.half_total.key().clone(),
                ],
                [DirtyChannel::Geometry, DirtyChannel::Contact],
            );
        }
    }
    for member in NAVE_TRUSS_MEMBER_SUFFIXES {
        bindings.bind(
            truss_member_instance_key(NAVE_TRUSS_EAST_STATION_KEY, member),
            [
                quantities.length.key().clone(),
                quantities.crossing_east.key().clone(),
                quantities.nave_truss_east.key().clone(),
            ],
            [DirtyChannel::Geometry, DirtyChannel::Contact],
        );
    }
    for station in west_truss_stations.items() {
        let station = west_truss_station_key(station.label());
        for member in NAVE_TRUSS_MEMBER_SUFFIXES {
            bindings.bind(
                truss_member_instance_key(&station, member),
                [
                    quantities.crossing_west.key().clone(),
                    quantities.nave_truss_bays.key().clone(),
                    quantities.nave_truss_west_start.key().clone(),
                    quantities.nave_truss_west_end.key().clone(),
                ],
                [DirtyChannel::Geometry, DirtyChannel::Contact],
            );
        }
    }

    bind_elements(
        &mut bindings,
        &[
            "west-facade",
            "aisle-wall-north",
            "aisle-wall-south",
            "aisle-roof-north",
            "aisle-roof-south",
            "aisle-eave-north",
            "aisle-eave-south",
            "east-aisle-end-north",
            "east-aisle-end-south",
        ],
        &[
            quantities.total_width.key().clone(),
            quantities.half_total.key().clone(),
            quantities.aisle_run.key().clone(),
            quantities.aisle_roof_run.key().clone(),
            quantities.aisle_roof_drop.key().clone(),
            quantities.aisle_roof_slope_length.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );

    let nave_masonry = [
        "nave-wall-north-west",
        "nave-wall-south-west-broken",
        "nave-wall-north-east",
        "nave-wall-south-east",
        "interior-arcade-north-west",
        "interior-arcade-south-west",
        "interior-arcade-north-east",
        "interior-arcade-south-east",
    ];
    bind_elements(
        &mut bindings,
        &nave_masonry,
        &[
            quantities.half_span.key().clone(),
            quantities.wall_thickness.key().clone(),
            quantities.clerestory_base.key().clone(),
            quantities.clerestory_height.key().clone(),
            quantities.clerestory_sill_height.key().clone(),
            quantities.clerestory_spring_height.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );
    bind_elements(
        &mut bindings,
        &["aisle-wall-north", "aisle-wall-south"],
        &[
            quantities.length.key().clone(),
            quantities.aisle_wall_top.key().clone(),
            quantities.arcade_bays.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );
    bind_elements(
        &mut bindings,
        &[
            "aisle-roof-north",
            "aisle-roof-south",
            "aisle-eave-north",
            "aisle-eave-south",
            "east-aisle-end-north",
            "east-aisle-end-south",
        ],
        &[
            quantities.aisle_wall_top.key().clone(),
            quantities.aisle_roof_inner_height.key().clone(),
            quantities.aisle_roof_bearing_height.key().clone(),
            quantities.aisle_roof_slope_length.key().clone(),
            quantities.aisle_roof_depth.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );

    let crossing_plan_quantities = [
        quantities.crossing_center.key().clone(),
        quantities.crossing_half_width.key().clone(),
        quantities.crossing_span.key().clone(),
        quantities.crossing_west.key().clone(),
        quantities.crossing_east.key().clone(),
    ];
    bind_elements(
        &mut bindings,
        &[
            "nave-wall-north-west",
            "nave-wall-south-west-broken",
            "nave-wall-north-east",
            "nave-wall-south-east",
            "nave-wall-plate-north-west",
            "nave-wall-plate-south-west-a",
            "nave-wall-plate-south-west-b",
            "nave-wall-plate-north-east",
            "nave-wall-plate-south-east",
            "nave-roof-north-west",
            "nave-roof-south-west-a",
            "nave-roof-south-west-b",
            "nave-roof-north-east",
            "nave-roof-south-east",
            "crossing-shoulder-west",
            "crossing-shoulder-east",
            "interior-arcade-north-west",
            "interior-arcade-south-west",
            "interior-arcade-north-east",
            "interior-arcade-south-east",
        ],
        &crossing_plan_quantities,
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );

    let skin_quantities = [
        quantities.north_roof_eave.key().clone(),
        quantities.south_roof_eave.key().clone(),
        quantities.ridge_point.key().clone(),
        quantities.roof_slope_length.key().clone(),
        quantities.roof_skin_depth.key().clone(),
    ];
    for element in [
        "nave-roof-north-west",
        "nave-roof-south-west-a",
        "nave-roof-south-west-b",
        "nave-roof-north-east",
        "nave-roof-south-east",
    ] {
        bindings.bind(
            element,
            skin_quantities.iter().cloned(),
            [DirtyChannel::Geometry, DirtyChannel::Contact],
        );
    }
    let plate_quantities = [
        quantities.half_span.key().clone(),
        quantities.wall_head.key().clone(),
        quantities.wall_plate_top.key().clone(),
        quantities.wall_plate_height.key().clone(),
        quantities.wall_plate_width.key().clone(),
    ];
    for element in [
        "nave-wall-plate-north-west",
        "nave-wall-plate-south-west-a",
        "nave-wall-plate-south-west-b",
        "nave-wall-plate-north-east",
        "nave-wall-plate-south-east",
        "wall-plate-north",
        "wall-plate-south",
    ] {
        bindings.bind(
            element,
            plate_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }
    let truss_section_quantities = [
        quantities.span.key().clone(),
        quantities.half_span.key().clone(),
        quantities.wall_head.key().clone(),
        quantities.wall_plate_top.key().clone(),
        quantities.ridge_height.key().clone(),
        quantities.pitch.key().clone(),
        quantities.north_wall_seat.key().clone(),
        quantities.south_wall_seat.key().clone(),
        quantities.ridge_point.key().clone(),
        quantities.rafter_length.key().clone(),
        quantities.principal_rafter_width.key().clone(),
        quantities.principal_rafter_depth.key().clone(),
        quantities.principal_rafter_reveal.key().clone(),
    ];
    for station in west_truss_stations
        .items()
        .iter()
        .map(|station| west_truss_station_key(station.label()))
        .chain([NAVE_TRUSS_EAST_STATION_KEY.to_owned()])
    {
        for member in NAVE_TRUSS_MEMBER_SUFFIXES {
            bindings.bind(
                truss_member_instance_key(&station, member),
                truss_section_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
    }
    for element in [
        "west-facade",
        "crossing-shoulder-west",
        "crossing-shoulder-east",
        "east-chancel-gable",
    ] {
        bindings.bind(
            element,
            [
                quantities.span.key().clone(),
                quantities.wall_plate_top.key().clone(),
                quantities.ridge_height.key().clone(),
            ],
            [DirtyChannel::Geometry],
        );
    }
    bindings.bind(
        "west-facade",
        [quantities.aisle_wall_top.key().clone()],
        [DirtyChannel::Geometry],
    );

    // The crossing's vertical datum is intentionally tied to the nave ridge.
    // A rise edit therefore moves the whole bearing stage, not merely the dome
    // skin; omitting these names would advertise an unsafe partial rebuild.
    let crossing_quantities = [
        quantities.crossing_center.key().clone(),
        quantities.crossing_span.key().clone(),
        quantities.crossing_spandrel_base.key().clone(),
        quantities.crossing_web_bottom.key().clone(),
        quantities.crossing_web_middle.key().clone(),
        quantities.platform_inner_radius.key().clone(),
        quantities.platform_height.key().clone(),
        quantities.platform_base.key().clone(),
        quantities.drum_radius.key().clone(),
        quantities.drum_height.key().clone(),
        quantities.drum_base.key().clone(),
        quantities.drum_top.key().clone(),
        quantities.cornice_radius.key().clone(),
        quantities.dome_radius.key().clone(),
        quantities.dome_height.key().clone(),
        quantities.ridge_height.key().clone(),
    ];
    for element in [
        "crossing-pier-south-west",
        "crossing-pier-north-west",
        "crossing-pier-south-east",
        "crossing-pier-north-east",
        "crossing-spandrel-south",
        "crossing-spandrel-north",
        "crossing-spandrel-west",
        "crossing-spandrel-east",
        "crossing-platform",
        "crossing-drum-cornice-base",
        "crossing-drum-cornice-top",
        "crossing-dome",
        "crossing-pendentive-north-east",
        "crossing-pendentive-north-west",
        "crossing-pendentive-south-west",
        "crossing-pendentive-south-east",
    ] {
        bindings.bind(
            element,
            crossing_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }
    for face in 0..12 {
        bindings.bind(
            format!("crossing-drum-panel-{face:02}"),
            crossing_quantities.iter().cloned(),
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
    }

    bind_elements(
        &mut bindings,
        &["east-apse", "east-apse-roof"],
        &[
            quantities.east_end.key().clone(),
            quantities.apse_radius.key().clone(),
            quantities.apse_inner_radius.key().clone(),
            quantities.apse_wall_height.key().clone(),
            quantities.conch_radius.key().clone(),
            quantities.conch_height.key().clone(),
        ],
        &[DirtyChannel::Geometry, DirtyChannel::Contact],
    );

    // The structure lab is a second real consumer of the same exact roof
    // spine. These mappings are grouped by construction role so a delta names
    // the complete affected assembly while leaving the east apse unrelated.
    for side in ["south", "north"] {
        bindings.bind(
            format!("masonry-{side}"),
            [
                quantities.half_span.key().clone(),
                quantities.wall_head.key().clone(),
            ],
            [
                DirtyChannel::Geometry,
                DirtyChannel::Contact,
                DirtyChannel::LoadPath,
            ],
        );
        let eave = if side == "south" {
            quantities.south_roof_eave.key().clone()
        } else {
            quantities.north_roof_eave.key().clone()
        };
        let roof_quantities = [
            eave,
            quantities.ridge_point.key().clone(),
            quantities.roof_slope_length.key().clone(),
            quantities.roof_skin_depth.key().clone(),
        ];
        for element in [
            format!("roof-covering-{side}"),
            format!("boarding-{side}"),
            format!("common-rafter-{side}-00"),
            format!("common-rafter-{side}-01"),
            format!("common-rafter-{side}-02"),
            format!("purlin-{side}-eave"),
            format!("purlin-{side}-mid"),
            format!("purlin-{side}-upper"),
        ] {
            bindings.bind(
                element,
                roof_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
    }
    let lab_frame_quantities = [
        quantities.span.key().clone(),
        quantities.half_span.key().clone(),
        quantities.wall_head.key().clone(),
        quantities.wall_plate_top.key().clone(),
        quantities.ridge_height.key().clone(),
        quantities.pitch.key().clone(),
        quantities.rafter_length.key().clone(),
    ];
    for station in ["west", "east"] {
        for element in [
            format!("tie-beam-{station}"),
            format!("principal-rafter-south-{station}"),
            format!("principal-rafter-north-{station}"),
            format!("king-post-{station}"),
            format!("strut-south-{station}"),
            format!("strut-north-{station}"),
        ] {
            bindings.bind(
                element,
                lab_frame_quantities.iter().cloned(),
                [
                    DirtyChannel::Geometry,
                    DirtyChannel::Contact,
                    DirtyChannel::LoadPath,
                ],
            );
        }
    }
    bindings
}

fn bind_elements(
    bindings: &mut BindingIndex,
    elements: &[&str],
    quantities: &[QuantityKey],
    channels: &[DirtyChannel],
) {
    for &element in elements {
        bindings.bind(
            element,
            quantities.iter().cloned(),
            channels.iter().copied(),
        );
    }
}
