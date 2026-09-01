// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;

#[test]
fn roof_skin_passes_through_the_wall_plate_and_overhangs_below_it() {
    // The previous visual roof was pitched from its outer overhang directly
    // to the ridge. At the wall line it merely happened to pass near the
    // timber. This test guards the corrected construction logic: the exact
    // slope passes through the wall-plate top, then continues outward and
    // down by the same rational pitch.
    let setout = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let roof = setout.roof();
    assert_eq!(roof.wall_plate_top, Offset::millimeters(11_180).unwrap());
    assert_eq!(roof.ridge_height, Offset::millimeters(14_380).unwrap());
    assert_eq!(roof.pitch, Rational::new(32, 45).unwrap());
    assert_eq!(
        roof.overhang_drop,
        Length::from_iota(2_240_000_000).unwrap()
    );
    assert_eq!(
        roof.roof_eave_height
            .checked_add_length(roof.overhang_drop)
            .unwrap(),
        roof.wall_plate_top
    );
    assert!(
        setout
            .explain_ridge()
            .contains("basilica/root/nave-wall-height")
    );
}

#[test]
fn principal_rafter_is_lowered_once_from_exact_endpoints() {
    // This is the complete Setout→Joiner seam. The analytic extent, its
    // frame bits, and every provenance link come from the evaluated binding;
    // the test does not author a second world-space endpoint.
    let setout = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    for side in [RoofSide::North, RoofSide::South] {
        let geometry = setout.principal_rafter_geometry(side).unwrap();
        assert!(geometry.extent.is_well_formed());
        assert_eq!(geometry.bindings.len(), 5);
        assert_eq!(
            geometry.extent.size[0],
            setout.roof().rafter_length.as_meters()
        );
    }
}

#[test]
fn roof_rise_edit_is_incremental_exact_and_excludes_unrelated_systems() {
    // A roof edit must recompute its dependent spine, report named geometry,
    // carry the crossing whose datum follows the ridge, and leave the apse
    // outside the dirty frontier. Warm/fresh equality is checked inside
    // `reconfigure` before the result is returned.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let mut edited = BasilicaPremises::default();
    edited.roof_rise = edited
        .roof_rise
        .checked_add(Length::millimeters(250).unwrap())
        .unwrap();
    let change = baseline.reconfigure(&edited).unwrap();
    assert_eq!(change.roof.rise, Length::millimeters(3_450).unwrap());
    assert!(!change.topology_changed);
    assert!(change.work.steps_reused > 0);
    assert!(change.work.steps_evaluated > 0);
    let dirty: Vec<_> = change
        .dirty
        .iter()
        .map(|item| item.element.as_ref())
        .collect();
    assert!(dirty.contains(&"nave-roof-north-west"));
    assert!(dirty.contains(&"east-chancel-gable"));
    assert!(dirty.contains(&"crossing-dome"));
    assert!(dirty.contains(&"principal-rafter-south-west"));
    assert!(!dirty.iter().any(|key| key.contains("apse")));
}

#[test]
fn length_edit_reports_the_eastward_rebuild_frontier() {
    // The crossing station is a fixed premise. Extending the rectangular
    // basilica therefore moves and lengthens the east fabric, repeats the
    // buttress pattern, and repositions the east truss without dirtying
    // the west facade or the crossing dome.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let mut edited = BasilicaPremises::default();
    edited.length = edited
        .length
        .checked_add(Length::meters(2).unwrap())
        .unwrap();
    let change = baseline.reconfigure(&edited).unwrap();
    assert_eq!(change.plan.length, Length::meters(38).unwrap());
    assert!(!change.topology_changed);
    assert_eq!(
        change.plan.east_nave_length,
        baseline
            .plan()
            .east_nave_length
            .checked_add(Length::meters(2).unwrap())
            .unwrap()
    );
    assert_eq!(change.plan.crossing_center, baseline.plan().crossing_center);

    let dirty: Vec<_> = change
        .dirty
        .iter()
        .map(|item| item.element.as_ref())
        .collect();
    for expected in [
        "nave-wall-north-east",
        "aisle-roof-north",
        "east-chancel-gable",
        "east-apse",
        "buttress-north-end",
        "nave-truss-east-tie-beam",
    ] {
        assert!(
            dirty.contains(&expected),
            "missing dirty element {expected}"
        );
    }
    assert!(!dirty.contains(&"west-facade"));
    assert!(!dirty.contains(&"crossing-dome"));
}

