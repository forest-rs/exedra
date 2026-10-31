// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained-native machining and rectilinear Boolean scaling wind tunnel.
use exedra_analytic::{
    body::Body,
    extrusion::{EdgeRole, Extrusion, FaceRole},
    machining::{
        CylinderTool, MachiningPolicy, MachiningReport, subtract_through_cylinder_with_roles,
        subtract_through_cylinders_with_roles,
    },
    rectilinear::{BooleanOperation, RectilinearPolicy, boolean_rectilinear},
};
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_math::Placement3;
use std::{hint::black_box, time::Instant};
fn stock(min: [f64; 2], max: [f64; 2]) -> Extrusion {
    let outline = Loop::new(vec![
        Segment::line([max[0], min[1]]),
        Segment::line([max[0], max[1]]),
        Segment::line([min[0], max[1]]),
        Segment::line([min[0], min[1]]),
    ])
    .unwrap();
    Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        20.0,
    )
    .unwrap()
}
fn requests(holes: u32) -> (Extrusion, Vec<CylinderTool>, MachiningPolicy) {
    let half = (f64::from(holes) - 1.0) * 11.0;
    let source = stock([-half - 34.0, -50.0], [half + 34.0, 50.0]);
    let tools = (0..holes)
        .map(|i| {
            CylinderTool::new([f64::from(i) * 22.0 - half, 0.0, 0.0], [0.1, 0.0, 1.0], 4.0).unwrap()
        })
        .collect();
    let policy = MachiningPolicy {
        geometry_gap: 1e-9_f64.max((half + 34.0) * 1e-11),
        ..Default::default()
    };
    (source, tools, policy)
}
fn pattern(
    source: &Extrusion,
    tools: &[CylinderTool],
    policy: &MachiningPolicy,
    batch: bool,
) -> (Body<FaceRole, EdgeRole>, Vec<MachiningReport>) {
    if batch {
        let result = subtract_through_cylinders_with_roles(
            black_box(source.body()),
            black_box(tools),
            policy,
            source.body().faces()[0].role,
            source.body().edges()[0].role,
        )
        .unwrap();
        return (result.body, result.operations);
    }
    let mut body: Option<Body<FaceRole, EdgeRole>> = None;
    let mut history = Vec::with_capacity(tools.len());
    for tool in tools {
        let input = body.as_ref().unwrap_or_else(|| source.body());
        let result = subtract_through_cylinder_with_roles(
            black_box(input),
            black_box(tool),
            policy,
            source.body().faces()[0].role,
            source.body().edges()[0].role,
        )
        .unwrap();
        history.push(result.report);
        body = Some(result.body);
    }
    (body.unwrap_or_else(|| source.body().clone()), history)
}
/// Three-phase allocation window used by the existing isolated dhat harness.
/// Prepared source/tools and warmup are excluded. Retained includes the final
/// native body plus operation history; `native_only` drops history; released drops
/// the final body. No mesh, STEP, compiler, JS or GPU work is included.
pub fn allocation_window(mut checkpoint: impl FnMut(&'static str)) {
    let holes = std::env::var("EXEDRA_MACHINING_HOLES")
        .ok()
        .map(|s| s.parse().expect("hole count"))
        .unwrap_or(7);
    let (source, tools, policy) = requests(holes);
    let batch = std::env::var_os("EXEDRA_MACHINING_SINGLE").is_none();
    black_box(pattern(&source, &tools, &policy, batch));
    checkpoint("ready");
    let (body, history) = pattern(&source, &tools, &policy, batch);
    checkpoint("retained");
    drop(history);
    checkpoint("native_only");
    drop(body);
    checkpoint("released");
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|s| s == "rect") {
        rectilinear(&args[1..]);
        return;
    }
    let repetitions = args
        .first()
        .map(|s| s.parse::<u32>().expect("repetition count"))
        .unwrap_or(1000);
    let holes = args
        .get(1)
        .map(|s| s.parse::<u32>().expect("hole count"))
        .unwrap_or(7);
    assert!(holes > 0 && repetitions > 0, "counts must be positive");
    let batch = args.get(2).is_none_or(|s| s != "single");
    let (source, tools, policy) = requests(holes);
    let start = Instant::now();
    let mut checksum = 0.0;
    let mut faces = 0;
    for _ in 0..repetitions {
        let (body, history) = pattern(&source, &tools, &policy, batch);
        assert!(body.is_closed(), "machining must retain closed incidence");
        checksum += history.iter().map(|r| r.removed_volume).sum::<f64>();
        faces = body.faces().len();
        black_box((body, history));
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "workload=native_oblique_bores holes={holes} repetitions={repetitions} operations={} elapsed_ms={:.3} ns_per_operation={:.1} final_faces={faces} batch={batch} retained_history=true removed_volume_checksum={checksum:.9}",
        u64::from(repetitions) * u64::from(holes),
        elapsed * 1000.0,
        elapsed * 1e9 / (f64::from(repetitions) * f64::from(holes))
    );
}
fn rectilinear(args: &[String]) {
    let repetitions = args
        .first()
        .map(|s| s.parse::<u32>().expect("repetition count"))
        .unwrap_or(100);
    let lugs = args
        .get(1)
        .map(|s| s.parse::<u32>().expect("lug count"))
        .unwrap_or(4);
    assert!(lugs > 0 && repetitions > 0, "counts must be positive");
    let half = (f64::from(lugs) - 1.0) * 10.0;
    let source = stock([-30.0, -half - 15.0], [30.0, half + 15.0]);
    let tools = (0..lugs)
        .map(|i| {
            let y = f64::from(i) * 20.0 - half;
            stock([30.0, y - 5.0], [45.0, y + 5.0])
        })
        .collect::<Vec<_>>();
    let policy = RectilinearPolicy::default();
    let start = Instant::now();
    let mut cells = 0;
    let mut face_tests = 0;
    let mut face_test_bound = 0;
    let mut faces = 0;
    let mut checksum = 0.0;
    for _ in 0..repetitions {
        let mut body = source.body().clone().map_roles(|_| (), |_| ());
        for tool in &tools {
            let result = boolean_rectilinear(
                black_box(&body),
                black_box(tool.body()),
                BooleanOperation::Union,
                &policy,
            )
            .unwrap();
            cells = result.report.cells;
            face_tests = result.report.face_tests;
            face_test_bound = result.report.face_test_bound;
            checksum += result.report.volume;
            body = result.body;
        }
        faces = body.faces().len();
        black_box(body);
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "workload=native_rectilinear_lug_union lugs={lugs} repetitions={repetitions} operations={} elapsed_ms={:.3} us_per_operation={:.3} final_cells={cells} final_faces={faces} face_tests={face_tests} face_test_bound={face_test_bound} volume_checksum={checksum:.9}",
        u64::from(repetitions) * u64::from(lugs),
        elapsed * 1000.0,
        elapsed * 1e6 / (f64::from(repetitions) * f64::from(lugs))
    );
}
