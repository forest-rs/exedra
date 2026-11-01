// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native ruled-patch nearest-point query wind tunnel.
//!
//! The fixed-U scan is an explicitly uncertified candidate baseline. It uses
//! exact native source curves, eliminates V on each ruling, and does not mesh.

#[path = "../spline_wall_fixture.rs"]
mod fixture;

use exedra_analytic::{
    geometry::{Curve, CurveSpan, NurbsCurve, ParameterDomain, ParameterRange, Ruled, Surface},
    query::{ProjectionPolicy, RuledProjection},
};
use exedra_math::{Placement3, add, dot, scale, sub};
use std::{error::Error, hint::black_box, path::Path, time::Instant};

#[derive(Clone, Copy)]
struct Candidate {
    uv: [f64; 2],
    point: [f64; 3],
    distance: f64,
}

fn length(v: [f64; 3]) -> f64 {
    v[0].hypot(v[1]).hypot(v[2])
}

fn scan(wall: &Ruled, target: [f64; 3], intervals: u32) -> Candidate {
    let curves = wall.curves();
    let mut best = Candidate {
        uv: [0.0; 2],
        point: [0.0; 3],
        distance: f64::INFINITY,
    };
    for i in 0..=intervals {
        let u = f64::from(i) / f64::from(intervals);
        let a = curves[0].evaluate(u).unwrap().point;
        let b = curves[1].evaluate(u).unwrap().point;
        let direction = sub(b, a);
        let v = (dot(sub(target, a), direction) / dot(direction, direction)).clamp(0.0, 1.0);
        let point = add(a, scale(direction, v));
        let distance = length(sub(point, target));
        if distance < best.distance {
            best = Candidate {
                uv: [u, v],
                point,
                distance,
            };
        }
    }
    best
}