#[test]
fn nave_wall_height_edit_moves_the_shared_upper_datum() {
    // Raising the nave wall head must carry the roof, trusses, and crossing
    // vertically while preserving the independently rooted aisle and apse.
    // This guards the vertical half of the whole-building setout boundary.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let mut edited = BasilicaPremises::default();
    edited.nave_wall_height = edited
        .nave_wall_height
        .checked_add(Length::millimeters(500).unwrap())
        .unwrap();
    let change = baseline.reconfigure(&edited).unwrap();
    assert!(!change.topology_changed);

    assert_eq!(
        change.levels.clerestory_height,
        baseline
            .levels()
            .clerestory_height
            .checked_add(Length::millimeters(500).unwrap())
            .unwrap()
    );
    assert_eq!(
        change.roof.ridge_height.as_meters() - baseline.roof().ridge_height.as_meters(),
        0.5
    );
    assert_eq!(change.aisle, *baseline.aisle());
    assert_eq!(change.east_end, *baseline.east_end());

    let dirty: Vec<_> = change
        .dirty
        .iter()
        .map(|item| item.element.as_ref())
        .collect();
    for expected in [
        "nave-wall-north-west",
        "nave-roof-north-west",
        "crossing-dome",
        "nave-truss-west-start-king-post",
    ] {
        assert!(
            dirty.contains(&expected),
            "missing dirty element {expected}"
        );
    }
    assert!(!dirty.contains(&"aisle-wall-north"));
    assert!(!dirty.contains(&"east-apse"));
}

#[test]
fn crossing_station_edit_invalidates_complete_truss_frames() {
    // Truss station placement is derived from the edges of the crossing bay.
    // Every member in a frame must move together; leaving keys or braces clean
    // would permit a future incremental builder to tear the assembly apart.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let mut edited = BasilicaPremises::default();
    edited.crossing_station = edited
        .crossing_station
        .checked_add(Length::meters(1).unwrap())
        .unwrap();
    let change = baseline.reconfigure(&edited).unwrap();
    assert!(
        change
            .west_truss_delta
            .changed()
            .iter()
            .any(|key| key.as_str() == "basilica/nave-trusses/west/end"),
        "moving the crossing must report the exact generated endpoint payload"
    );
    let dirty: Vec<_> = change
        .dirty
        .iter()
        .map(|item| item.element.as_ref())
        .collect();

    for member in crate::NAVE_TRUSS_MEMBER_SUFFIXES {
        let west = format!("nave-truss-west-start-{member}");
        let east = format!("nave-truss-east-{member}");
        assert!(dirty.contains(&west.as_str()), "missing {west}");
        assert!(dirty.contains(&east.as_str()), "missing {east}");
    }
}

#[test]
fn aisle_wall_height_edit_invalidates_the_stepped_west_facade() {
    // The west facade profile contains the aisle wall-head datum even though
    // it is built by the nave module. Its binding must cross that module seam.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let mut edited = BasilicaPremises::default();
    edited.aisle_wall_height = edited
        .aisle_wall_height
        .checked_add(Length::millimeters(100).unwrap())
        .unwrap();
    let change = baseline.reconfigure(&edited).unwrap();

    assert!(
        change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "west-facade")
    );
}

#[test]
fn reconstruction_catalogue_covers_every_active_root_and_method() {
    // Adding a whole-building premise must never silently downgrade the
    // reconstruction sidecar into a partial assessment.
    let setout = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    assert!(setout.reconstruction().findings.is_empty());
}

#[test]
fn contradictory_positive_premises_fail_before_geometry_lowering() {
    // Individual dimensions can all be positive while their combination is
    // impossible. The exact network, not downstream profile assertions, must
    // reject a zero-width aisle or a crossing that extends past the east end.
    let mut no_aisles = BasilicaPremises::default();
    no_aisles.total_width = no_aisles.nave_width;
    assert!(BasilicaSetout::new(&no_aisles).is_err());

    let crossing_past_end = BasilicaPremises {
        crossing_station: Length::meters(35).unwrap(),
        ..BasilicaPremises::default()
    };
    assert!(BasilicaSetout::new(&crossing_past_end).is_err());
}

