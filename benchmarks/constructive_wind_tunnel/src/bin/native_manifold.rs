// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase-separated retained-native machining, realization and support validation.
use exedra_analytic::{
    body::{Body, ValidationPolicy},
    extrusion::Extrusion,
    halfspace::{ClipPolicy, HalfSpace, intersect_half_space},
    machining::{
        BoreDepth, BoreReport, BoreStep, BoreTool, CylinderTool, MachiningPolicy, MachiningReport,
        PassageReport, subtract_bore, subtract_parallel_passages, subtract_through_cylinder,
        subtract_through_cylinders,
    },
    realize::{RealizationPolicy, RealizedBody},
};
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_math::Placement3;
use serde_json::{Value, json};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const CASES: [&str; 6] = [
    "nonparallel",
    "blind",
    "stepped",
    "overlap_pair",
    "overlap_chain",
    "plate_31",
];
#[derive(Clone)]
enum Tool {
    Through(CylinderTool),
    Bore(BoreTool),
    Union(Vec<CylinderTool>),
    Pattern(Vec<CylinderTool>),
}
struct Prepared {
    name: String,
    source: Body,
    tool: Tool,
    policy: MachiningPolicy,
}
enum History {
    Through(MachiningReport),
    Bore(BoreReport),
    Union(PassageReport),
    Pattern(Vec<MachiningReport>),
}
struct Output {
    body: Body,
    history: History,
}
impl Output {
    fn summary(&self) -> Value {
        let (removed, wall_fragments, pairs) = match &self.history {
            History::Through(r) => (r.removed_volume, 1, 0),
            History::Bore(r) => (r.removed_volume, r.walls.len(), 0),
            History::Union(r) => (r.removed_volume, r.walls.len(), r.pair_tests),
            History::Pattern(r) => (r.iter().map(|r| r.removed_volume).sum(), r.len(), 0),
        };
        json!({"faces":self.body.faces().len(),"edges":self.body.edges().len(),"vertices":self.body.vertices().len(),"trim_records":self.body.trims().len(),"wall_fragments":wall_fragments,"circle_pair_tests":pairs,"removed_volume":removed})
    }
}
fn prepare(name: &str) -> Prepared {
    let (min, max) = if name == "plate_31" {
        ([-364.0, -50.0], [364.0, 50.0])
    } else {
        ([-30.0, -20.0], [30.0, 20.0])
    };
    let outline = Loop::new(vec![
        Segment::line([max[0], min[1]]),
        Segment::line([max[0], max[1]]),
        Segment::line([min[0], max[1]]),
        Segment::line([min[0], min[1]]),
    ])
    .unwrap();
    let source = Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9)).unwrap(),
        Placement3::IDENTITY,
        20.0,
    )
    .unwrap();
    let mut body = source.body().clone().map_roles(|_| (), |_| ());
    let tool = match name {
        "nonparallel" => {
            body = intersect_half_space(
                &body,
                &HalfSpace::new([0.0, 0.0, 12.0], [-0.1, 0.0, 1.0]).unwrap(),
                &ClipPolicy::default(),
            )
            .unwrap()
            .body;
            Tool::Through(CylinderTool::new([0.0; 3], [0.2, 0.1, 1.0], 2.0).unwrap())
        }
        "blind" => Tool::Bore(
            BoreTool::new(
                [0.0; 3],
                [0.0, 0.0, 1.0],
                &[BoreStep {
                    radius: 5.0,
                    end: BoreDepth::ToDepth(7.0),
                }],
            )
            .unwrap(),
        ),
        "stepped" => Tool::Bore(
            BoreTool::new(
                [0.0; 3],
                [0.0, 0.0, 1.0],
                &[
                    BoreStep {
                        radius: 6.0,
                        end: BoreDepth::ToDepth(3.0),
                    },
                    BoreStep {
                        radius: 3.0,
                        end: BoreDepth::Through,
                    },
                ],
            )
            .unwrap(),
        ),
        "overlap_pair" => Tool::Union(
            [-2.0, 2.0]
                .into_iter()
                .map(|x| CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 3.0).unwrap())
                .collect(),
        ),
        "overlap_chain" => Tool::Union(
            [-5.0, 0.0, 5.0]
                .into_iter()
                .map(|x| CylinderTool::new([x, 0.0, 0.0], [0.0, 0.0, 1.0], 4.0).unwrap())
                .collect(),
        ),
        "plate_31" => Tool::Pattern(
            (0..31)
                .map(|i| {
                    CylinderTool::new(
                        [f64::from(i) * 22.0 - 330.0, 0.0, 0.0],
                        [0.1, 0.0, 1.0],
                        4.0,
                    )
                    .unwrap()
                })
                .collect(),
        ),
        _ => panic!("unknown workload {name}"),
    };
    Prepared {
        name: name.to_owned(),
        source: body,
        tool,
        policy: MachiningPolicy {
            geometry_gap: if name == "plate_31" { 364.0e-11 } else { 1e-9 },
            ..Default::default()
        },
    }
}
fn cut(input: &Prepared) -> Output {
    match &input.tool {
        Tool::Through(tool) => {
            let r =
                subtract_through_cylinder(black_box(&input.source), black_box(tool), &input.policy)
                    .unwrap();
            Output {
                body: r.body,
                history: History::Through(r.report),
            }
        }
        Tool::Bore(tool) => {
            let r =
                subtract_bore(black_box(&input.source), black_box(tool), &input.policy).unwrap();
            Output {
                body: r.body,
                history: History::Bore(r.report),
            }
        }
        Tool::Union(tools) => {
            let r = subtract_parallel_passages(
                black_box(&input.source),
                black_box(tools),
                &input.policy,
            )
            .unwrap();
            Output {
                body: r.body,
                history: History::Union(r.report),
            }
        }
        Tool::Pattern(tools) => {
            let r = subtract_through_cylinders(
                black_box(&input.source),
                black_box(tools),
                &input.policy,
            )
            .unwrap();
            Output {
                body: r.body,
                history: History::Pattern(r.operations),
            }
        }
    }
}
fn realize(body: &Body) -> RealizedBody {
    body.realize(
        Placement3::IDENTITY,
        &RealizationPolicy {
            tolerance: 0.03,
            geometry_gap: 4e-9,
            ..Default::default()
        },
    )
    .unwrap()
}
fn support_validation(body: &Body) {
    black_box(body.validate(&ValidationPolicy { max_gap: 4e-9 }).unwrap());
}
fn phase(name: &str, phase: &str, input: &Prepared, output: &Output) {
    match phase {
        "construct" => {
            black_box(prepare(name));
        }
        "cut" => {
            black_box(cut(input));
        }
        "realize" => {
            black_box(realize(&output.body));
        }
        "validate" => support_validation(&output.body),
        _ => panic!("unknown phase {phase}"),
    }
}
fn summarize(samples: &mut [f64]) -> Value {
    samples.sort_by(f64::total_cmp);
    let middle = samples[samples.len() / 2];
    json!({"median_ns":middle,"min_ns":samples[0],"max_ns":samples[samples.len()-1],"batch_ns":samples})
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|x| x == "--sample") {
        let name = args.get(1).map(String::as_str).unwrap_or("overlap_chain");
        let which = args.get(2).map(String::as_str).unwrap_or("cut");
        let seconds = args.get(3).map(|x| x.parse::<u64>().unwrap()).unwrap_or(30);
        let input = prepare(name);
        let output = cut(&input);
        println!(
            "{}",
            json!({"mode":"sample","pid":std::process::id(),"case":name,"phase":which,"seconds":seconds,"geometry":output.summary()})
        );
        let start = Instant::now();
        let mut operations = 0_u64;
        while start.elapsed() < Duration::from_secs(seconds) {
            phase(name, which, &input, &output);
            operations += 1;
        }
        println!(
            "{}",
            json!({"mode":"sample_finished","operations":operations,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0})
        );
        return;
    }
    let repeats = args
        .first()
        .map(|x| x.parse::<u32>().unwrap())
        .unwrap_or(100);
    let batches = args.get(1).map(|x| x.parse::<u32>().unwrap()).unwrap_or(7);
    assert!(repeats > 0 && batches > 0, "counts must be positive");
    let selected = args.get(2).map(String::as_str).unwrap_or("all");
    let mut results = Vec::new();
    for name in CASES
        .into_iter()
        .filter(|&name| selected == "all" || selected == name)
    {
        let input = prepare(name);
        let output = cut(&input);
        let mesh = realize(&output.body);
        assert!(
            mesh.report.within_tolerance(),
            "benchmark output must meet its requested tolerance"
        );
        assert!(
            mesh.mesh.boundary_loops().unwrap().is_empty(),
            "benchmark solid must remain closed"
        );
        let mut phases = serde_json::Map::new();
        for which in ["construct", "cut", "realize", "validate"] {
            for _ in 0..3 {
                phase(name, which, &input, &output);
            }
            let mut samples = Vec::with_capacity(batches as usize);
            for _ in 0..batches {
                let start = Instant::now();
                for _ in 0..repeats {
                    phase(name, which, &input, &output);
                }
                samples.push(start.elapsed().as_secs_f64() * 1e9 / f64::from(repeats));
            }
            phases.insert(which.into(), summarize(&mut samples));
        }
        results.push(json!({"case":input.name,"source_faces":input.source.faces().len(),"geometry":output.summary(),"mesh_triangles":mesh.mesh.faces().count(),"mesh_vertices":mesh.mesh.vertices().count(),"max_deviation":mesh.report.max_deviation,"tolerance":mesh.report.tolerance,"phases":phases}));
    }
    assert!(!results.is_empty(), "unknown case {selected}");
    println!("{}",serde_json::to_string_pretty(&json!({"workload":"native_manifold","roles":"unit metadata in all new workloads; original native_machining FaceRole/EdgeRole benchmark remains unchanged","repetitions_per_batch":repeats,"batches":batches,"timing":"native host release, prepared inputs excluded from cut/realize/validate, source and tool construction included in construct; output destruction included; validation means local native topology/support/explicit trims, not global solid proof; no STEP/compiler/JS/Wasm/GPU","results":results})).unwrap());
}

/// Isolated dhat window selected by `EXEDRA_MANIFOLD_CASE` and
/// `EXEDRA_MANIFOLD_PHASE` (cut/realize/validate). Prepared inputs and warmup are
/// excluded; output plus report survive until retained, then all are released.
pub fn allocation_window(mut checkpoint: impl FnMut(&str)) {
    let name = std::env::var("EXEDRA_MANIFOLD_CASE").unwrap_or_else(|_| "overlap_chain".into());
    let which = std::env::var("EXEDRA_MANIFOLD_PHASE").unwrap_or_else(|_| "cut".into());
    let input = prepare(&name);
    let output = cut(&input);
    phase(&name, &which, &input, &output);
    checkpoint("ready");
    match which.as_str() {
        "cut" => {
            let value = cut(&input);
            checkpoint("retained");
            drop(value);
        }
        "realize" => {
            let value = realize(&output.body);
            checkpoint("retained");
            drop(value);
        }
        "validate" => {
            let value = output
                .body
                .validate(&ValidationPolicy { max_gap: 4e-9 })
                .unwrap();
            checkpoint("retained");
            black_box(value);
        }
        _ => panic!("allocation phase must be cut, realize or validate"),
    }
    checkpoint("released");
}
