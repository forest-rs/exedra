// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Throughput of UV authoring on an imported quad grid, excluding mesh creation.

use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

use exedra_mesh_ops::uv::{
    BoxUv, PerspectiveUv, UvAuthoringScratch, UvFrame, UvMapping, UvMappingOptions,
    UvMappingParams, UvProjection, UvScope, UvTransform, map_uvs, map_uvs_with_scratch,
};
use exedra_testkit::fixtures::grid_mesh;

fn main() -> Result<(), Box<dyn Error>> {
    if let Some(mode) = std::env::var_os("EXEDRA_UV_PROFILE") {
        return allocation_profile(mode == "warm");
    }
    let side: u32 = std::env::args().nth(1).map_or(Ok(192), |x| x.parse())?;
    let repetitions: u32 = std::env::args().nth(2).map_or(Ok(9), |x| x.parse())?;
    if side < 8 || repetitions == 0 {
        return Err("grid size must be at least 8 and repetitions must be positive".into());
    }
    for (name, projection) in [
        ("planar", UvProjection::Planar(UvFrame::XY)),
        ("box", UvProjection::Box(BoxUv::default())),
        ("existing", UvProjection::Existing),
        ("face", UvProjection::Face),
        ("cylindrical", UvProjection::Cylindrical(UvFrame::XY)),
        ("spherical", UvProjection::Spherical(UvFrame::XY)),
        (
            "toroidal",
            UvProjection::Toroidal {
                frame: UvFrame::XY,
                major_radius: 2.0,
            },
        ),
        (
            "perspective",
            UvProjection::Perspective(PerspectiveUv {
                frame: UvFrame {
                    origin: [0.0, 0.0, -2.0],
                    ..UvFrame::XY
                },
                focal: [1.0; 2],
                near: 0.01,
            }),
        ),
    ] {
        for reuse in [false, true] {
            let mut mesh = if matches!(name, "cylindrical" | "spherical" | "toroidal") {
                ring_mesh(side)
            } else {
                grid_mesh(side, side)
            };
            let mut parameters = UvMappingParams {
                scope: UvScope::WholeMesh,
                mapping: UvMapping {
                    projection: UvProjection::Planar(UvFrame::XY),
                    transform: UvTransform::IDENTITY,
                },
                options: UvMappingOptions::default(),
                write_missing_only: false,
            };
            let mut edit = mesh.edit();
            map_uvs(&mut edit, &parameters)?;
            let _: () = edit.finish();
            parameters.mapping.projection = projection;
            // Compare both public execution paths before timing either one.
            let mut direct = mesh.clone();
            let mut retained = mesh.clone();
            let mut scratch = UvAuthoringScratch::default();
            let mut edit = direct.edit();
            map_uvs(&mut edit, &parameters)?;
            let _: () = edit.finish();
            let mut edit = retained.edit();
            map_uvs_with_scratch(&mut edit, &parameters, &mut scratch)?;
            let _: () = edit.finish();
            assert_eq!(
                signature(&direct),
                signature(&retained),
                "fresh/reused scratch must agree for {name}"
            );
            assert!(
                direct.validate_deep().is_empty(),
                "mapped mesh must remain valid"
            );
            let mut durations = Vec::new();
            let mut corners = 0;
            for iteration in 0..repetitions + 1 {
                // Alternate phase to measure real writes on every timed iteration.
                parameters.mapping.transform.offset = [f64::from(iteration % 2), 0.0];
                if name == "existing" {
                    parameters.mapping.transform.matrix = [[0.0, -1.0], [1.0, 0.0]];
                }
                let start = Instant::now();
                let mut edit = mesh.edit();
                let output = if reuse {
                    map_uvs_with_scratch(&mut edit, black_box(&parameters), &mut scratch)?
                } else {
                    map_uvs(&mut edit, black_box(&parameters))?
                };
                let _: () = edit.finish();
                let elapsed = start.elapsed().as_secs_f64();
                corners = black_box(output.corners_written);
                if iteration > 0 {
                    durations.push(elapsed);
                }
            }
            durations.sort_by(f64::total_cmp);
            let median = durations[durations.len() / 2];
            println!(
                "{name}: scratch={} faces={} corners={corners} median_ms={:.3} million_corners_per_second={:.3} repetitions={repetitions} retained_bytes={}",
                if reuse { "reused" } else { "fresh" },
                mesh.faces().count(),
                median * 1000.0,
                corners as f64 / median / 1e6,
                scratch.retained_bytes(),
            );
        }
    }
    selected_face_benchmark(side, repetitions)?;
    quality_benchmark(side, repetitions)?;
    Ok(())
}