#[test]
fn arcade_count_edit_reports_topology_rebuild_and_rejects_inventory_overflow() {
    // The exterior repeat and the split nave runs share exact margins but not
    // a false modular relationship: changing exterior bays must leave the
    // accepted five-west/one-east nave topology and crossing gap untouched.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    assert_eq!(baseline.outer_arcade_bays().items().len(), 7);
    assert_eq!(baseline.west_arcade_bays().items().len(), 5);
    assert_eq!(baseline.east_arcade_bays().items().len(), 1);
    assert_eq!(
        baseline.west_arcade_bays().items()[0].center(),
        Rational::new(i128::from(Offset::millimeters(3_730).unwrap().iota()), 1).unwrap()
    );
    assert_eq!(
        baseline.east_arcade_bays().items()[0].center(),
        Rational::new(i128::from(Offset::millimeters(2_650).unwrap().iota()), 1).unwrap()
    );
    let edited = BasilicaPremises {
        arcade_bays: Count::new(8),
        ..BasilicaPremises::default()
    };
    let change = baseline.reconfigure(&edited).unwrap();

    assert!(change.topology_changed);
    assert_eq!(change.plan.arcade_bays, Count::new(8));
    assert_eq!(change.plan.west_arcade_bays, Count::new(5));
    assert_eq!(
        change.outer_arcade_delta.added(),
        &[setout_generate::ItemKey::new("basilica/arcades/outer/bay/000008").unwrap()]
    );
    assert!(change.west_arcade_delta.is_empty());
    assert!(change.east_arcade_delta.is_empty());
    assert_eq!(
        change.buttress_delta.added(),
        &[setout_generate::ItemKey::new("basilica/aisle-buttresses/interior/000007").unwrap()]
    );
    assert!(
        change
            .buttress_delta
            .retained()
            .iter()
            .any(|key| key.as_str() == "basilica/aisle-buttresses/end")
    );
    assert!(
        change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "aisle-wall-north")
    );

    let too_many = BasilicaPremises {
        arcade_bays: Count::new(u64::from(u32::MAX)),
        ..BasilicaPremises::default()
    };
    assert!(matches!(
        BasilicaSetout::new(&too_many),
        Err(BasilicaSetoutError::ArcadeBayCountTooLarge { .. })
    ));
    let eight_bay = BasilicaSetout::new(&edited).unwrap();
    let mut length_edit = edited;
    length_edit.length = length_edit
        .length
        .checked_add(Length::meters(1).unwrap())
        .unwrap();
    let length_change = eight_bay.reconfigure(&length_edit).unwrap();
    assert!(!length_change.topology_changed);
    assert!(length_change.outer_arcade_delta.added().is_empty());
    assert!(length_change.outer_arcade_delta.removed().is_empty());
    assert_eq!(length_change.outer_arcade_delta.changed().len(), 8);
    assert!(length_change.west_arcade_delta.is_empty());
    assert_eq!(length_change.east_arcade_delta.changed().len(), 1);
    assert!(
        length_change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "buttress-north-end"),
        "non-default topology needs a complete existing-element inventory"
    );
}

#[test]
fn truss_count_edit_preserves_endpoints_and_keeps_the_ruin_omission_semantic() {
    // The west truss run is the first generated station that expands into
    // several assembly members. Growing it must add one station identity,
    // retain both endpoints, move only redistributed interiors, and keep the
    // authored ruin attached to `interior/000002` rather than an ordinal slot.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    assert_eq!(
        baseline.plan().nave_truss_west_start,
        Offset::meters(2).unwrap()
    );
    assert_eq!(
        baseline.plan().nave_truss_west_end,
        Offset::millimeters(19_300).unwrap()
    );
    assert_eq!(
        baseline.plan().nave_truss_east,
        Offset::millimeters(33_350).unwrap()
    );
    assert!(
        baseline
            .west_truss_stations()
            .orphaned_overrides()
            .is_empty()
    );
    assert!(
        baseline
            .west_truss_stations()
            .items()
            .iter()
            .all(|station| station.label().as_str() != "interior/000002")
    );

    let edited = BasilicaPremises {
        nave_truss_bays: Count::new(6),
        ..BasilicaPremises::default()
    };
    let change = baseline.reconfigure(&edited).unwrap();
    assert!(change.topology_changed);
    assert_eq!(change.west_truss_stations.items().len(), 6);
    assert_eq!(
        change.west_truss_delta.added(),
        &[setout_generate::ItemKey::new("basilica/nave-trusses/west/interior/000005").unwrap()]
    );
    assert!(change.west_truss_delta.removed().is_empty());
    assert!(change.west_truss_delta.orphaned_overrides().is_empty());
    for retained in [
        "basilica/nave-trusses/west/start",
        "basilica/nave-trusses/west/end",
    ] {
        assert!(
            change
                .west_truss_delta
                .retained()
                .iter()
                .any(|key| key.as_str() == retained),
            "missing retained truss endpoint {retained}"
        );
    }
    assert!(
        change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "nave-truss-west-start-tie-beam")
    );

    let six_bay = BasilicaSetout::new(&edited).unwrap();
    let seven_bay = BasilicaPremises {
        nave_truss_bays: Count::new(7),
        ..BasilicaPremises::default()
    };
    let second_change = six_bay.reconfigure(&seven_bay).unwrap();
    assert_eq!(
        second_change.west_truss_delta.added(),
        &[setout_generate::ItemKey::new("basilica/nave-trusses/west/interior/000006").unwrap()]
    );
    assert!(
        second_change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "nave-truss-west-interior-000005-tie-beam"),
        "a non-default baseline must bind every retained generated member"
    );

    let smaller = BasilicaPremises {
        nave_truss_bays: Count::new(2),
        ..BasilicaPremises::default()
    };
    let smaller = BasilicaSetout::new(&smaller).unwrap();
    assert_eq!(smaller.west_truss_stations().items().len(), 3);
    assert_eq!(smaller.west_truss_stations().orphaned_overrides().len(), 1);
    assert_eq!(
        smaller.west_truss_stations().orphaned_overrides()[0]
            .target()
            .as_str(),
        "interior/000002"
    );

    let too_many = BasilicaPremises {
        nave_truss_bays: Count::new(u64::from(u32::MAX)),
        ..BasilicaPremises::default()
    };
    assert!(matches!(
        BasilicaSetout::new(&too_many),
        Err(BasilicaSetoutError::NaveTrussBayCountTooLarge { .. })
    ));
}
