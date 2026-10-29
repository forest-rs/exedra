// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Repeated native valve-block point queries, independent of display meshes.
use exedra::analytic::{
    body::Body,
    extrusion::Extrusion,
    machining::{
        BoreDepth, BoreStep, BoreTool, CylinderTool, MachiningPolicy, subtract_bore,
        subtract_parallel_passages,
    },
    query::{PointLocation, SolidQuery, SolidQueryPolicy},
};
use exedra::constructive::ir::Placement3;
use exedra::profile::{Loop, Profile, ProfilePolicy, Segment};
use std::{hint::black_box, time::Instant};

fn valve_block() -> Body {
    let outline = Loop::new(vec![
        Segment::line([30.0, -20.0]),
        Segment::line([30.0, 20.0]),
        Segment::line([-30.0, 20.0]),
        Segment::line([-30.0, -20.0]),
    ])
    .unwrap();
    let source = Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        20.0,
    )
    .unwrap();
    let source = source.body().clone().map_roles(|_| (), |_| ());
    let policy = MachiningPolicy::default();
    let bore = BoreTool::new(
        [15.0, 0.0, 20.0],
        [0.0, 0.0, -1.0],
        &[
            BoreStep {
                radius: 5.0,
                end: BoreDepth::ToDepth(6.0),
            },
            BoreStep {
                radius: 2.0,
                end: BoreDepth::ToDepth(12.0),
            },
        ],
    )
    .unwrap();
    let source = subtract_bore(&source, &bore, &policy).unwrap().body;
    let passages =
        [-13.0, -7.0].map(|x| CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 4.0).unwrap());
    subtract_parallel_passages(&source, &passages, &policy)
        .unwrap()
        .body
}
fn probes() -> Vec<[f64; 3]> {
    (-16..=16)
        .flat_map(|x| {
            (-11..=11).flat_map(move |y| {
                [3.3, 9.3, 15.3]
                    .map(|z| [f64::from(x) * 2.0 + 0.137, f64::from(y) * 2.0 + 0.219, z])
            })
        })
        .collect()
}
fn batch(query: &SolidQuery<'_>, probes: &[[f64; 3]]) -> ([usize; 3], usize, usize) {
    let mut counts = [0_usize; 3];
    let mut work = 0;
    let mut rays = 0;
    for &point in probes {
        let result = query.query_point(black_box(point)).unwrap();
        counts[match result.location {
            PointLocation::Inside => 0,
            PointLocation::Outside => 1,
            PointLocation::Boundary => 2,
        }] += 1;
        work += result.work.total();
        rays += result.work.ray_attempts;
        black_box(result);
    }
    (counts, work, rays)
}
/// Existing isolated dhat protocol: construction, probes and warmup are excluded.
/// `EXEDRA_SOLID_QUERY_PHASE=prepare` measures preparation and its destruction;
/// the default measures one repeated-query batch against existing preparation.
/// No compiler, scene, mesh, STEP, JS, transfer or rendering work is included.
pub fn allocation_window(mut checkpoint: impl FnMut(&'static str)) {
    let body = valve_block();
    let policy = SolidQueryPolicy::default();
    let points = probes();
    let prepared = SolidQuery::prepare(&body, &policy).unwrap();
    black_box(batch(&prepared, &points));
    if std::env::var("EXEDRA_SOLID_QUERY_PHASE").is_ok_and(|s| s == "prepare") {
        checkpoint("ready");
        let prepared = SolidQuery::prepare(black_box(&body), black_box(&policy)).unwrap();
        black_box(prepared.report());
        checkpoint("retained");
        drop(prepared);
        checkpoint("released");
    } else {
        checkpoint("ready");
        black_box(batch(&prepared, &points));
        checkpoint("retained");
        // Queries return owned scalar records and retain no temporary buffers.
        checkpoint("released");
    }
}
fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}
fn main() {
    let repetitions = std::env::args().nth(1).map_or(10, |s| {
        s.parse::<u32>().expect("positive batch repetitions")
    });
    assert!(repetitions > 0, "batch repetitions must be positive");
    let body = valve_block();
    let policy = SolidQueryPolicy::default();
    let points = probes();
    let query = SolidQuery::prepare(&body, &policy).unwrap();
    println!("native valve block: box 60x40x20, joined R4 passages, R5/R2 blind stepped bore");
    println!("prepare evidence: {:?}", query.report());
    for (name, point) in [
        ("material", [0.0, 0.0, 10.0]),
        ("joined passage", [-10.0, 0.0, 10.0]),
        ("bore", [15.0, 0.0, 18.0]),
        ("under blind floor", [15.0, 0.0, 7.0]),
        ("shoulder", [18.0, 0.0, 14.0]),
        ("floor", [15.0, 0.0, 8.0]),
    ] {
        let result = query.query_point(point).unwrap();
        println!(
            "{name}: point={point:?} location={:?} boundary={:?} work={:?}",
            result.location, result.boundary, result.work
        );
    }
    let mut prepare_ns = [0.0; 31];
    for time in &mut prepare_ns {
        let start = Instant::now();
        let prepared = SolidQuery::prepare(black_box(&body), black_box(&policy)).unwrap();
        *time = start.elapsed().as_secs_f64() * 1e9;
        black_box(prepared);
    }
    let expected = batch(&query, &points);
    let mut query_ns = [0.0; 9];
    for time in &mut query_ns {
        let start = Instant::now();
        for _ in 0..repetitions {
            assert_eq!(
                black_box(batch(black_box(&query), black_box(&points))),
                expected,
                "repeated native queries must preserve result and work counts"
            );
        }
        *time = start.elapsed().as_secs_f64() * 1e9 / f64::from(repetitions);
    }
    println!(
        "timing: native release, geometry/probes prepared outside query window, preparation construction only; query includes scalar result handling and counters"
    );
    println!(
        "prepare_median_us={:.3} query_batch_median_ms={:.3} points_per_batch={} ns_per_point={:.1} inside={} outside={} boundary={} work_per_batch={} rays_per_batch={}",
        median(&mut prepare_ns) / 1e3,
        median(&mut query_ns) / 1e6,
        points.len(),
        median(&mut query_ns)
            / f64::from(u32::try_from(points.len()).expect("bounded probe count")),
        expected.0[0],
        expected.0[1],
        expected.0[2],
        expected.1,
        expected.2
    );
    if let Some(directory) = std::env::args().nth(2) {
        export_sections(&query, std::path::Path::new(&directory)).expect("section export");
    }
    let limited = SolidQuery::prepare(
        &body,
        &SolidQueryPolicy {
            max_query_work: 1,
            ..policy
        },
    )
    .unwrap();
    let refused = limited.query_point([0.0, 0.0, 10.0]).unwrap_err();
    println!("bounded refusal: {refused}");
}