fn selected_face_benchmark(side: u32, repetitions: u32) -> Result<(), Box<dyn Error>> {
    let mut mesh = grid_mesh(side, side);
    let mut params = UvMappingParams {
        scope: UvScope::WholeMesh,
        mapping: UvMapping {
            projection: UvProjection::Planar(UvFrame::XY),
            transform: UvTransform::IDENTITY,
        },
        options: UvMappingOptions::default(),
        write_missing_only: false,
    };
    let mut edit = mesh.edit();
    map_uvs(&mut edit, &params)?;
    let _: () = edit.finish();
    // Last face forces a full-layer scan to show up if incremental updates regress.
    params.scope = UvScope::FaceSet(vec![mesh.faces().last().expect("nonempty grid")]);
    let mut scratch = UvAuthoringScratch::default();
    let mut durations = Vec::new();
    for repetition in 0..repetitions + 1 {
        params.mapping.transform.offset = [f64::from(repetition % 2) + 0.25, 0.0];
        let start = Instant::now();
        for _ in 0..100 {
            params.mapping.transform.offset[0] += 0.125;
            let mut edit = mesh.edit();
            let output = map_uvs_with_scratch(&mut edit, black_box(&params), &mut scratch)?;
            let _: () = edit.finish();
            assert_eq!(
                black_box(output.corners_written),
                4,
                "every selected quad corner must change"
            );
        }
        if repetition > 0 {
            durations.push(start.elapsed().as_secs_f64() / 100.0);
        }
    }
    durations.sort_by(f64::total_cmp);
    println!(
        "selected_last_face: mesh_faces={} corners=4 median_us={:.3} repetitions={repetitions}",
        mesh.faces().count(),
        durations[durations.len() / 2] * 1e6
    );
    Ok(())
}

fn quality_benchmark(side: u32, repetitions: u32) -> Result<(), Box<dyn Error>> {
    use exedra_mesh_ops::uv::quality::{UvQualityParams, UvQualityScratch, inspect_uv_quality};
    let mut mesh = grid_mesh(side, side);
    let mut edit = mesh.edit();
    map_uvs(
        &mut edit,
        &UvMappingParams {
            scope: UvScope::WholeMesh,
            mapping: UvMapping {
                projection: UvProjection::Planar(UvFrame::XY),
                transform: UvTransform::IDENTITY,
            },
            options: UvMappingOptions::default(),
            write_missing_only: false,
        },
    )?;
    let _: () = edit.finish();
    for (name, scope, triangulation) in [
        (
            "whole_fan",
            UvScope::WholeMesh,
            exedra_mesh::FaceTriangulation::Fan,
        ),
        (
            "last_face_fan",
            UvScope::FaceSet(vec![mesh.faces().last().expect("grid face")]),
            exedra_mesh::FaceTriangulation::Fan,
        ),
        (
            "whole_robust",
            UvScope::WholeMesh,
            exedra_mesh::FaceTriangulation::Robust,
        ),
        (
            "last_face_robust",
            UvScope::FaceSet(vec![mesh.faces().last().expect("grid face")]),
            exedra_mesh::FaceTriangulation::Robust,
        ),
    ] {
        let params = UvQualityParams {
            scope,
            triangulation,
            texture_size: [1024; 2],
            ..UvQualityParams::default()
        };
        let mut scratch = UvQualityScratch::default();
        let expected = inspect_uv_quality(&mesh, &params, &mut scratch)?;
        assert!(
            expected.is_acceptable(),
            "planar quality must be measurable and within limits"
        );
        assert_eq!(
            inspect_uv_quality(&mesh, &params, &mut scratch)?,
            expected,
            "quality reports must be deterministic"
        );
        let mut durations = Vec::new();
        let batch = if name.starts_with("last_face") {
            100
        } else {
            1
        };
        for repetition in 0..repetitions + 1 {
            let start = Instant::now();
            for _ in 0..batch {
                let report = inspect_uv_quality(&mesh, black_box(&params), &mut scratch)?;
                black_box(report);
            }
            if repetition > 0 {
                durations.push(start.elapsed().as_secs_f64() / f64::from(batch));
            }
        }
        durations.sort_by(f64::total_cmp);
        let median = durations[durations.len() / 2];
        println!(
            "uv_quality: scope={name} mesh_faces={} triangles={} median_us={:.3} million_triangles_per_second={:.3} retained_bytes={}",
            mesh.faces().count(),
            expected.triangles_examined,
            median * 1e6,
            expected.triangles_examined as f64 / median / 1e6,
            scratch.retained_bytes()
        );
    }
    Ok(())
}

