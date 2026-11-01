// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Weighted Bezier profile, native solid, parameter UVs and exact STEP.
//! `cargo run -p analytic_extrusion --example spline_profile -- /tmp/spline-profile.step`

mod support;

use exedra::{
    analytic::{
        extrusion::{Extrusion, FaceRole},
        realize::{RealizationPolicy, TriangleQuality},
    },
    compile::{CompilePolicy, NativeSource, PartCompiler, PartId, PartInput, PartSource},
    constructive::ir::Placement3,
    curves::profile::{Bezier, Loop, Profile, ProfilePolicy, SegTag, Segment},
    step::{StepOptions, export_body},
};
use std::{error::Error, time::Instant};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const CROWN: SegTag = SegTag(42);

fn main() -> Result<()> {
    // Along the chord, then back over a cubic crown whose positive
    // rational weights stay authoritative in the solid.
    let outer = Loop::new(vec![
        Segment::line([4., 0.]),
        Segment::bezier(
            [0., 0.],
            Bezier::new(vec![[3., 2.], [1., 2.]], Some(vec![1., 1.4, 0.7, 1.])),
        )
        .tagged(CROWN),
    ])?;
    let hole = Loop::new(vec![
        Segment::line([1.5, 0.6]),
        Segment::line([2.5, 0.6]),
        Segment::line([2.5, 0.3]),
        Segment::line([1.5, 0.3]),
    ])?;
    let start = Instant::now();
    let profile = Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9))?;
    let solid = Extrusion::new(profile, Placement3::translate(10., 20., 30.), 2.)?;
    let construction_ms = start.elapsed().as_secs_f64() * 1000.;
    println!("proved profile and construction: {construction_ms:.3} ms");
    assert!(
        solid.body().is_closed(),
        "extrusion must share closed native incidence"
    );
    assert_eq!(
        solid.faces_tagged(CROWN).count(),
        1,
        "the crown wall must retain its feature tag"
    );
    let display = solid.realize_with_uvs(
        &RealizationPolicy {
            tolerance: 0.01,
            quality: Some(TriangleQuality::new(0.2)),
            ..Default::default()
        },
        |_, f| {
            if matches!(
                f.role,
                FaceRole::Wall {
                    tag: Some(CROWN),
                    ..
                }
            ) {
                42
            } else {
                0
            }
        },
        |_, coordinates| Ok::<_, std::convert::Infallible>(texture_coordinates(coordinates)),
    )?;
    assert!(
        display.report.within_tolerance(),
        "display must meet its requested tolerance"
    );
    assert!(
        display.mesh.boundary_loops()?.is_empty(),
        "display must remain watertight"
    );
    println!(
        "display: {} triangles; {:?}",
        display.mesh_faces.len(),
        display.report
    );
    let quality = display
        .report
        .quality
        .as_ref()
        .expect("requested shape evidence");
    println!(
        "whole display shape: target={} observed={} remaining_bad={} within_target={} budget_exhausted={}",
        quality.target_radius_edge_ratio,
        quality.max_radius_edge_ratio,
        quality.remaining_bad_triangles,
        quality.within_target(),
        quality.budget_exhausted,
    );
    if let Some(path) = std::env::args().nth(2) {
        support::export(
            &path,
            "Checked spline profile",
            vec![support::Exhibit {
                name: "Weighted crown with rectangular hole",
                mesh: display.mesh,
                placement: Placement3::IDENTITY,
                color: [0.35, 0.62, 0.8, 1.0],
                grid: true,
                cut_regions: &[],
            }],
        )?;
    }
    let step = export_body(
        solid.body(),
        solid.frame(),
        &StepOptions {
            name: "Weighted crown with a hole",
            ..Default::default()
        },
    )?;
    assert!(
        step.text.contains("RATIONAL_B_SPLINE_SURFACE"),
        "STEP must retain the exact rational ruled wall"
    );
    assert!(
        step.text.contains("RATIONAL_B_SPLINE_CURVE"),
        "STEP must retain the exact rational boundary"
    );
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(path, &step.text)?;
    }
    println!("STEP: {} bytes; {:?}", step.text.len(), step.report.counts);
    // Installation preserves exact geometry, tags and placement through the same
    // generic body path as other directly authored BReps. Display quality is separate.
    let native = NativeSource::from(solid);
    let source = PartSource::Native(native.clone());
    let inputs = [PartInput::new("crown", &source)];
    let mut compiler = PartCompiler::with_capacity(4);
    for tolerance in [0.05, 0.005] {
        let mut policy = CompilePolicy::default();
        policy.native.tolerance = tolerance;
        let snapshot = compiler.compile_snapshot(&inputs, &policy)?;
        let body = snapshot.body(PartId(0), 0).expect("one native solid");
        let selection = body.native_render_face(0).expect("source correspondence");
        native.resolve_face(&selection)?;
        println!(
            "compiled at {tolerance}: {} triangles",
            snapshot
                .compiled()
                .part(PartId(0))
                .expect("part")
                .triangle_count()
        );
    }
    Ok(())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "display UVs are f32; the native mapper independently checks finite output"
)]
fn texture_coordinates(coordinates: [[f64; 2]; 3]) -> [[f32; 2]; 3] {
    coordinates.map(|uv| uv.map(|v| v as f32))
}
