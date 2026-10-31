// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Timing benchmark for `exedra_curves`.
//!
//! Four workloads matter to its callers: one long NURBS curve, many short
//! curves taken all the way to tube meshes (where per-curve setup and
//! allocation can outweigh the arithmetic), measuring, and realization
//! plans kept across updates (building one, moving one control or all of
//! them, and realizing a tube or GPU data). Each scenario reports the
//! median of several runs.
//!
//! Run with:
//! `cargo run --release -p exedra_curves_bench`
//!
//! Pass a scale factor to grow every workload, e.g.
//! `cargo run --release -p exedra_curves_bench -- 4`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use exedra_curves::realize::{
    Attribute, Centerline, CurveDescription, GpuCurve, PlanOptions, RealizationPlan,
};
use exedra_curves::{
    CubicBasis, Frame, Nurbs, NurbsSamples, Station, SweptMesh, Tolerance, Workspace,
    adaptive_stations, chain_lengths, rotation_minimizing_frames, rotation_minimizing_frames_into,
    sample, sample_into, stations, stations_into, tube, tube_into,
};

fn main() {
    let scale = std::env::args().nth(1).map_or(1, |arg| {
        arg.parse::<u32>().unwrap_or_else(|_| {
            eprintln!("the scale must be a positive integer, got {arg}");
            std::process::exit(2);
        })
    });
    println!("scenario                              work      median");

    // One long weighted cubic NURBS.
    let controls = 4096 * scale;
    let long = LongNurbs::new(controls);
    let curve = long
        .curve()
        .validate()
        .expect("the benchmark curve is valid");
    report("nurbs_sample", controls as usize, || {
        curve.sample(None, 8).map(|s| s.points.len())
    });
    let mut workspace = Workspace::new();
    let mut samples = NurbsSamples::default();
    report("nurbs_sample_into", controls as usize, || {
        curve
            .sample_into(None, 8, &mut workspace, &mut samples)
            .map(|()| samples.points.len())
    });
    let prepared = curve.prepare();
    let params = curve
        .sample(None, 8)
        .expect("the benchmark range is valid")
        .params;
    report("nurbs_evaluate", params.len(), || {
        params.iter().map(|&t| curve.evaluate(t)[0]).sum::<f64>()
    });
    report("nurbs_prepared_evaluate", params.len(), || {
        params.iter().map(|&t| prepared.evaluate(t)[0]).sum::<f64>()
    });
    report("nurbs_prepare", controls as usize, || {
        curve.prepare().spans()
    });
    report("nurbs_lengths", controls as usize, || {
        prepared.lengths(2).len()
    });

    // Many short Catmull-Rom curves to tubes.
    let curves = 10_000 * scale;
    let short = ShortCurves::new(curves);
    report("short_tubes_allocating", curves as usize, || {
        let mut vertices = 0;
        for controls in &short.controls {
            let at = stations(5, 4, false);
            let points = sample(CubicBasis::CatmullRom, controls, &short.spans, &at)
                .expect("the spans are in range");
            let (frames, _) = rotation_minimizing_frames(&points, false, None);
            let mesh = tube(&frames, &vec![0.05; frames.len()], 8, false).expect("the tube fits");
            vertices += mesh.positions.len();
        }
        vertices
    });
    let mut buffers = Buffers::default();
    report("short_tubes_into", curves as usize, || {
        let mut vertices = 0;
        for controls in &short.controls {
            buffers.tube(controls, &short.spans);
            vertices += buffers.mesh.positions.len();
        }
        vertices
    });

    // Frames through long runs of coincident points.
    let coincident = vec![[1.0, 2.0, 3.0]; 64_000 * scale as usize];
    report("coincident_frames", coincident.len(), || {
        rotation_minimizing_frames(&coincident, false, None).0.len()
    });

    // Measuring a long chain.
    let chain = &short.long_chain;
    report("chain_lengths", chain.spans.len(), || {
        chain_lengths(CubicBasis::BSpline, &chain.controls, &chain.spans, 2)
            .map(|lengths| lengths.len())
    });
    let tolerance = Tolerance {
        chord: 1e-4,
        angle: 0.05,
    };
    report("adaptive_stations", chain.spans.len(), || {
        adaptive_stations(
            CubicBasis::BSpline,
            &chain.controls,
            &chain.spans,
            false,
            tolerance,
            u32::MAX,
        )
        .map(|(at, _)| at.len())
    });

    realization(&long, &short);
}

