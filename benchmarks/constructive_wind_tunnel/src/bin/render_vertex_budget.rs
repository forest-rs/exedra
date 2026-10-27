// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Delivered vertex limits over a fixed source, excluding source construction.

use exedra_mesh::{
    BuildParams, ExtractParams, ExtractStats, Mesh, NormalsSource, RenderVertexLimit, TriMesh,
};
use std::{hint::black_box, time::Instant};

fn fixture() -> Mesh {
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    for i in 0..1000_u16 {
        let x = f32::from(i) * 2.0;
        let i = u32::from(i);
        positions.extend([[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]]);
        triangles.push([i * 3, i * 3 + 1, i * 3 + 2]);
    }
    Mesh::from_indexed_triangles(&positions, &triangles, &BuildParams::default()).unwrap()
}

fn extract(
    mesh: &Mesh,
    maximum: Option<u32>,
) -> Result<(TriMesh, ExtractStats), RenderVertexLimit> {
    // Deliberately excludes global normal derivation to isolate early extraction
    // refusal. A delivered-vertex limit does not promise to bound that scratch.
    let params = ExtractParams {
        normals: NormalsSource::CustomOnly,
        ..Default::default()
    };
    match maximum {
        Some(maximum) => mesh.to_trimesh_bounded(&params, maximum),
        None => Ok(mesh.to_trimesh(&params)),
    }
}

/// Fixed mesh prepared before the caller starts its existing allocation profiler.
/// Set `EXEDRA_RENDER_VERTICES` for bounded extraction; unset means uncapped.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let mesh = fixture();
    let maximum = std::env::var("EXEDRA_RENDER_VERTICES")
        .ok()
        .map(|s| s.parse().unwrap());
    checkpoint("ready");
    let output = black_box(extract(&mesh, maximum));
    checkpoint("retained");
    drop(output);
    checkpoint("released");
}

fn main() {
    let mesh = fixture();
    assert_eq!(
        extract(&mesh, Some(3000)).unwrap(),
        extract(&mesh, None).unwrap(),
        "sufficient bound preserves every buffer and counter"
    );
    let mut rows = Vec::new();
    for maximum in [None, Some(3000), Some(4)] {
        let mut ns = Vec::new();
        for pass in 0..104 {
            let start = Instant::now();
            let output = black_box(extract(&mesh, maximum));
            let elapsed = start.elapsed().as_nanos();
            match (&output, maximum) {
                (Ok((tri, _)), None | Some(3000)) => assert_eq!(
                    tri.positions.len(),
                    3000,
                    "all fixture vertices are emitted"
                ),
                (Err(error), Some(4)) => assert_eq!(
                    error.required_at_least, 5,
                    "refusal stops at first excess vertex"
                ),
                _ => panic!("unexpected extraction outcome"),
            }
            if pass >= 3 {
                ns.push(elapsed);
            }
            drop(output);
        }
        rows.push(serde_json::json!({"maximum":maximum,"ns":ns}));
    }
    println!(
        "{}",
        serde_json::json!({"cases":rows,"scope":"1000 disconnected triangles,3000 unique vertices; custom-only missing normals avoid global normal derivation; fixed source construction and output destruction excluded; uncapped,exact sufficient bound,early refusal at5/4; no JS/GPU"})
    );
}
