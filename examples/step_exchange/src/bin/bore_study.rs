// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Public native bore capability and construction/realization/exchange workload.

use exedra_analytic::{
    bore::{Bore, BorePolicy, BoredPlate},
    extrusion::Extrusion,
    realize::{RealizationPolicy, RealizedBody},
};
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_math::Placement3;
use exedra_step::{StepDocument, StepOptions, export_body};
use std::{hint::black_box, time::Instant};

fn source(size: f64, frame: Placement3) -> Extrusion {
    let outline = Loop::new(vec![
        Segment::line([35.0 * size, -25.0 * size]),
        Segment::line([35.0 * size, 25.0 * size]),
        Segment::line([-35.0 * size, 25.0 * size]),
        Segment::line([-35.0 * size, -25.0 * size]),
    ])
    .unwrap();
    Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-8 * size)).unwrap(),
        frame,
        20.0 * size,
    )
    .unwrap()
}
fn bore(degrees: f64, size: f64) -> Bore {
    let angle = degrees.to_radians();
    Bore::new(
        [-10.0 * size * angle.tan(), 0.0, 0.0],
        [angle.sin(), 0.0, angle.cos()],
        6.0 * size,
    )
    .unwrap()
}
fn policy(size: f64) -> BorePolicy {
    BorePolicy {
        separation: 0.1 * size,
        geometry_gap: 1e-8 * size,
        min_axis_dot: 0.1,
    }
}
fn run(source: &Extrusion, bore: Bore) -> (BoredPlate, RealizedBody, StepDocument, [u128; 3]) {
    let start = Instant::now();
    let model = source.with_through_bore(bore, &policy(1.0)).unwrap();
    let construction = start.elapsed().as_nanos();
    let start = Instant::now();
    let mesh = model
        .body()
        .realize(
            model.frame(),
            &RealizationPolicy {
                tolerance: 0.025,
                geometry_gap: 1e-8,
                ..Default::default()
            },
        )
        .unwrap();
    let realization = start.elapsed().as_nanos();
    let start = Instant::now();
    let step = export_body(model.body(), model.frame(), &StepOptions::default()).unwrap();
    let exchange = start.elapsed().as_nanos();
    black_box((&model, &mesh, &step));
    (model, mesh, step, [construction, realization, exchange])
}

/// Three actual model/mesh/exchange results, then native ownership and final release.
/// Original source, requests, container capacity and warmup precede the window.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let source = source(1.0, Placement3::IDENTITY);
    let bores = [bore(0.0, 1.0), bore(30.0, 1.0), bore(60.0, 1.0)];
    drop(run(&source, bores[1].clone()));
    let mut results = Vec::with_capacity(3);
    let mut models = Vec::with_capacity(3);
    checkpoint("ready");
    for bore in bores {
        results.push(run(&source, bore));
    }
    checkpoint("retained");
    for (model, mesh, step, _) in results.drain(..) {
        models.push(model);
        drop(mesh);
        drop(step);
    }
    black_box(models.as_slice());
    checkpoint("native_only");
    models.clear();
    checkpoint("released");
}

