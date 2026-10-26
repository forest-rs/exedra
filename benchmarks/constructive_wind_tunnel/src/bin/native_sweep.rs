// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native and constructive realization of the same constant-section rail.

#[path = "../sweep_fixture.rs"]
mod fixture;
use std::{hint::black_box, time::Instant};

fn measure<T>(mut action: impl FnMut() -> T) -> Vec<u128> {
    for _ in 0..3 {
        black_box(action());
    }
    (0..21)
        .map(|_| {
            let start = Instant::now();
            let output = action();
            black_box(&output);
            let elapsed = start.elapsed().as_nanos();
            drop(output);
            elapsed
        })
        .collect()
}
fn main() {
    let profile = fixture::profile();
    let path = fixture::path();
    let native = fixture::native(&profile, &path);
    let recipe = fixture::recipe(profile.clone(), path.clone());
    if let Some(mode) = std::env::args().nth(1) {
        assert!(
            matches!(mode.as_str(), "native" | "constructive"),
            "profile mode must name a backend"
        );
        let start = Instant::now();
        let mut iterations = 0_u64;
        while start.elapsed().as_secs() < 20 {
            if mode == "native" {
                black_box(fixture::native_realization(&native));
            } else {
                black_box(fixture::recipe_realization(&recipe));
            }
            iterations += 1;
        }
        println!(
            "{mode}: {iterations} fresh realizations in {:?}",
            start.elapsed()
        );
        return;
    }
    let native_output = fixture::native_realization(&native);
    let a = &native_output.mesh;
    let recipe_output = fixture::recipe_realization(&recipe);
    let b = &recipe_output.bodies[0].body.mesh;
    for mesh in [a, b] {
        assert!(mesh.validate_deep().is_empty(), "valid matched fixture");
        assert!(
            mesh.boundary_loops().unwrap().is_empty(),
            "closed matched fixture"
        );
    }
    let triangles = |mesh: &exedra_mesh::Mesh| {
        mesh.faces()
            .map(|f| mesh.face_loop(f).count() - 2)
            .sum::<usize>()
    };
    let mut results = Vec::new();
    for round in 0..4 {
        for native_first in if round % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        } {
            let (backend, samples) = if native_first {
                ("native", measure(|| fixture::native_realization(&native)))
            } else {
                (
                    "constructive",
                    measure(|| fixture::recipe_realization(&recipe)),
                )
            };
            results.push(serde_json::json!({"round":round,"backend":backend,"samples_ns":samples}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"native_vertices":a.vertices().count(),"native_triangles":triangles(a),
        "constructive_vertices":b.vertices().count(),"constructive_triangles":triangles(b),
        "native_faces":native.body().faces().len(),"native_volume":native.volume(),"native_area":native.surface_area(),"results":results})
    );
}