/// Realization plans: one long NURBS curve planned, updated and realized,
/// and many short curves updated and swept as an animating caller does.
fn realization(long: &LongNurbs, short: &ShortCurves) {
    let curve = long
        .curve()
        .validate()
        .expect("the benchmark curve is valid")
        .to_curve();
    let controls = curve.points().len();
    let description = CurveDescription {
        centerline: Centerline::Nurbs(curve),
        widths: Attribute::Constant(0.05),
        normals: None,
        closed: false,
    };
    let mut workspace = Workspace::new();
    report("realize_plan_nurbs", controls, || {
        RealizationPlan::new(description.clone(), PlanOptions::new(8), &mut workspace)
            .map(|plan| plan.stations().len())
    });
    let mut plan = RealizationPlan::new(description, PlanOptions::new(8), &mut workspace)
        .expect("the benchmark curve plans");
    let original = long.points.clone();
    let mut nudged = original.clone();
    nudged[controls / 2][1] += 0.5;
    let shifted: Vec<[f64; 3]> = original.iter().map(|p| [p[0], p[1] + 0.5, p[2]]).collect();
    let work = plan
        .set_points(&nudged, &mut workspace)
        .expect("the points are valid");
    println!("  one control moved: {work:?}");
    let mut flip = false;
    report("realize_move_one_control", 1, || {
        flip = !flip;
        let points = if flip { &original } else { &nudged };
        plan.set_points(points, &mut workspace)
            .map(|work| work.evaluated_stations)
    });
    report("realize_move_all_controls", controls, || {
        flip = !flip;
        let points = if flip { &original } else { &shifted };
        plan.set_points(points, &mut workspace)
            .map(|work| work.evaluated_stations)
    });
    let mut mesh = SweptMesh::default();
    report("realize_tube_into", plan.stations().len(), || {
        plan.tube_into(8, &mut workspace, &mut mesh)
            .map(|()| mesh.positions.len())
    });
    let mut gpu = GpuCurve::default();
    plan.gpu_into([0.0; 3], &mut gpu).unwrap();
    report("realize_gpu_into_unchanged", plan.spans(), || {
        plan.gpu_into([0.0; 3], &mut gpu).unwrap().is_unchanged()
    });

    // Many short Catmull-Rom curves, each kept as a plan: every update
    // moves all their controls, then sweeps them.
    let mut plans: Vec<RealizationPlan> = short
        .controls
        .iter()
        .map(|controls| {
            let description = CurveDescription {
                centerline: Centerline::Cubic {
                    basis: CubicBasis::CatmullRom,
                    points: controls.clone(),
                    spans: short.spans.clone(),
                },
                widths: Attribute::Constant(0.1),
                normals: None,
                closed: false,
            };
            RealizationPlan::new(description, PlanOptions::new(4), &mut workspace)
                .expect("the short curves plan")
        })
        .collect();
    let lifted: Vec<Vec<[f64; 3]>> = short
        .controls
        .iter()
        .map(|controls| controls.iter().map(|p| [p[0], p[1], p[2] + 0.1]).collect())
        .collect();
    report("short_plans_update_and_tube", plans.len(), || {
        flip = !flip;
        let mut vertices = 0;
        for (i, plan) in plans.iter_mut().enumerate() {
            let points = if flip { &lifted[i] } else { &short.controls[i] };
            plan.set_points(points, &mut workspace)
                .expect("the points are valid");
            plan.tube_into(8, &mut workspace, &mut mesh)
                .expect("the tube fits");
            vertices += mesh.positions.len();
        }
        vertices
    });
}

