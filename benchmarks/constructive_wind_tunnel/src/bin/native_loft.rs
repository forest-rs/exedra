// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained loft construction, remeshing, charts and direct mesh realization.

#[path = "../loft_fixture.rs"]
mod fixture;
use std::{hint::black_box, time::Instant};
fn measure<T>(mut action: impl FnMut() -> T) -> Vec<u128> {
    for _ in 0..3 {
        black_box(action());
    }
    (0..31)
        .map(|_| {
            let start = Instant::now();
            let result = action();
            black_box(&result);
            let elapsed = start.elapsed().as_nanos();
            drop(result);
            elapsed
        })
        .collect()
}
fn main() {
    if let Some(mode) = std::env::args().nth(1) {
        assert!(
            matches!(mode.as_str(), "native" | "charted"),
            "profile mode must name native or charted"
        );
        let sections = fixture::sections(true);
        let loft = fixture::native(&sections);
        let start = Instant::now();
        let mut iterations = 0_u64;
        while start.elapsed().as_secs() < 20 {
            black_box(if mode == "charted" {
                fixture::charted_mesh(&loft)
            } else {
                fixture::native_mesh(&loft)
            });
            iterations += 1;
        }
        println!(
            "{mode}: {iterations} fresh realizations in {:?}",
            start.elapsed()
        );
        return;
    }
    let mut cases = Vec::new();
    for hollow in [false, true] {
        let sections = fixture::sections(hollow);
        let loft = fixture::native(&sections);
        assert_eq!(
            loft,
            fixture::native(&sections),
            "native preparation must be repeatable"
        );
        let native = fixture::native_mesh(&loft);
        let mesh = fixture::mesh_loft(&sections);
        assert!(native.report.within_tolerance(), "native tolerance");
        assert!(
            mesh.loft_sampling
                .as_ref()
                .unwrap()
                .within_surface_tolerance(),
            "mesh wall tolerance"
        );
        for m in [&native.mesh, &mesh.mesh] {
            assert!(m.validate_deep().is_empty(), "deep validity");
            assert!(m.boundary_loops().unwrap().is_empty(), "closure");
        }
        let triangles =
            |m: &exedra_mesh::Mesh| m.faces().map(|f| m.face_loop(f).count() - 2).sum::<usize>();
        let timings = serde_json::json!({"construct_native":measure(||fixture::native(&sections)),
            "native_realize":measure(||fixture::native_mesh(&loft)),"native_charted":measure(||fixture::charted_mesh(&loft)),
            "direct_mesh":measure(||fixture::mesh_loft(&sections))});
        cases.push(serde_json::json!({"hollow":hollow,"native_faces":loft.body().faces().len(),
            "native_vertices":native.mesh.vertices().count(),"native_triangles":triangles(&native.mesh),
            "mesh_vertices":mesh.mesh.vertices().count(),"mesh_triangles":triangles(&mesh.mesh),
            "native_deviation":native.report.max_deviation,"timings_ns":timings}));
    }
    println!("{}", serde_json::json!({"cases":cases}));
}