fn capability_matrix() -> serde_json::Value {
    let mut rows = Vec::new();
    let mut required = 0;
    let mut successes = 0;
    for size in [1e-6, 1.0, 1e6] {
        for angle in [0.0, 30.0, 60.0] {
            let source = source(size, Placement3::IDENTITY);
            required += 1;
            let model = source
                .with_through_bore(bore(angle, size), &policy(size))
                .unwrap();
            let mesh = model
                .body()
                .realize(
                    model.frame(),
                    &RealizationPolicy {
                        tolerance: 0.025 * size,
                        geometry_gap: 1e-8 * size,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(
                mesh.report.within_tolerance(),
                "required capability must meet accuracy"
            );
            assert!(
                mesh.mesh.validate_deep().is_empty(),
                "required capability must emit valid topology"
            );
            successes += 1;
            rows.push(serde_json::json!({"required":true,"scale":size,"angle_degrees":angle,"native":"success","display":"accuracy_met","deviation":mesh.report.max_deviation,"tolerance":mesh.report.tolerance}));
        }
    }
    for (label, size, translation) in [
        ("tiny f32 emission", 1e-80, 0.0),
        ("huge f32 emission", 1e80, 0.0),
        ("far occurrence", 1.0, 1e6 + 0.03),
        ("tight relative placement", 1e-6, 1000.0 * 1e-6),
    ] {
        let source = source(
            size,
            Placement3::rotate_y_then_translate(
                if label == "tight relative placement" {
                    0.35
                } else {
                    0.0
                },
                translation,
                if label == "tight relative placement" {
                    -700.0 * size
                } else {
                    0.0
                },
                if label == "tight relative placement" {
                    300.0 * size
                } else {
                    0.0
                },
            ),
        );
        let result = source.with_through_bore(bore(30.0, size), &policy(size));
        let row = match result {
            Err(error) => {
                serde_json::json!({"required":false,"label":label,"scale":size,"native":"refused","diagnostic":error.to_string()})
            }
            Ok(model) => match model.body().realize(
                model.frame(),
                &RealizationPolicy {
                    tolerance: 0.025 * size,
                    geometry_gap: 1e-8 * size,
                    ..Default::default()
                },
            ) {
                Err(error) => {
                    serde_json::json!({"required":false,"label":label,"scale":size,"native":"success","display":"refused","diagnostic":error.to_string()})
                }
                Ok(mesh) => {
                    let report = &mesh.report;
                    let other_error = report.max_vertex_displacement
                        + report.max_ruling_displacement
                        + 2.0 * report.geometry_gap;
                    let reason = if report.within_tolerance() {
                        "target_achieved"
                    } else if other_error < 0.25 * report.tolerance {
                        "curve_budget_with_small_other_error"
                    } else {
                        "emission_and_native_error_budget"
                    };
                    serde_json::json!({"required":false,"label":label,"scale":size,"native":"success","display":if report.within_tolerance(){"accuracy_met"}else{"accuracy_unmet"},"reason":reason,"deviation":report.max_deviation,"tolerance":report.tolerance,"curve_deviation":report.max_curve_deviation,"surface_deviation":report.max_surface_deviation,"vertex_displacement":report.max_vertex_displacement,"ruling_displacement":report.max_ruling_displacement,"native_gap":report.geometry_gap})
                }
            },
        };
        rows.push(row);
    }
    let counts: Vec<_> = [true, false].into_iter().map(|required| {
        let cases: Vec<_> = rows.iter().filter(|row| row["required"] == required).collect();
        serde_json::json!({"required":required,"cases":cases.len(),"supported_native":cases.iter().filter(|r| r["native"] == "success").count(),"native_refused":cases.iter().filter(|r| r["native"] == "refused").count(),"achieved_target":cases.iter().filter(|r| r["display"] == "accuracy_met").count(),"accuracy_unmet":cases.iter().filter(|r| r["display"] == "accuracy_unmet").count(),"display_refused":cases.iter().filter(|r| r["display"] == "refused").count()})
    }).collect();
    serde_json::json!({"required_cases":required,"required_successes":successes,"counts":counts,"cases":rows})
}
fn main() {
    let source = source(1.0, Placement3::IDENTITY);
    let mut rows = Vec::new();
    for angle in [0.0, 30.0, 60.0] {
        let bore = bore(angle, 1.0);
        for _ in 0..3 {
            drop(run(&source, bore.clone()));
        }
        let mut samples = Vec::new();
        let mut last = None;
        for _ in 0..101 {
            let output = run(&source, bore.clone());
            samples.push(output.3);
            last = Some((
                output.0.body().faces().len(),
                output.1.mesh_faces.len(),
                output.2.text.len(),
            ));
            drop(output);
        }
        let (faces, triangles, step_bytes) = last.unwrap();
        rows.push(serde_json::json!({"angle_degrees":angle,"native_faces":faces,"display_triangles":triangles,"step_bytes":step_bytes,"samples_ns":samples}));
    }
    println!(
        "{}",
        serde_json::json!({"timings":rows,"capabilities":capability_matrix(),"scope":"preauthored source/axis outside timing; native construction retains original Extrusion; mesh/exchange outputs held at stop then released; no compiler cache, JS, GPU or file I/O"})
    );
}
