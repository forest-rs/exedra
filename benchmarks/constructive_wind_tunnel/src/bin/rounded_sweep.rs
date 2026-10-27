// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Fresh meshing of a retained hollow round rail with two toroidal bends.

use exedra_analytic::{realize::RealizationPolicy, sweep::Sweep};
use exedra_constructive::ir::Placement3;
use exedra_constructive::{builders, path::PathCursor3};
use exedra_profile::ProfilePolicy;
use std::{hint::black_box, time::Instant};

fn main() {
    let profile = builders::circle(10.0)
        .unwrap()
        .with_cutouts_at(&builders::circle(6.0).unwrap(), [(0.0, 0.0)])
        .unwrap();
    let path = PathCursor3::new([0.0; 3], [0.0, 1.0, 0.0])
        .tangent_arc(40.0, core::f64::consts::PI, [0.0, 0.0, 1.0])
        .tangent_arc(40.0, -core::f64::consts::FRAC_PI_2, [0.0, 0.0, 1.0])
        .line_by([-40.0, 0.0, 0.0])
        .finish([1.0, 0.0, 0.0])
        .unwrap();
    let solid = Sweep::from_profile(
        profile,
        &path,
        Placement3::IDENTITY,
        &ProfilePolicy::new(1e-7),
    )
    .unwrap();
    let policy = RealizationPolicy::default();
    let warm = solid.realize(&policy).unwrap();
    assert!(warm.report.within_tolerance(), "fixture accuracy");
    assert!(warm.mesh.validate_deep().is_empty(), "fixture topology");
    assert!(
        warm.mesh.boundary_loops().unwrap().is_empty(),
        "fixture closure"
    );
    if std::env::args().nth(1).as_deref() == Some("profile") {
        let start = Instant::now();
        let mut iterations = 0_u64;
        while start.elapsed().as_secs() < 20 {
            black_box(solid.realize(&policy).unwrap());
            iterations += 1;
        }
        println!(
            "{iterations} fresh rounded sweep realizations in {:?}",
            start.elapsed()
        );
        return;
    }
    let samples: Vec<_> = (0..21)
        .map(|_| {
            let start = Instant::now();
            let output = solid.realize(&policy).unwrap();
            black_box(&output);
            let elapsed = start.elapsed().as_nanos();
            drop(output);
            elapsed
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"native_faces":solid.body().faces().len(),"vertices":warm.mesh.vertices().count(),"triangles":warm.mesh.faces().count(),"deviation":warm.report.max_deviation,"samples_ns":samples})
    );
}
