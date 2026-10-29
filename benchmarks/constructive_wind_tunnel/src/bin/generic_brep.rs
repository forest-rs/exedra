// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Construction-independent retained trim validation and realization.
//! `cargo run --release -p constructive_wind_tunnel --bin generic_brep -- 7`
//! Source tables and validation/quality inspection are outside timing windows.

use exedra_analytic::body::{
    Body, BodyParts, BoundaryLoop, Coedge, Edge, EdgeId, Face, LoopId, LoopTrims, Sense, TrimCurve,
    ValidationPolicy, VertexId,
};
use exedra_analytic::geometry::{Curve, ParameterDomain, ParameterRange, Surface};
use exedra_analytic::realize::RealizationPolicy;
use exedra_constructive::ir::Placement3;
use serde_json::json;
use std::{error::Error, hint::black_box, time::Instant};

fn coedge(edge: u32, reversed: bool) -> Coedge {
    Coedge {
        edge: EdgeId(edge),
        reversed,
    }
}
fn line(start: [f64; 2], end: [f64; 2]) -> TrimCurve {
    TrimCurve::Line { start, end }
}
fn cylinder(explicit: bool) -> Body {
    let full = ParameterRange::new(0.0, core::f64::consts::TAU).unwrap();
    let tau = core::f64::consts::TAU;
    let domain = ParameterDomain {
        u: ParameterRange::new(-2.0, 2.0).unwrap(),
        v: ParameterRange::new(-2.0, 2.0).unwrap(),
    };
    let cap_trim = || TrimCurve::Harmonic {
        origin: [0.0; 2],
        linear: [0.0; 2],
        cosine: [2.0, 0.0],
        sine: [0.0, 2.0],
        range: full,
    };
    let parts = BodyParts {
        vertices: vec![[2.0, 0.0, 0.0], [2.0, 0.0, 3.0]],
        edges: vec![
            Edge::new(
                Curve::circle(Placement3::IDENTITY, 2.0).unwrap(),
                full,
                VertexId(0),
                VertexId(0),
                (),
            ),
            Edge::new(
                Curve::circle(Placement3::translate(0.0, 0.0, 3.0), 2.0).unwrap(),
                full,
                VertexId(1),
                VertexId(1),
                (),
            ),
            Edge::new(
                Curve::line([2.0, 0.0, 0.0], [0.0, 0.0, 3.0]).unwrap(),
                ParameterRange::UNIT,
                VertexId(0),
                VertexId(1),
                (),
            ),
        ],
        loops: vec![
            BoundaryLoop {
                coedges: vec![
                    coedge(0, false),
                    coedge(2, false),
                    coedge(1, true),
                    coedge(2, true),
                ],
            },
            BoundaryLoop {
                coedges: vec![coedge(0, true)],
            },
            BoundaryLoop {
                coedges: vec![coedge(1, false)],
            },
        ],
        faces: vec![
            Face {
                surface: Surface::cylinder(Placement3::IDENTITY, 2.0).unwrap(),
                domain: ParameterDomain {
                    u: full,
                    v: ParameterRange::new(0.0, 3.0).unwrap(),
                },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::IDENTITY).unwrap(),
                domain,
                sense: Sense::Reverse,
                loops: vec![LoopId(1)],
                role: (),
            },
            Face {
                surface: Surface::plane(Placement3::translate(0.0, 0.0, 3.0)).unwrap(),
                domain,
                sense: Sense::Forward,
                loops: vec![LoopId(2)],
                role: (),
            },
        ],
        trims: if explicit {
            vec![
                LoopTrims {
                    loop_id: LoopId(0),
                    curves: vec![
                        line([0.0, 0.0], [tau, 0.0]),
                        line([tau, 0.0], [tau, 3.0]),
                        line([0.0, 3.0], [tau, 3.0]),
                        line([0.0, 0.0], [0.0, 3.0]),
                    ],
                },
                LoopTrims {
                    loop_id: LoopId(1),
                    curves: vec![cap_trim()],
                },
                LoopTrims {
                    loop_id: LoopId(2),
                    curves: vec![cap_trim()],
                },
            ]
        } else {
            vec![]
        },
    };
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}
fn split_cylinder(segments: u32) -> Body {
    let mut parts = cylinder(true).into_parts();
    let seam = parts.edges[2].regular().unwrap().0.clone();
    parts.edges.truncate(2);
    let tau = core::f64::consts::TAU;
    let mut right = Vec::new();
    let mut left = Vec::new();
    for j in 0..segments {
        let a = f64::from(j) / f64::from(segments);
        let b = f64::from(j + 1) / f64::from(segments);
        let from = if j == 0 { VertexId(0) } else { VertexId(j + 1) };
        let to = if j + 1 == segments {
            VertexId(1)
        } else {
            let id = VertexId(u32::try_from(parts.vertices.len()).unwrap());
            parts.vertices.push([2.0, 0.0, 3.0 * b]);
            id
        };
        parts.edges.push(Edge::new(
            seam.clone(),
            ParameterRange::new(a, b).unwrap(),
            from,
            to,
            (),
        ));
        right.push(line([tau, 3.0 * a], [tau, 3.0 * b]));
        left.push(line([0.0, 3.0 * a], [0.0, 3.0 * b]));
    }
    let mut coedges = vec![coedge(0, false)];
    coedges.extend((0..segments).map(|i| coedge(i + 2, false)));
    coedges.push(coedge(1, true));
    coedges.extend((0..segments).rev().map(|i| coedge(i + 2, true)));
    parts.loops[0].coedges = coedges;
    let mut curves = vec![line([0.0, 0.0], [tau, 0.0])];
    curves.extend(right);
    curves.push(line([0.0, 3.0], [tau, 3.0]));
    curves.extend(left.into_iter().rev());
    parts.trims[0].curves = curves;
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}

