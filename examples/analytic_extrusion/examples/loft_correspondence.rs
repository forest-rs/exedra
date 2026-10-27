// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Circle/rectangle lofts through exact profile correspondence, with visible charts.
//! `cargo run --release -p analytic_extrusion --example loft_correspondence -- /tmp/lofts.glb`

mod support;
use exedra::{
    Mesh,
    constructive::{
        builders,
        chart::{ChartTransform, LoftStations, SurfaceChart},
        ir::{CapMode, LoftPolicy, Placement3},
        loft::{BoundaryPoint, CorrespondencePolicy, CorrespondenceProfile, correspond_profiles},
        tessellate::{EvalPolicy, tessellate_loft_with_chart},
    },
    profile::{LoopRef, ProfilePolicy},
};
use std::error::Error;
use support::{Exhibit, export};
type Result<T> = std::result::Result<T, Box<dyn Error>>;

const POLICY: CorrespondencePolicy = CorrespondencePolicy::new(ProfilePolicy::new(1e-7));
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

fn loft(smooth: bool, hollow: bool) -> Result<Mesh> {
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
    let shoulder = builders::rounded_rect_centered(86.0, 60.0, 6.0)?;
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
    let inputs = if smooth {
        vec![
            first,
            CorrespondenceProfile::new(&shoulder).with_landmarks(
                LoopRef::Outer,
                [
                    BoundaryPoint::new(7, 0.5),
                    BoundaryPoint::new(1, 0.5),
                    BoundaryPoint::new(3, 0.5),
                    BoundaryPoint::new(5, 0.5),
                ],
            ),
            last,
        ]
    } else {
        vec![first, last]
    };
    let matched = correspond_profiles(&inputs, &POLICY)?;
    println!(
        "{} input segments → {} matched segments across {} sections; knot adjustment {:.3e}",
        matched.report.input_segments,
        matched.report.output_segments,
        matched.profiles.len(),
        matched.report.max_knot_adjustment
    );
    let sections: Vec<_> = matched
        .profiles
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let t = i as f64 / (matched.profiles.len() - 1) as f64;
            (
                Placement3::translate(if smooth && i == 1 { 20.0 } else { 0.0 }, 0.0, 100.0 * t),
                &p.profile,
            )
        })
        .collect();
    let mut policy = EvalPolicy::default();
    policy.discretize.chord_tolerance = 0.05;
    policy.loft.chord_tolerance = 0.05;
    let output = tessellate_loft_with_chart(
        &sections,
        if smooth {
            LoftPolicy::Smooth
        } else {
            LoftPolicy::Ruled
        },
        CapMode::Both,
        SurfaceChart::Loft {
            reference_section: 0,
            rest_length: 100.0,
            stations: LoftStations::DatumDistance,
            wall: GRID,
            caps: GRID,
        },
        &policy,
    )?;
    let sampling = output
        .loft_sampling
        .as_ref()
        .expect("loft sampling evidence");
    assert!(
        sampling.within_surface_tolerance(),
        "emitted side-surface bound: {sampling:?}"
    );
    println!(
        "{} shared spans, emitted side-surface bound {:.6} (profile chord tolerance separate)",
        sampling.spans.len(),
        sampling.surface_deviation_bound()
    );
    println!(
        "mesh loft: {} vertices, {} faces; original segment intervals remain in the correspondence result",
        output.mesh.vertices().count(),
        output.mesh.faces().count()
    );
    Ok(output.mesh)
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "lofts.glb".to_owned());
    export(
        &path,
        "explicit profile correspondence",
        vec![
            Exhibit {
                name: "ruled circle to rectangle",
                mesh: loft(false, false)?,
                placement: Placement3::translate(-115.0, 0.0, 0.0),
                color: [0.18, 0.42, 0.66, 1.0],
                cut_regions: &[0, 1],
            },
            Exhibit {
                name: "smooth rounded transition",
                mesh: loft(true, false)?,
                placement: Placement3::IDENTITY,
                color: [0.75, 0.59, 0.29, 1.0],
                cut_regions: &[0, 1],
            },
            Exhibit {
                name: "hollow circle to rectangle",
                mesh: loft(false, true)?,
                placement: Placement3::translate(115.0, 0.0, 0.0),
                color: [0.12, 0.52, 0.42, 1.0],
                cut_regions: &[0, 1],
            },
        ],
    )
}
