// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Oblique cut through an asymmetric smooth loft, with separated capped halves
//! and the section outline. Run `cargo run -p constructive_probe --bin plane_cut
//! -- target/plane-cut.glb`.

use exedra_assembly::{Assembly, PartCompiler};
use exedra_constructive::ir::{CapMode, LoftPolicy, Placement3, Plane3};
use exedra_constructive::profile::{Loop2, Profile2, Seg2};
use exedra_constructive::section::{CutCap, SectionPolicy, split_body};
use exedra_constructive::tessellate::{
    EvalPolicy, TessellatedBody, tessellate_loft, tessellate_sweep,
};
use exedra_gltf::{GltfExportOptions, export_glb_with_options};
use exedra_mesh::{FaceTriangulation, Mesh};
use std::path::PathBuf;

const CAP_REGION: u32 = 1000;

fn add(assembly: &mut Assembly, name: &str, mesh: Mesh, offset: [f64; 3], material: &str) {
    let part = assembly
        .add_baked_part(name, mesh, &["surface", "cut"])
        .expect("part");
    assembly.set_default_slot(part, "surface").expect("slot");
    assembly
        .bind_region_slot(part, CAP_REGION, "cut")
        .expect("cap region");
    assembly
        .set_part_material(part, "surface", material)
        .expect("surface material");
    assembly
        .set_part_material(part, "cut", "cut.gold")
        .expect("cap material");
    assembly
        .add_instance(
            None,
            name,
            part,
            Placement3::translate(offset[0], offset[1], offset[2]),
        )
        .expect("instance");
}
fn volume(body: &TessellatedBody) -> f64 {
    let mut triangles = Vec::new();
    let mut volume = 0.0;
    for face in body.mesh.faces() {
        assert!(
            !body
                .mesh
                .face_triangles_into(face, FaceTriangulation::Robust, &mut triangles),
            "volume needs robust triangulation"
        );
        for triangle in &triangles {
            let [a, b, c] = triangle.map(|corner| {
                body.mesh
                    .vertex_position(body.mesh.to_vertex(corner).expect("vertex"))
                    .expect("position")
                    .map(f64::from)
            });
            volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
        }
    }
    volume
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/plane-cut.glb"));
    let profiles: Vec<_> = [(1.0, 1.0), (1.5, 0.8), (0.7, 1.2)]
        .into_iter()
        .map(|(x, y)| {
            Profile2::simple(
                Loop2::new(vec![
                    Seg2::line((-0.5 * x, -0.3 * y)),
                    Seg2::line((0.4 * x, -0.35 * y)),
                    Seg2::line((0.65 * x, 0.05 * y)),
                    Seg2::line((0.15 * x, 0.4 * y)),
                    Seg2::line((-0.45 * x, 0.2 * y)),
                ])
                .expect("loop"),
            )
            .expect("profile")
        })
        .collect();
    let sections = [
        (Placement3::IDENTITY, &profiles[0]),
        (Placement3::translate(0.3, 0.1, 1.5), &profiles[1]),
        (Placement3::translate(-0.1, 0.2, 3.0), &profiles[2]),
    ];
    let mut policy = EvalPolicy::default();
    policy.loft.chord_tolerance = 0.001;
    let source = tessellate_loft(&sections, LoftPolicy::Smooth, CapMode::Both, &policy)?;
    let plane = Plane3 {
        normal: [0.55, -0.2, 1.0],
        distance: 1.51,
    };
    let split = split_body(
        &source,
        plane,
        &SectionPolicy::default(),
        CutCap {
            region: CAP_REGION,
            material: None,
        },
    )?;
    let negative = split.negative.expect("negative half");
    let positive = split.positive.expect("positive half");
    let original = volume(&source);
    let halves = volume(&negative) + volume(&positive);
    assert!(
        (halves - original).abs() < 1e-5,
        "cut halves must conserve volume"
    );
    for body in [&negative, &positive] {
        assert!(body.mesh.validate_deep().is_empty(), "valid half topology");
        assert!(body.mesh.boundary_loops()?.is_empty(), "closed cut caps");
    }
    println!(
        "volume: source={original:.9}, halves={halves:.9}, error={:.3e}",
        (halves - original).abs()
    );
    println!(
        "section: {} regions, {} vertices, {} cap triangles, max plane deviation {:.3e}",
        split.section.regions.len(),
        split.section.stats.section_vertices,
        split.section.stats.cap_triangles,
        split.section.stats.max_plane_deviation
    );
    let mut assembly = Assembly::new();
    add(
        &mut assembly,
        "original",
        source.mesh,
        [-2.3, 0.0, 0.0],
        "surface.blue",
    );
    add(
        &mut assembly,
        "negative",
        negative.mesh,
        [-0.15, 0.0, -0.15],
        "surface.blue",
    );
    add(
        &mut assembly,
        "positive",
        positive.mesh,
        [0.25, 0.0, 0.75],
        "surface.blue",
    );
    let frame = split.section.frame.rows;
    let ring_profile = exedra_constructive::builders::circle(0.012)?;
    let mut edge_index = 0;
    for region in &split.section.regions {
        for boundary in std::iter::once(&region.outer).chain(&region.holes) {
            for i in 0..boundary.points.len() {
                let p = [
                    boundary.points[i],
                    boundary.points[(i + 1) % boundary.points.len()],
                ];
                let path = p.map(|p| {
                    std::array::from_fn(|axis| {
                        frame[axis][0] * p[0] + frame[axis][1] * p[1] + frame[axis][3]
                    })
                });
                let outline = tessellate_sweep(
                    &ring_profile,
                    &Placement3::IDENTITY,
                    &path,
                    CapMode::Both,
                    &policy,
                )?;
                add(
                    &mut assembly,
                    &format!("section-{edge_index}"),
                    outline.mesh.clone(),
                    [2.4, 0.0, 0.0],
                    "cut.gold",
                );
                add(
                    &mut assembly,
                    &format!("cut-line-{edge_index}"),
                    outline.mesh,
                    [-2.3, 0.0, 0.0],
                    "cut.gold",
                );
                edge_index += 1;
            }
        }
    }
    let compiled = PartCompiler::new().compile_parts(&assembly, &policy.into())?;
    let glb = export_glb_with_options(&assembly, &compiled, GltfExportOptions::z_up_to_y_up())?;
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, glb.bytes)?;
    println!("{}", output.display());
    Ok(())
}