/// Runs `work` several times and prints the median time.
fn report<R>(label: &str, items: usize, mut work: impl FnMut() -> R) {
    let mut times: Vec<Duration> = (0..7)
        .map(|_| {
            let start = Instant::now();
            black_box(work());
            start.elapsed()
        })
        .collect();
    times.sort();
    let millis = times[times.len() / 2].as_secs_f64() * 1.0e3;
    println!("{label:<32} {items:>9} {millis:>9.3}ms");
}

/// A weighted cubic of `controls` controls with uniform interior knots.
struct LongNurbs {
    knots: Vec<f64>,
    points: Vec<[f64; 3]>,
    weights: Vec<f64>,
}

impl LongNurbs {
    fn new(controls: u32) -> Self {
        let knots = (0..controls + 4)
            .map(|i| f64::from(i.clamp(3, controls)))
            .collect();
        let points = (0..controls)
            .map(|i| [f64::from(i), f64::from(i % 7), f64::from(i % 3)])
            .collect();
        let weights = (0..controls).map(|i| 1.0 + f64::from(i % 3)).collect();
        Self {
            knots,
            points,
            weights,
        }
    }

    fn curve(&self) -> Nurbs<'_> {
        Nurbs {
            order: 4,
            knots: &self.knots,
            points: &self.points,
            weights: Some(&self.weights),
        }
    }
}

/// Chains of cubic spans, each over its own controls.
struct Chain {
    controls: Vec<[f64; 3]>,
    spans: Vec<[u32; 4]>,
}

/// Many short curves of eight controls, and one long spiral chain.
struct ShortCurves {
    controls: Vec<Vec<[f64; 3]>>,
    spans: Vec<[u32; 4]>,
    long_chain: Chain,
}

impl ShortCurves {
    fn new(curves: u32) -> Self {
        let mut state = 0x9e37_79b9_u32;
        let controls = (0..curves)
            .map(|_| {
                (0..8)
                    .map(|i| {
                        [
                            f64::from(i) + next_unit(&mut state),
                            next_unit(&mut state),
                            next_unit(&mut state),
                        ]
                    })
                    .collect()
            })
            .collect();
        let spans = (0..5).map(|s| [s, s + 1, s + 2, s + 3]).collect();
        let spiral = 2000;
        let long_chain = Chain {
            controls: (0..spiral + 3)
                .map(|k| {
                    let a = f64::from(k) * 0.3;
                    let (s, c) = a.sin_cos();
                    [c * (1.0 + 0.01 * a), s * (1.0 + 0.01 * a), 0.02 * a]
                })
                .collect(),
            spans: (0..spiral).map(|s| [s, s + 1, s + 2, s + 3]).collect(),
        };
        Self {
            controls,
            spans,
            long_chain,
        }
    }
}

/// Buffers kept across curves, as a retained caller keeps them.
#[derive(Default)]
struct Buffers {
    workspace: Workspace,
    stations: Vec<Station>,
    points: Vec<[f64; 3]>,
    frames: Vec<Frame>,
    radii: Vec<f64>,
    mesh: SweptMesh,
}

impl Buffers {
    fn tube(&mut self, controls: &[[f64; 3]], spans: &[[u32; 4]]) {
        stations_into(5, 4, false, &mut self.stations);
        sample_into(
            CubicBasis::CatmullRom,
            controls,
            spans,
            &self.stations,
            &mut self.points,
        )
        .expect("the spans are in range");
        rotation_minimizing_frames_into(
            &self.points,
            None,
            false,
            None,
            &mut self.workspace,
            &mut self.frames,
        )
        .expect("no tangents are supplied");
        self.radii.clear();
        self.radii.resize(self.frames.len(), 0.05);
        tube_into(
            &self.frames,
            &self.radii,
            8,
            false,
            &mut self.workspace,
            &mut self.mesh,
        )
        .expect("the tube fits");
    }
}

/// A deterministic value in `[0, 1)`.
fn next_unit(state: &mut u32) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    f64::from(*state) / 4_294_967_296.0
}