fn allocation_profile(warm: bool) -> Result<(), Box<dyn Error>> {
    use std::io::{self, Write};
    let mut mesh = grid_mesh(128, 128);
    let mut params = UvMappingParams {
        scope: UvScope::WholeMesh,
        mapping: UvMapping {
            projection: UvProjection::Box(BoxUv::default()),
            transform: UvTransform::IDENTITY,
        },
        options: UvMappingOptions::default(),
        write_missing_only: false,
    };
    let mut scratch = UvAuthoringScratch::default();
    let mut edit = mesh.edit();
    map_uvs(&mut edit, &params)?;
    let _: () = edit.finish();
    if warm {
        params.mapping.transform.offset = [1.0; 2];
        let mut edit = mesh.edit();
        map_uvs_with_scratch(&mut edit, &params, &mut scratch)?;
        let _: () = edit.finish();
    }
    flush_stack_log();
    println!(
        "profile_ready pid={} warm={warm}; press Enter to map three times",
        std::process::id()
    );
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    for iteration in 0..3 {
        params.mapping.transform.offset = [10.0 + f64::from(iteration); 2];
        let mut edit = mesh.edit();
        black_box(map_uvs_with_scratch(&mut edit, &params, &mut scratch)?);
        let _: () = edit.finish();
    }
    flush_stack_log();
    println!(
        "profile_done retained_bytes={}; press Enter to exit",
        scratch.retained_bytes()
    );
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    Ok(())
}

// Full macOS stack logging buffers events. These allocations are outside the
// measured mapping batch and have their own stack, so before/after histories
// include even a tiny warm batch without attributing the flush to UV authoring.
fn flush_stack_log() {
    let blocks: Vec<_> = (0..8192)
        .map(|index| Vec::<u8>::with_capacity(32 + index % 16))
        .collect();
    black_box(blocks);
}

fn signature(mesh: &exedra_mesh::Mesh) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for face in mesh.faces() {
        for corner in mesh.face_loop(face) {
            for value in
                exedra_mesh_ops::uv::corner_uv(mesh, exedra_mesh_ops::uv::UvSet::Primary, corner)
                    .expect("complete authored UVs")
            {
                for byte in value.to_bits().to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
                }
            }
        }
    }
    hash
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "bounded benchmark coordinates narrow to mesh storage"
)]
fn ring_mesh(side: u32) -> exedra_mesh::Mesh {
    let rows = side / 2;
    let mut builder = exedra_mesh::MeshBuilder::new();
    for row in 0..rows {
        for col in 0..side {
            let u = f64::from(col) * std::f64::consts::TAU / f64::from(side);
            let v = f64::from(row) * std::f64::consts::TAU / f64::from(rows);
            let radius = 2.0 + 0.5 * v.cos();
            builder.push_vertex([
                (radius * u.cos()) as f32,
                (radius * u.sin()) as f32,
                (0.5 * v.sin()) as f32,
            ]);
        }
    }
    for row in 0..rows {
        for col in 0..side {
            builder
                .add_face(&[
                    row * side + col,
                    row * side + (col + 1) % side,
                    ((row + 1) % rows) * side + (col + 1) % side,
                    ((row + 1) % rows) * side + col,
                ])
                .unwrap();
        }
    }
    builder.build().unwrap().mesh
}
