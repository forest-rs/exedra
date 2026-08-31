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
        "nave-truss-east-00-tie-beam",
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
        "nave-truss-west-00-king-post",
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
    let dirty: Vec<_> = change
        .dirty
        .iter()
        .map(|item| item.element.as_ref())
        .collect();

    for member in [
        "tie-beam",
        "principal-rafter-north",
        "principal-rafter-south",
        "king-post",
        "king-post-key",
        "diagonal-brace-north",
        "diagonal-brace-south",
    ] {
        let west = format!("nave-truss-west-00-{member}");
        let east = format!("nave-truss-east-00-{member}");
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
    // Dirty bindings can update only stable identities that already exist.
    // Changing the repeat count therefore needs an explicit topology signal;
    // an unrepresentable count must fail before architecture uses `u32` math.
    let baseline = BasilicaSetout::new(&BasilicaPremises::default()).unwrap();
    let edited = BasilicaPremises {
        arcade_bays: Count::new(8),
        ..BasilicaPremises::default()
    };
    let change = baseline.reconfigure(&edited).unwrap();

    assert!(change.topology_changed);
    assert_eq!(change.plan.arcade_bays, Count::new(8));
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
    assert!(
        length_change
            .dirty
            .iter()
            .any(|item| item.element.as_ref() == "buttress-north-end"),
        "non-default topology needs a complete existing-element inventory"
    );
}