fn median(repetitions: usize, mut action: impl FnMut()) -> f64 {
    for _ in 0..3 {
        action();
    }
    let mut times: Vec<_> = (0..repetitions)
        .map(|_| {
            let start = Instant::now();
            action();
            start.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    times.sort_unstable_by(f64::total_cmp);
    times[times.len() / 2]
}

fn plane() -> Surface {
    let span = |z| {
        CurveSpan::new(
            Curve::line([0.0, 0.0, z], [4.0, 0.0, 0.0]).unwrap(),
            ParameterRange::UNIT,
            false,
        )
        .unwrap()
    };
    Surface::ruled(span(0.0), span(1.0)).unwrap()
}

fn wavy() -> Surface {
    let range = ParameterRange::new(2.0, 5.0).unwrap();
    let curve = Curve::nurbs(
        NurbsCurve::new(
            6,
            [vec![2.0; 6], vec![5.0; 6]].concat(),
            [0.0, 3.0, -3.0, -3.0, 3.0, 0.0]
                .into_iter()
                .zip([0.0, 0.8, 1.6, 2.4, 3.2, 4.0])
                .map(|(y, x)| [x, y, 0.0])
                .collect(),
            Some(vec![1.0, 0.8, 1.3, 1.3, 0.8, 1.0]),
        )
        .unwrap(),
    )
    .unwrap();
    let upper = curve
        .transformed(Placement3::translate(0.0, 0.0, 1.0))
        .unwrap();
    Surface::ruled(
        CurveSpan::new(curve, range, true).unwrap(),
        CurveSpan::new(upper, range, true).unwrap(),
    )
    .unwrap()
}

fn targets(surface: &Surface) -> Vec<(&'static str, [f64; 3])> {
    let mut points = Vec::new();
    for (name, uv, offset) in [
        ("interior-positive", [0.37, 0.43], 0.25),
        ("interior-negative", [0.71, 0.64], -0.25),
        ("on-support", [0.5, 0.5], 0.0),
        ("u-start", [0.0, 0.43], 0.25),
        ("u-end", [1.0, 0.43], 0.25),
        ("v-start", [0.37, 0.0], 0.25),
        ("v-end", [0.37, 1.0], 0.25),
        ("corner", [1.0, 1.0], 0.25),
    ] {
        let e = surface.evaluate(uv).unwrap();
        points.push((name, add(e.point, scale(e.normal, offset))));
    }
    points.push(("outside-domain", [-1.0, -0.5, 2.0]));
    points.push(("above-crown", [2.0, 2.0, 0.5]));
    points.push((
        "on-support-first",
        surface.evaluate([0.37, 0.43]).unwrap().point,
    ));
    points.push((
        "on-support-second",
        surface.evaluate([0.71, 0.64]).unwrap().point,
    ));
    points
}

/// One warmed prepared query; source, preparation, scratch and profiler setup
/// are outside the allocation window. Query reports have no owned allocation.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let body = fixture::body(true, 0.75);
    let Surface::Ruled(wall) = &body.faces()[2].surface else {
        unreachable!()
    };
    let domain = ParameterDomain {
        u: ParameterRange::UNIT,
        v: ParameterRange::UNIT,
    };
    let query = RuledProjection::prepare(wall, domain, &ProjectionPolicy::default()).unwrap();
    let mut scratch = query.scratch().unwrap();
    black_box(query.closest([2., 2., 0.5], &mut scratch).unwrap());
    checkpoint("ready");
    black_box(query.closest([2., 2., 0.5], &mut scratch).unwrap());
    checkpoint("retained");
    checkpoint("released");
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let repetitions = args.get(1).map_or(Ok(31), |s| s.parse())?;
    let intervals: u32 = args.get(2).map_or(Ok(1024), |s| s.parse())?;
    assert!(repetitions > 0 && intervals > 0, "positive work limits");
    let output = args.get(3).map(Path::new);
    let method = std::env::var("EXEDRA_PROJECTION_METHOD").unwrap_or_else(|_| "bounded".into());
    let policy = ProjectionPolicy {
        distance_tolerance: std::env::var("EXEDRA_PROJECTION_TOLERANCE")
            .map_or(1e-6, |s| s.parse().expect("distance tolerance")),
        max_work: std::env::var("EXEDRA_PROJECTION_WORK")
            .map_or(100_000, |s| s.parse().expect("work budget")),
        ..Default::default()
    };
    if let Some(path) = output {
        std::fs::create_dir_all(path)?;
    }
    let prism = fixture::body(true, 1.0);
    let taper = fixture::body(true, 0.75);
    let plane = plane();
    let wavy = wavy();
    let mut rows = Vec::new();
    for (name, surface) in [
        ("plane", &plane),
        ("rational-prism", &prism.faces()[2].surface),
        ("rational-taper", &taper.faces()[2].surface),
        ("wavy-reversed-nonunit", &wavy),
    ] {
        let Surface::Ruled(wall) = surface else {
            unreachable!("ruled fixture");
        };
        let domain = ParameterDomain {
            u: ParameterRange::UNIT,
            v: ParameterRange::UNIT,
        };
        let prepare_us = median(repetitions, || {
            black_box(RuledProjection::prepare(wall, domain, &policy).unwrap());
        });
        let prepared = RuledProjection::prepare(wall, domain, &policy)?;
        let mut scratch = prepared.scratch()?;
        for (query, target) in targets(surface) {
            if method == "bounded" {
                let query_us = median(repetitions, || {
                    black_box(prepared.closest(black_box(target), &mut scratch).unwrap());
                });
                let found = prepared.closest(target, &mut scratch)?;
                let work = found.work;
                let row = serde_json::json!({"case":name,"query":query,"target":target,"method":"bounded-native-ruled","query_us":query_us,"prepare_us":prepare_us,"preparation_capacity_bytes":prepared.preparation().owned_capacity_bytes,"preparation_work_bound":prepared.preparation().preparation_work_bound,"scratch_capacity_bytes":scratch.capacity_bytes(),"uv":found.witness.uv,"point":found.witness.point,"point_error_upper":found.witness.point_error_upper,"distance_upper":found.witness.distance_upper,"distance_lower":found.distance_lower,"status":format!("{:?}",found.status),"certain":found.within_tolerance(),"work":{"total":work.total(),"ruling_evaluations":work.ruling_evaluations,"interval_tests":work.interval_tests,"source_enclosures":work.source_enclosures,"control_steps":work.control_steps}});
                println!("{row}");
                rows.push(row);
                continue;
            }
            assert_eq!(method, "scan", "method must be bounded or scan");
            let query_us = median(repetitions, || {
                black_box(scan(black_box(wall), black_box(target), intervals));
            });
            let found = scan(wall, target, intervals);
            let row = serde_json::json!({
                "case":name,"query":query,"target":target,
                "method":"uncertified-fixed-u-ruling-scan",
                "intervals":intervals,"ruling_evaluations":u64::from(intervals)+1,
                "query_us":query_us,"uv":found.uv,"point":found.point,
                "candidate_distance":found.distance,"certain":false,
            });
            println!("{row}");
            rows.push(row);
        }
    }
    if let Some(path) = output {
        std::fs::write(path.join("queries.json"), serde_json::to_vec_pretty(&rows)?)?;
    }
    Ok(())
}