fn export_sections(query: &SolidQuery<'_>, directory: &std::path::Path) -> std::io::Result<()> {
    use std::{fmt::Write as _, io::Write as _};
    std::fs::create_dir_all(directory)?;
    let mut csv =
        std::io::BufWriter::new(std::fs::File::create(directory.join("valve-sections.csv"))?);
    writeln!(
        csv,
        "x,y,z,location,boundary_kind,boundary_id,work,ray_attempts"
    )?;
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="740" height="580" viewBox="0 0 740 580"><rect width="740" height="580" fill="#f8fafb"/><style>text{font:13px system-ui,sans-serif;fill:#263238}</style><text x="16" y="23">Native solid point queries · material / void / boundary</text><rect x="16" y="35" width="12" height="12" fill="#91a7b0"/><text x="33" y="46">material</text><rect x="115" y="35" width="12" height="12" fill="white" stroke="#c7d0d4"/><text x="132" y="46">void</text><rect x="188" y="35" width="12" height="12" fill="#dc8b33"/><text x="205" y="46">boundary</text>"##,
    );
    for (panel, z) in [3.3, 8.0, 14.0, 17.0].into_iter().enumerate() {
        let column = u32::try_from(panel % 2).expect("two columns");
        let row = u32::try_from(panel / 2).expect("two rows");
        let (left, top) = (16 + column * 365, 80 + row * 250);
        writeln!(svg, r#"<text x="{left}" y="{}">z = {z}</text>"#, top - 8).expect("string write");
        for y in -22..22 {
            for x in -32..32 {
                let point = [f64::from(x) + 0.5, f64::from(y) + 0.5, z];
                let result = query.query_point(point).expect("section query");
                let (name, color) = match result.location {
                    PointLocation::Inside => ("inside", "#91a7b0"),
                    PointLocation::Outside => ("outside", "#ffffff"),
                    PointLocation::Boundary => ("boundary", "#dc8b33"),
                };
                let (kind, id) = match result.boundary {
                    Some(exedra::analytic::query::BoundaryWitness::Edge(id)) => {
                        ("edge", id.0.to_string())
                    }
                    Some(exedra::analytic::query::BoundaryWitness::Face(id)) => {
                        ("face", id.0.to_string())
                    }
                    None => ("", String::new()),
                };
                writeln!(
                    csv,
                    "{},{},{z},{name},{kind},{id},{},{}",
                    point[0],
                    point[1],
                    result.work.total(),
                    result.work.ray_attempts
                )?;
                let px = i64::from(left) + i64::from(x + 32) * 5;
                let py = i64::from(top) + i64::from(21 - y) * 5;
                writeln!(
                    svg,
                    r#"<rect x="{px}" y="{py}" width="5" height="5" fill="{color}"/>"#
                )
                .expect("string write");
            }
        }
        writeln!(svg,r##"<rect x="{left}" y="{top}" width="320" height="220" fill="none" stroke="#c7d0d4"/>"##).expect("string write");
    }
    svg.push_str("</svg>\n");
    std::fs::write(directory.join("valve-sections.svg"), svg)?;
    csv.flush()?;
    println!("section artifacts: {}", directory.display());
    Ok(())
}
