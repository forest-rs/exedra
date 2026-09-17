// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{
    builders,
    clearance::{BoundaryPolicy, PlanarPatch},
    ir::{CapMode, Placement3, Plane3},
    profile::{Loop2, Profile2, Seg2},
    section::{SectionPolicy, section_body},
    tessellate::{EvalPolicy, tessellate_extrude},
    workplane::{SurfaceSelector, WorkplaneAttachment, WorkplanePolicy},
};
use alloc::vec;

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-10, "{a} != {b}");
}

#[test]
fn patch_and_section_measure_off_center_hole_in_their_own_frames() {
    let profile = Profile2::new(
        builders::rect(6.0, 4.0).unwrap().outer().clone(),
        vec![
            Loop2::new(vec![
                Seg2::line((1.0, 1.0)),
                Seg2::line((1.0, 2.0)),
                Seg2::line((3.0, 2.0)),
                Seg2::line((3.0, 1.0)),
            ])
            .unwrap(),
        ],
    )
    .unwrap();
    let body = tessellate_extrude(
        &profile,
        &Placement3::IDENTITY,
        2.0,
        CapMode::Both,
        &EvalPolicy::default(),
    )
    .unwrap();
    let workplane = WorkplaneAttachment {
        surface: SurfaceSelector::EndCap,
        anchor: [1.0, 0.0, 0.0],
        projection: [0.0, 0.0, 1.0],
        x_direction: [0.0, 1.0, 0.0],
    }
    .resolve(&body, &WorkplanePolicy::default())
    .unwrap();
    let patch = PlanarPatch::from_workplane(&body, &workplane, &BoundaryPolicy::default()).unwrap();
    let m = patch.measure().unwrap();
    near(m.area, 22.0);
    near(m.perimeter, 26.0);
    let c = m.centroid.unwrap();
    // Body centroid is (68/22, 45/22); rotated/translated local frame.
    near(c[0], 45.0 / 22.0);
    near(c[1], 1.0 - 68.0 / 22.0);
    assert_eq!(
        m.bounds,
        Some(PlanarBounds {
            min: [0.0, -5.0],
            max: [4.0, 1.0]
        })
    );
    assert_eq!(m.edges_examined, 8);
    let section = section_body(
        &body,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 1.0,
        },
        &SectionPolicy::default(),
    )
    .unwrap();
    let s = section.measure().unwrap();
    near(s.area, m.area);
    near(s.perimeter, m.perimeter);
    let c = s.centroid.unwrap();
    let axes = section.frame;
    // Use the exposed placement to return the centroid to body coordinates.
    let body_center = axes.rows.map(|row| row[0] * c[0] + row[1] * c[1] + row[3]);
    near(body_center[0], 68.0 / 22.0);
    near(body_center[1], 45.0 / 22.0);
    near(body_center[2], 1.0);
    let empty = section_body(
        &body,
        Plane3 {
            normal: [0.0, 0.0, 1.0],
            distance: 3.0,
        },
        &SectionPolicy::default(),
    )
    .unwrap()
    .measure()
    .unwrap();
    assert_eq!(empty.area, 0.0);
    assert_eq!(empty.perimeter, 0.0);
    assert_eq!(empty.centroid, None);
    assert_eq!(empty.bounds, None);
    assert_eq!(empty.edges_examined, 0);
}

#[test]
fn concavity_and_disconnected_regions_keep_area_weighted_centroids() {
    let concave = [
        [0.0, 0.0],
        [4.0, 0.0],
        [4.0, 1.0],
        [1.0, 1.0],
        [1.0, 4.0],
        [0.0, 4.0],
    ];
    let m = measure([(&concave[..], false)]).unwrap();
    near(m.area, 7.0);
    near(m.perimeter, 16.0);
    for v in m.centroid.unwrap() {
        near(v, 9.5 / 7.0);
    }
    let a = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
    let b = a.map(|p| [p[0] + 1e12, p[1] + 1e12]);
    let m = measure([(&a[..], false), (&b[..], false)]).unwrap();
    assert_eq!(m.area, 8.0);
    assert_eq!(m.centroid, Some([5e11 + 1.0; 2]));
    let translated = measure([(&b[..], false)]).unwrap();
    assert_eq!(translated.area, 4.0);
    assert_eq!(translated.centroid, Some([1e12 + 1.0; 2]));
}

#[test]
fn invalid_winding_degeneracy_and_nonfinite_arithmetic_are_refused() {
    let triangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    assert_eq!(
        measure([(&triangle[..], true)]),
        Err(MeasurementError::InvalidBoundary)
    );
    let line = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
    assert_eq!(
        measure([(&line[..], false)]),
        Err(MeasurementError::InvalidBoundary)
    );
    let mut bad = triangle;
    bad[0][0] = f64::NAN;
    assert_eq!(
        measure([(&bad[..], false)]),
        Err(MeasurementError::NumericLimit)
    );
    // Area remains finite, but its first moment would silently underflow.
    let tiny = triangle.map(|p| p.map(|v| v * 1e-110));
    assert_eq!(
        measure([(&tiny[..], false)]),
        Err(MeasurementError::NumericLimit)
    );
    let huge = triangle.map(|p| p.map(|v| v * 1e200));
    assert_eq!(
        measure([(&huge[..], false)]),
        Err(MeasurementError::NumericLimit)
    );
}

#[test]
fn public_section_aggregates_regions_and_centroid_can_lie_in_a_hole() {
    use crate::section::{PlaneSection, SectionLoop, SectionRegion, SectionStats};
    use crate::tessellate::Feature;
    let boundary = |points: &[[f64; 2]]| SectionLoop {
        points: points.to_vec(),
        edge_features: vec![Feature::Imported; points.len()],
    };
    let mut section = PlaneSection {
        frame: Placement3::IDENTITY,
        regions: vec![SectionRegion {
            outer: boundary(&[[-2.0, -2.0], [2.0, -2.0], [2.0, 2.0], [-2.0, 2.0]]),
            holes: vec![boundary(&[
                [-1.0, -1.0],
                [-1.0, 1.0],
                [1.0, 1.0],
                [1.0, -1.0],
            ])],
        }],
        stats: SectionStats::default(),
    };
    let ring = section.measure().unwrap();
    assert_eq!(ring.area, 12.0);
    assert_eq!(ring.perimeter, 24.0);
    assert_eq!(ring.centroid, Some([0.0; 2]));
    section.regions.push(SectionRegion {
        outer: boundary(&[[4.0, 0.0], [6.0, 0.0], [6.0, 2.0], [4.0, 2.0]]),
        holes: vec![],
    });
    let m = section.measure().unwrap();
    assert_eq!(m.area, 16.0);
    assert_eq!(m.perimeter, 32.0);
    assert_eq!(m.centroid, Some([1.25, 0.25]));
    assert_eq!(
        m.bounds,
        Some(PlanarBounds {
            min: [-2.0, -2.0],
            max: [6.0, 2.0]
        })
    );
    assert_eq!(m.edges_examined, 12);
    section.regions[1].outer.points.reverse();
    assert_eq!(section.measure(), Err(MeasurementError::InvalidBoundary));
}
