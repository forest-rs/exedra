// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact ruled lofts and variable sections, remeshed with construction charts.

mod support;
use exedra::constructive::ir::Placement3;
use exedra::{
    Mesh,
    analytic::{
        End,
        loft::{FaceRole, Loft, LoftPolicy},
        realize::RealizationPolicy,
    },
    constructive::{
        builders,
        chart::{ChartTransform, LoftStations, SurfaceChart},
        loft::{BoundaryPoint, CorrespondencePolicy, CorrespondenceProfile, correspond_profiles},
        profile::Profile2,
    },
    profile::{LoopRef, ProfilePolicy},
};
use std::error::Error;
use support::{Exhibit, export};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PROFILE: ProfilePolicy = ProfilePolicy::new(1e-7);
const GRID: ChartTransform = ChartTransform {
    matrix: [[0.05, 0.0], [0.0, 0.05]],
    offset: [0.0; 2],
};
const CIRCLE_CORNERS: [BoundaryPoint; 4] = [
    BoundaryPoint::new(0, 0.25),
    BoundaryPoint::new(0, 0.75),
    BoundaryPoint::new(1, 0.25),
    BoundaryPoint::new(1, 0.75),
];
const RECT_CORNERS: [BoundaryPoint; 4] = [
    BoundaryPoint::new(0, 0.0),
    BoundaryPoint::new(1, 0.0),
    BoundaryPoint::new(2, 0.0),
    BoundaryPoint::new(3, 0.0),
];
fn transition(hollow: bool) -> Result<Mesh> {
    let base = if hollow {
        builders::ring(35.0, 25.0)?
    } else {
        builders::circle(35.0)?
    };
    let top = builders::rect_centered(80.0, 56.0)?;
    let top = if hollow {
        top.with_cutouts_at(&builders::rect_centered(64.0, 40.0)?, [(0.0, 0.0)])?
    } else {
        top
    };
    let mut first =
        CorrespondenceProfile::new(&base).with_landmarks(LoopRef::Outer, CIRCLE_CORNERS);
    let mut last = CorrespondenceProfile::new(&top).with_landmarks(LoopRef::Outer, RECT_CORNERS);
    if hollow {
        let clockwise = [
            BoundaryPoint::new(1, 0.75),
            BoundaryPoint::new(0, 0.25),
            BoundaryPoint::new(0, 0.75),
            BoundaryPoint::new(1, 0.25),
        ];
        first = first.with_landmarks(LoopRef::Hole(0), clockwise);
        last = last.with_landmarks(LoopRef::Hole(0), RECT_CORNERS);
    }
    let matched = correspond_profiles(&[first, last], &CorrespondencePolicy::new(PROFILE))?;
    realize(
        matched
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    Placement3::translate(0.0, 0.0, 100.0 * i as f64),
                    &p.profile,
                )
            })
            .collect(),
    )
}
fn variable_section() -> Result<Mesh> {
    let profiles = [
        builders::rounded_rect_centered(52.0, 38.0, 8.0)?,
        builders::rounded_rect_centered(80.0, 52.0, 12.0)?,
        builders::rounded_rect_centered(60.0, 40.0, 6.0)?,
        builders::rounded_rect_centered(68.0, 44.0, 10.0)?,
    ];
    let frames = [
        Placement3::IDENTITY,
        Placement3::rotate_z_then_translate(0.25, 18.0, 0.0, 40.0),
        Placement3::rotate_z_then_translate(-0.15, -12.0, 8.0, 80.0),
        Placement3::rotate_z_then_translate(0.2, 0.0, 0.0, 120.0),
    ];
    realize(frames.into_iter().zip(&profiles).collect())
}
fn realize(sections: Vec<(Placement3, &Profile2)>) -> Result<Mesh> {
    let mut stations = vec![0.0];
    for pair in sections.windows(2) {
        let a = pair[0].0.rows.map(|r| r[3]);
        let b = pair[1].0.rows.map(|r| r[3]);
        stations
            .push(stations.last().unwrap() + (b[0] - a[0]).hypot(b[1] - a[1]).hypot(b[2] - a[2]));
    }
    let chart = SurfaceChart::Loft {
        reference_section: 0,
        rest_length: *stations.last().unwrap(),
        stations: LoftStations::DatumDistance,
        wall: GRID,
        caps: GRID,
    };
    let loft = Loft::from_profiles(sections, Placement3::IDENTITY, &LoftPolicy::new(PROFILE))?;
    assert!(loft.body().is_closed(), "oriented native boundary");
    let output = loft.realize_with_uvs(
        &RealizationPolicy {
            tolerance: 0.05,
            ..RealizationPolicy::default()
        },
        0,
        &stations,
        |_, f| match f.role {
            FaceRole::Cap { end: End::Start } => 0,
            FaceRole::Cap { end: End::End } => 1,
            FaceRole::Wall { .. } => 2,
        },
        |role, p| match role {
            FaceRole::Wall { .. } => chart.map_loft_wall_triangle(p),
            FaceRole::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "native deviation {:?}",
        output.report
    );
    println!(
        "{} native faces, {} shared edges → {} vertices, {} triangles; total deviation {:.6} mm",
        loft.body().faces().len(),
        loft.body().edges().len(),
        output.mesh.vertices().count(),
        output.mesh.faces().count(),
        output.report.max_deviation
    );
    Ok(output.mesh)
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "native-lofts.glb".to_owned());
    export(
        &path,
        "retained ruled lofts",
        vec![
            Exhibit {
                name: "native circle to rectangle",
                mesh: transition(false)?,
                placement: Placement3::translate(-115.0, 0.0, 0.0),
                color: [0.18, 0.42, 0.66, 1.0],
                grid: true,
                cut_regions: &[0, 1],
            },
            Exhibit {
                name: "native variable rounded sections",
                mesh: variable_section()?,
                placement: Placement3::IDENTITY,
                color: [0.75, 0.59, 0.29, 1.0],
                grid: true,
                cut_regions: &[0, 1],
            },
            Exhibit {
                name: "native hollow transition",
                mesh: transition(true)?,
                placement: Placement3::translate(115.0, 0.0, 0.0),
                color: [0.12, 0.52, 0.42, 1.0],
                grid: true,
                cut_regions: &[0, 1],
            },
        ],
    )
}
