// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native rounded turns and smooth round-section sweeps, with construction charts.
//! `cargo run --release -p analytic_extrusion --example rounded_geometry -- /tmp/rounded.glb`

use exedra::Mesh;
use exedra::analytic::{
    realize::RealizationPolicy,
    revolve::{FaceRole as TurnFace, Revolution},
    sweep::{FaceRole as SweepFace, Sweep},
};
use exedra::constructive::ir::Placement3;
use exedra::constructive::{
    builders,
    chart::{ChartTransform, SurfaceChart},
    path::PathCursor3,
};
use exedra::profile::ProfilePolicy;
use std::{
    error::Error,
    f64::consts::{PI, TAU},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PROFILE: ProfilePolicy = ProfilePolicy::new(1e-7);
const REALIZE: RealizationPolicy = RealizationPolicy {
    tolerance: 0.04,
    geometry_gap: 1e-9,
    min_arc_segments: 4,
    max_edge_segments: 4096,
    max_vertices: 1 << 20,
    max_boundary_samples: 1 << 22,
};
const GRID: ChartTransform = ChartTransform {
    matrix: [[0.05, 0.0], [0.0, 0.05]],
    offset: [0.0; 2],
};

fn turn(rounded: bool) -> Result<Mesh> {
    let (profile, sweep) = if rounded {
        (
            builders::rounded_rect_centered(30.0, 38.0, 5.0)?.translated((60.0, 19.0))?,
            TAU,
        )
    } else {
        (builders::circle(12.0)?.translated((60.0, 12.0))?, 1.5 * PI)
    };
    let upright =
        Placement3::from_axes([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0], [0.0; 3]);
    let solid = Revolution::from_profile(profile, upright, sweep, &PROFILE)?;
    let chart = SurfaceChart::Revolve {
        wall: GRID,
        caps: GRID,
        reference_radius: 60.0,
    };
    let output = solid.realize_with_uvs(
        &REALIZE,
        |_, face| u32::from(matches!(face.role, TurnFace::Cap { .. })),
        |role, p| match role {
            TurnFace::Wall { .. } => chart.map_revolve_wall_triangle(p),
            TurnFace::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "rounded turn accuracy: {:?}",
        output.report
    );
    println!(
        "turn: {} native faces, volume {:.6}, area {:.6}, {} triangles, deviation {:.6}",
        solid.body().faces().len(),
        solid.volume(),
        solid.surface_area(),
        output.mesh.faces().count(),
        output.report.max_deviation
    );
    Ok(output.mesh)
}

fn rail(hollow: bool) -> Result<Mesh> {
    let profile = builders::circle(10.0)?;
    let profile = if hollow {
        profile.with_cutouts_at(&builders::circle(6.0)?, [(0.0, 0.0)])?
    } else {
        profile
    };
    let path = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
        .tangent_arc(40.0, PI, [0.0, 0.0, 1.0])
        .tangent_arc(40.0, -0.5 * PI, [0.0, 0.0, 1.0])
        .line_by([-40.0, 0.0, 0.0])
        .finish([1.0, 0.0, 0.0])?;
    let solid = Sweep::from_profile(
        profile,
        &path,
        Placement3::translate(0.0, 0.0, 10.0),
        &PROFILE,
    )?;
    let chart = SurfaceChart::Sweep {
        wall: GRID,
        caps: GRID,
    };
    let output = solid.realize_with_uvs(
        &REALIZE,
        |_, face| u32::from(matches!(face.role, SweepFace::Cap { .. })),
        |role, p| match role {
            SweepFace::Wall { .. } => chart.map_sweep_wall_triangle(p),
            SweepFace::Cap { .. } => chart.map_cap_triangle(p),
        },
    )?;
    assert!(
        output.report.within_tolerance(),
        "rounded sweep accuracy: {:?}",
        output.report
    );
    println!(
        "rail: {} native faces, volume {:.6}, area {:.6}, {} triangles, deviation {:.6}",
        solid.body().faces().len(),
        solid.volume(),
        solid.surface_area(),
        output.mesh.faces().count(),
        output.report.max_deviation
    );
    Ok(output.mesh)
}

mod support;
use support::{Exhibit, export};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rounded.glb".to_owned());
    export(
        &path,
        "native rounded geometry",
        vec![
            Exhibit {
                name: "rounded native turn",
                mesh: turn(true)?,
                placement: Placement3::translate(-100.0, 100.0, 0.0),
                color: [0.18, 0.42, 0.66, 1.0],
                grid: true,
                cut_regions: &[1],
            },
            Exhibit {
                name: "toroidal cutaway",
                mesh: turn(false)?,
                placement: Placement3::translate(100.0, 100.0, 0.0),
                color: [0.75, 0.59, 0.29, 1.0],
                grid: true,
                cut_regions: &[1],
            },
            Exhibit {
                name: "round native sweep",
                mesh: rail(false)?,
                placement: Placement3::translate(-20.0, -80.0, 0.0),
                color: [0.12, 0.52, 0.42, 1.0],
                grid: true,
                cut_regions: &[1],
            },
            Exhibit {
                name: "hollow native sweep",
                mesh: rail(true)?,
                placement: Placement3::translate(180.0, -80.0, 0.0),
                color: [0.55, 0.25, 0.15, 1.0],
                grid: true,
                cut_regions: &[1],
            },
        ],
    )
}
