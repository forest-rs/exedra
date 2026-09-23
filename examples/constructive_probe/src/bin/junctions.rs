// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tube junctions for visual review.
//!
//! Run `cargo run -p constructive_probe --bin junctions -- target/junctions`.
//! Each configuration joins open tubes with `exedra_mesh_ops::junction`, caps
//! the outer ends, checks the result is a valid closed shell, and writes an
//! OBJ whose `tube` and `junction` groups separate the input tubes from the
//! skin. `tools/render_junctions.py` renders them with Blender.

use std::f64::consts::TAU;
use std::fmt::Write as _;

use exedra_mesh::{FaceId, HalfEdgeId, Mesh, MeshBuilder, attr, op};
use exedra_mesh_ops::junction::{JunctionParams, add_junction};

/// One open tube: `count` vertices per ring, starting `start` from the
/// junction along `direction`.
struct Tube {
    direction: [f64; 3],
    start: f64,
    length: f64,
    radius: f64,
    count: usize,
}

fn tube(direction: [f64; 3], radius: f64, count: usize) -> Tube {
    Tube {
        direction,
        start: radius * 1.6 + 0.15,
        length: 1.2,
        radius,
        count,
    }
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / length)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn spherical(azimuth_degrees: f64, polar_degrees: f64) -> [f64; 3] {
    let (a, p) = (azimuth_degrees.to_radians(), polar_degrees.to_radians());
    [p.sin() * a.cos(), p.sin() * a.sin(), p.cos()]
}

/// Builds outward-oriented open tubes; returns one inner boundary half-edge
/// per tube.
fn tubes(specs: &[Tube]) -> (Mesh, Vec<HalfEdgeId>) {
    let mut builder = MeshBuilder::new();
    let mut inner_rings = Vec::new();
    for tube in specs {
        let d = normalize(tube.direction);
        let helper = if d[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let e1 = normalize(cross(cross(d, helper), d));
        let e2 = cross(d, e1);
        let mut ring = |distance: f64| {
            (0..tube.count)
                .map(|k| {
                    let angle = TAU * k as f64 / tube.count as f64;
                    let (s, c) = angle.sin_cos();
                    let p: [f64; 3] = std::array::from_fn(|i| {
                        d[i] * distance + tube.radius * (e1[i] * c + e2[i] * s)
                    });
                    #[expect(clippy::cast_possible_truncation, reason = "mesh positions are f32")]
                    let p = p.map(|v| v as f32);
                    builder.push_vertex(p)
                })
                .collect::<Vec<_>>()
        };
        let inner = ring(tube.start);
        let outer = ring(tube.start + tube.length);
        for k in 0..tube.count {
            let k1 = (k + 1) % tube.count;
            builder
                .add_face(&[inner[k], inner[k1], outer[k1], outer[k]])
                .expect("tube face");
        }
        inner_rings.push((inner[1], inner[0]));
    }
    let mesh = builder.build().expect("tubes build").mesh;
    let seeds = inner_rings
        .iter()
        .map(|&(a, b)| {
            mesh.half_edges()
                .find(|&h| {
                    mesh.face(h) == Some(FaceId::OUTSIDE)
                        && mesh.from_vertex(h).map(|v| v.index()) == Some(a)
                        && mesh.to_vertex(h).map(|v| v.index()) == Some(b)
                })
                .expect("inner boundary half-edge")
        })
        .collect();
    (mesh, seeds)
}

fn build(specs: &[Tube]) -> Result<Mesh, Box<dyn std::error::Error>> {
    let (mut mesh, seeds) = tubes(specs);
    let mut edit = mesh.edit();
    let output = add_junction(
        &mut edit,
        &JunctionParams {
            center: [0.0; 3],
            rings: seeds,
            chart: None,
            region: Some(1),
        },
    )?;
    let _: () = edit.finish();
    let loops = mesh.boundary_loops()?;
    let mut edit = mesh.edit();
    for boundary in &loops {
        let vertices = boundary
            .iter()
            .map(|&h| edit.mesh().from_vertex(h).expect("vertex"))
            .collect::<Vec<_>>();
        op::add_face(&mut edit, &vertices)?;
    }
    let _: () = edit.finish();
    assert!(mesh.boundary_loops()?.is_empty(), "closed shell");
    assert!(mesh.validate_deep().is_empty(), "valid topology");
    println!(
        "  {} skin faces ({} bridge quads, {} bridge triangles, {} crotch quads), \
         {} crotches, virtual arm: {}",
        output.faces.len(),
        output.stats.bridge_quads,
        output.stats.bridge_triangles,
        output.stats.crotch_quads,
        output.stats.crotches,
        output.stats.virtual_arm,
    );
    Ok(mesh)
}

fn write_obj(mesh: &Mesh, path: &std::path::Path) -> std::io::Result<()> {
    let mut text = String::new();
    let mut index = std::collections::HashMap::new();
    for (k, vertex) in mesh.vertices().enumerate() {
        let p = mesh.vertex_position(vertex).expect("position");
        let _ = writeln!(text, "v {} {} {}", p[0], p[1], p[2]);
        index.insert(vertex, k + 1);
    }
    let regions = mesh.attrs().dense(attr::FACE_REGION).expect("regions");
    for (group, region) in [("tube", 0), ("junction", 1)] {
        let _ = writeln!(text, "g {group}");
        for face in mesh.faces() {
            if regions.get(face.into()) != Some(&region) {
                continue;
            }
            text.push('f');
            for corner in mesh.face_loop(face) {
                let vertex = mesh.to_vertex(corner).expect("vertex");
                let _ = write!(text, " {}", index[&vertex]);
            }
            text.push('\n');
        }
    }
    std::fs::write(path, text)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/junctions".into()),
    );
    std::fs::create_dir_all(&out)?;
    let cases: Vec<(&str, Vec<Tube>)> = vec![
        (
            "y-fork",
            vec![
                tube([0.0, 0.0, -1.0], 0.3, 16),
                tube(spherical(0.0, 32.0), 0.22, 12),
                tube(spherical(180.0, 38.0), 0.2, 12),
            ],
        ),
        (
            "t-junction",
            vec![
                tube([-1.0, 0.0, 0.0], 0.25, 16),
                tube([1.0, 0.0, 0.0], 0.25, 16),
                tube([0.0, 0.0, 1.0], 0.18, 12),
            ],
        ),
        (
            "four-way",
            vec![
                tube([0.0, 0.0, -1.0], 0.3, 20),
                tube(spherical(10.0, 62.0), 0.17, 12),
                tube(spherical(130.0, 70.0), 0.15, 12),
                tube(spherical(250.0, 65.0), 0.16, 12),
                tube([0.0, 0.0, 1.0], 0.2, 12),
            ],
        ),
        (
            "roots",
            vec![
                tube(spherical(0.0, 120.0), 0.14, 12),
                tube(spherical(100.0, 130.0), 0.15, 12),
                tube(spherical(190.0, 115.0), 0.13, 12),
                tube(spherical(275.0, 125.0), 0.14, 12),
            ],
        ),
    ];
    for (name, specs) in &cases {
        println!("{name}:");
        let mesh = build(specs)?;
        write_obj(&mesh, &out.join(format!("{name}.obj")))?;
    }
    println!("wrote {}", out.display());
    Ok(())
}
