// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Transactional native ruling subdivision: borrowed source and cloned output.
use exedra_analytic::{
    body::{Body, FaceId},
    extrusion::Extrusion,
    realize::RealizationPolicy,
    split::{RuledSplitPolicy, split_ruled_face_at_u},
};
use exedra_curves::profile::{Bezier, Loop, Profile, ProfilePolicy, SegTag, Segment};
use exedra_math::Placement3;
use std::{error::Error, hint::black_box, time::Instant};

fn source() -> (Body<(), u8>, FaceId) {
    let outer = Loop::new(vec![
        Segment::line([4., 0.]),
        Segment::bezier(
            [0., 0.],
            Bezier::new(vec![[3., 2.], [1., 2.]], Some(vec![1., 1.4, 0.7, 1.])),
        )
        .tagged(SegTag(42)),
    ])
    .unwrap();
    let hole = Loop::new(vec![
        Segment::line([1.5, 0.6]),
        Segment::line([2.5, 0.6]),
        Segment::line([2.5, 0.3]),
        Segment::line([1.5, 0.3]),
    ])
    .unwrap();
    let profile = Profile::new(outer, vec![hole], &ProfilePolicy::new(1e-9)).unwrap();
    let solid = Extrusion::new(profile, Placement3::IDENTITY, 2.).unwrap();
    let face = solid
        .faces_tagged(SegTag(42))
        .next()
        .expect("named crown")
        .0;
    (solid.body().clone().map_roles(|_| (), |_| 0), face)
}
fn measure<T>(repetitions: usize, mut action: impl FnMut() -> T) -> f64 {
    for _ in 0..3 {
        black_box(action());
    }
    let mut samples: Vec<_> = (0..repetitions)
        .map(|_| {
            let start = Instant::now();
            let output = black_box(action());
            let elapsed = start.elapsed().as_secs_f64() * 1e6;
            black_box(&output);
            drop(output);
            elapsed
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}
/// One warmed split against a retained borrowed source. Input creation and
/// warmup are excluded; cloned output tables and the report are retained, then
/// released. No mesh, STEP, compiler, JS or rendering work is included.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let (body, face) = source();
    let policy = RuledSplitPolicy::default();
    drop(split_ruled_face_at_u(&body, face, 0.37, 99, &policy).unwrap());
    checkpoint("ready");
    let result = black_box(
        split_ruled_face_at_u(black_box(&body), face, 0.37, 99, black_box(&policy)).unwrap(),
    );
    checkpoint("retained");
    drop(result);
    checkpoint("released");
}
fn main() -> Result<(), Box<dyn Error>> {
    let repetitions = std::env::args()
        .nth(1)
        .map_or(Ok(101), |s| s.parse::<usize>())?;
    assert!(repetitions > 0, "positive repetition count required");
    let (body, face) = source();
    let policy = RuledSplitPolicy::default();
    let source_validation = measure(repetitions, || {
        body.validate_boundary(&policy.validation).unwrap()
    });
    let split = measure(repetitions, || {
        split_ruled_face_at_u(&body, face, 0.37, 99, &policy).unwrap()
    });
    let output = split_ruled_face_at_u(&body, face, 0.37, 99, &policy)?;
    let before = body.realize(Placement3::IDENTITY, &RealizationPolicy::default())?;
    let after = output
        .body
        .realize(Placement3::IDENTITY, &RealizationPolicy::default())?;
    assert!(
        before.report.within_tolerance() && after.report.within_tolerance(),
        "both meshes meet geometric tolerance"
    );
    assert!(
        before.mesh.boundary_loops()?.is_empty() && after.mesh.boundary_loops()?.is_empty(),
        "both solids remain watertight"
    );
    assert_eq!(
        output.body.vertices().len(),
        body.vertices().len() + 2,
        "two shared rim vertices"
    );
    assert_eq!(
        output.body.edges().len(),
        body.edges().len() + 3,
        "three additional shared edges"
    );
    assert_eq!(
        output.body.faces().len(),
        body.faces().len() + 1,
        "one additional native face"
    );
    println!(
        "{}",
        serde_json::json!({"case":"weighted_crown_rectangular_hole","repetitions":repetitions,
        "scope":"release medians; source validation is a baseline workload, not an earlier split implementation; split includes input/output validation, result cloning and correspondence; source creation/warmup/mesh/STEP/IO/output destruction excluded",
        "source_validation_us":source_validation,"split_us":split,
        "source":{"vertices":body.vertices().len(),"edges":body.edges().len(),"faces":body.faces().len(),"triangles":before.mesh_faces.len()},
        "result":{"vertices":output.body.vertices().len(),"edges":output.body.edges().len(),"faces":output.body.faces().len(),"triangles":after.mesh_faces.len()},
        "source_proof":{"controls":output.report.work.source_controls,"metadata_work":output.report.work.source_metadata_work,"preparation_work_bound":output.report.work.source_preparation_work,"enclosure_work":output.report.work.source_enclosure_work},
        "input_coedges":output.report.work.input_coedges,"output_coedges":output.report.validation.coedges,
        "output_pair_work_bound":output.report.validation.pair_test_bound,"output_geometry_gap":output.report.validation.geometry.max_gap})
    );
    Ok(())
}