fn sphere_patch() -> Body {
    let surface = Surface::sphere(Placement3::IDENTITY, 2.0).unwrap();
    let (u, v) = (
        ParameterRange::new(0.2, 1.0).unwrap(),
        ParameterRange::new(-0.3, 0.4).unwrap(),
    );
    let corners = [[0.2, -0.3], [1.0, -0.3], [1.0, 0.4], [0.2, 0.4]];
    let vertices = corners
        .map(|uv| surface.evaluate(uv).unwrap().point)
        .to_vec();
    let latitude = |lat: f64| {
        Curve::circle(
            Placement3::translate(0.0, 0.0, 2.0 * lat.sin()),
            2.0 * lat.cos(),
        )
        .unwrap()
    };
    let meridian = |angle: f64| {
        Curve::circle(
            Placement3::from_axes(
                [angle.cos(), angle.sin(), 0.0],
                [0.0, 0.0, 1.0],
                [angle.sin(), -angle.cos(), 0.0],
                [0.0; 3],
            ),
            2.0,
        )
        .unwrap()
    };
    Body::try_from_parts(
        BodyParts {
            vertices,
            edges: vec![
                Edge::new(latitude(-0.3), u, VertexId(0), VertexId(1), ()),
                Edge::new(meridian(1.0), v, VertexId(1), VertexId(2), ()),
                Edge::new(latitude(0.4), u, VertexId(3), VertexId(2), ()),
                Edge::new(meridian(0.2), v, VertexId(0), VertexId(3), ()),
            ],
            loops: vec![BoundaryLoop {
                coedges: vec![
                    coedge(0, false),
                    coedge(1, false),
                    coedge(2, true),
                    coedge(3, true),
                ],
            }],
            faces: vec![Face {
                surface,
                domain: ParameterDomain { u, v },
                sense: Sense::Forward,
                loops: vec![LoopId(0)],
                role: (),
            }],
            trims: vec![LoopTrims {
                loop_id: LoopId(0),
                curves: vec![
                    line(corners[0], corners[1]),
                    line(corners[1], corners[2]),
                    line(corners[3], corners[2]),
                    line(corners[0], corners[3]),
                ],
            }],
        },
        &ValidationPolicy { max_gap: 1e-9 },
    )
    .unwrap()
}
fn fixture(mode: &str) -> Body {
    match mode {
        "support_cylinder" => cylinder(false),
        "explicit_cylinder" => cylinder(true),
        "split_cylinder" => split_cylinder(2),
        "four_split_cylinder" => split_cylinder(4),
        "sphere_patch" => sphere_patch(),
        _ => panic!("unknown generic BRep fixture"),
    }
}

