// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native construction and construction-independent versus extrusion realization.
//! `cargo run --release -p constructive_wind_tunnel --bin analytic_body -- 9`
//! Inputs are built once outside realization timings; output destruction is included.

use exedra_analytic::extrusion::{Extrusion, TessellationPolicy};
use exedra_analytic::realize::RealizationPolicy;
use exedra_constructive::builders;
use exedra_constructive::ir::Placement3;
use exedra_profile::{Profile, ProfilePolicy};
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

fn profile(holes: u32) -> Result<Profile, Box<dyn Error>> {
    let columns = holes.div_ceil(4).max(1);
    let width = 8.0 * f64::from(columns + 1);
    let cutouts: Vec<_> = (0..holes)
        .map(|i| {
            builders::circle(1.5)?.translated((
                8.0 * f64::from(i / 4 + 1) - width * 0.5,
                8.0 * f64::from(i % 4) - 12.0,
            ))
        })
        .collect::<Result<_, _>>()?;
    Ok(builders::rounded_rect_centered(width, 40.0, 3.0)?
        .with_cutouts(cutouts)?
        .to_line_arc(&ProfilePolicy::new(1e-6))?)
}

fn median(samples: &[f64]) -> f64 {
    let mut samples = samples.to_vec();
    samples.sort_unstable_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() -> Result<(), Box<dyn Error>> {
    if let Ok(mode) = std::env::var("EXEDRA_ANALYTIC_PROFILE") {
        return allocation_profile(mode == "body");
    }
    let repetitions: u32 = std::env::args().nth(1).map_or(Ok(9), |x| x.parse())?;
    if repetitions == 0 {
        return Err("repetitions must be positive".into());
    }
    println!("holes native_faces triangles construct_us extrusion_us body_us ratio");
    for holes in [0, 8, 32, 128] {
        let profile = profile(holes)?;
        let solid = Extrusion::new(profile.clone(), Placement3::IDENTITY, 5.0)?;
        let body_policy = RealizationPolicy::default();
        let extrusion_policy = TessellationPolicy::default();
        let specific = solid.tessellate(&extrusion_policy)?;
        let generic = solid.body().realize(Placement3::IDENTITY, &body_policy)?;
        for mesh in [&specific.mesh, &generic.mesh] {
            assert!(
                mesh.boundary_loops()?.is_empty(),
                "both paths must remain closed"
            );
            assert!(
                mesh.validate_deep().is_empty(),
                "both paths must have valid topology"
            );
        }
        assert!(
            specific.report.within_tolerance(),
            "extrusion accuracy must meet policy"
        );
        assert!(
            generic.report.within_tolerance(),
            "body accuracy must meet policy"
        );
        assert_eq!(
            specific
                .mesh
                .faces()
                .map(|f| specific.mesh.face_loop(f).count() - 2)
                .sum::<usize>(),
            generic.mesh.faces().count(),
            "same circular sampling must produce the same triangle count"
        );
        let mut construction = Vec::new();
        let mut specific_times = Vec::new();
        let mut generic_times = Vec::new();
        let batch = 3;
        // Alternate order on successive samples to reduce ordering bias.
        for iteration in 0..repetitions + 2 {
            let start = Instant::now();
            for _ in 0..batch {
                black_box(Extrusion::new(
                    black_box(profile.clone()),
                    Placement3::IDENTITY,
                    5.0,
                )?);
            }
            let elapsed = start.elapsed().as_secs_f64() * 1e6 / f64::from(batch);
            if iteration >= 2 {
                construction.push(elapsed);
            }
            for generic_first in [iteration % 2 == 0, iteration % 2 != 0] {
                let start = Instant::now();
                for _ in 0..batch {
                    if generic_first {
                        black_box(
                            solid
                                .body()
                                .realize(Placement3::IDENTITY, black_box(&body_policy))?,
                        );
                    } else {
                        black_box(solid.tessellate(black_box(&extrusion_policy))?);
                    }
                }
                let elapsed = start.elapsed().as_secs_f64() * 1e6 / f64::from(batch);
                if iteration >= 2 {
                    if generic_first {
                        generic_times.push(elapsed);
                    } else {
                        specific_times.push(elapsed);
                    }
                }
            }
        }
        let (specific_us, generic_us) = (median(&specific_times), median(&generic_times));
        println!(
            "{holes} {} {} {:.3} {specific_us:.3} {generic_us:.3} {:.3}",
            solid.faces().len(),
            generic.mesh_faces.len(),
            median(&construction),
            generic_us / specific_us
        );
    }
    Ok(())
}

// Pause around exactly three realization calls for external allocation tools.
// This keeps allocator instrumentation and unsafe code out of the benchmark.
fn allocation_profile(generic: bool) -> Result<(), Box<dyn Error>> {
    use std::io::{self, Write};
    let solid = Extrusion::new(profile(8)?, Placement3::IDENTITY, 5.0)?;
    let body_policy = RealizationPolicy::default();
    let extrusion_policy = TessellationPolicy::default();
    let run = || -> Result<(), Box<dyn Error>> {
        if generic {
            black_box(solid.body().realize(Placement3::IDENTITY, &body_policy)?);
        } else {
            black_box(solid.tessellate(&extrusion_policy)?);
        }
        Ok(())
    };
    run()?;
    flush_stack_log();
    println!(
        "profile_ready pid={} generic={generic}; Enter to realize three times",
        std::process::id()
    );
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    for _ in 0..3 {
        run()?;
    }
    flush_stack_log();
    println!("profile_done; Enter to exit");
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    Ok(())
}
fn flush_stack_log() {
    let blocks: Vec<_> = (0..8192)
        .map(|i| Vec::<u8>::with_capacity(32 + i % 16))
        .collect();
    black_box(blocks);
}