/// One fixed-source realization for the isolated external allocation profiler.
/// Source geometry and checkpoint machinery are outside the measured window.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let mode = std::env::var("EXEDRA_BREP_PROFILE").unwrap_or_else(|_| "sphere_patch".into());
    let body = fixture(&mode);
    let policy = RealizationPolicy {
        tolerance: 0.025,
        ..Default::default()
    };
    checkpoint("ready");
    let output = black_box(body.realize(Placement3::IDENTITY, &policy).unwrap());
    checkpoint("retained");
    drop(output);
    checkpoint("released");
}
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    values[values.len() / 2]
}
fn main() -> Result<(), Box<dyn Error>> {
    let reps: usize = std::env::args().nth(1).map_or(Ok(7), |s| s.parse())?;
    if reps == 0 {
        return Err("repetitions must be positive".into());
    }
    let mut rows = Vec::new();
    for mode in [
        "support_cylinder",
        "explicit_cylinder",
        "split_cylinder",
        "four_split_cylinder",
        "sphere_patch",
    ] {
        let body = fixture(mode);
        let validation = body.validate(&ValidationPolicy { max_gap: 1e-9 })?;
        for tolerance in [0.1, 0.025] {
            let policy = RealizationPolicy {
                tolerance,
                ..Default::default()
            };
            let output = body.realize(Placement3::IDENTITY, &policy)?;
            assert!(
                output.report.within_tolerance(),
                "fixture must meet its requested geometric deviation"
            );
            assert!(
                output.mesh.validate_deep().is_empty(),
                "realized topology must validate"
            );
            assert_eq!(
                output.mesh.boundary_loops()?.len(),
                usize::from(mode == "sphere_patch"),
                "closed fixtures stay closed and the sphere patch keeps one boundary"
            );
            let mut validate = Vec::new();
            let mut realize = Vec::new();
            for i in 0..reps + 2 {
                let start = Instant::now();
                black_box(body.validate(black_box(&ValidationPolicy { max_gap: 1e-9 }))?);
                if i >= 2 {
                    validate.push(start.elapsed().as_secs_f64() * 1e6);
                }
                let start = Instant::now();
                let candidate = black_box(body.realize(Placement3::IDENTITY, black_box(&policy))?);
                if i >= 2 {
                    realize.push(start.elapsed().as_secs_f64() * 1e6);
                }
                drop(candidate);
            }
            let normal_values = output
                .mesh
                .attrs()
                .sparse(exedra_mesh::attr::CORNER_NORMAL_OVERRIDE)
                .map_or(0, |layer| layer.entries().len());
            rows.push(json!({"fixture":mode,"tolerance":tolerance,"native_faces":body.faces().len(),"native_edges":body.edges().len(),"trimmed_coedges":validation.trimmed_coedges,"max_trim_gap":validation.max_trim_gap,"vertices":output.mesh_vertices.len(),"triangles":output.mesh_faces.len(),"max_deviation":output.report.max_deviation,"refinement_batches":output.report.work.trim_refinement_batches,"triangle_error_tests":output.report.work.trim_triangle_tests,"generated_vertices":output.report.work.trim_generated_vertices,"analytic_normal_corners":output.report.work.trim_normal_corners,"normal_scratch_capacity_bytes":output.report.work.trim_normal_scratch_bytes,"retained_normal_payload_bytes":normal_values*size_of::<(u32,[f32;3])>(),"validate_us_p50":median(validate),"realize_us_p50":median(realize)}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"scope":"native fixed-source validation and realization; source construction, quality inspection, output destruction excluded; no compile/STEP/JS/GPU","repetitions":reps,"rows":rows})
        )?
    );
    Ok(())
}
