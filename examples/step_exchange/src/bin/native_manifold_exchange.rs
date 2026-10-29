// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Interacting native machining fixtures for independent STEP material checks.
use exedra_analytic::{
    body::{Body, EdgeId, VertexId},
    extrusion::Extrusion,
    halfspace::{ClipPolicy, HalfSpace, intersect_half_space},
    machining::{
        BoreDepth, BoreStep, BoreTool, CylinderTool, MachiningPolicy, subtract_bore,
        subtract_parallel_passages, subtract_through_cylinder,
    },
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_step::{StepOptions, export_body};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    error::Error,
    path::{Path, PathBuf},
};

fn stock() -> Result<Extrusion, Box<dyn Error>> {
    let outer = Loop::new(vec![
        Segment::line([30.0, -20.0]),
        Segment::line([30.0, 20.0]),
        Segment::line([-30.0, 20.0]),
        Segment::line([-30.0, -20.0]),
    ])?;
    Ok(Extrusion::new(
        Profile::new(outer, vec![], &ProfilePolicy::new(1e-9))?,
        Placement3::IDENTITY,
        20.0,
    )?)
}
fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "native-manifold-fixtures".into()),
    );
    std::fs::create_dir_all(&output)?;
    let source = stock()?;
    let body = source.body().clone().map_roles(|_| (), |_| ());
    let policy = MachiningPolicy::default();
    let mut fixtures = Vec::new();
    let wedge = intersect_half_space(
        &body,
        &HalfSpace::new([0.0, 0.0, 12.0], [-0.1, 0.0, 1.0])?,
        &ClipPolicy::default(),
    )?
    .body;
    let wedge_volume = 28800.0;
    let wedge_area = 4800.0 + 2400.0 * 1.01_f64.sqrt();
    for (name, direction) in [
        ("nonparallel-exits", [0.2, 0.1, 1.0]),
        ("nonparallel-reversed", [-0.2, -0.1, -1.0]),
    ] {
        let result = subtract_through_cylinder(
            &wedge,
            &CylinderTool::new([0.0; 3], direction, 2.0)?,
            &policy,
        )?;
        save(
            &output,
            &mut fixtures,
            name,
            &result.body,
            Placement3::IDENTITY,
            wedge_volume - result.report.removed_volume,
            wedge_area + result.report.surface_area_delta,
            &[
                ([1.2, 0.6, 6.0], "outside"),
                ([8.0, 0.0, 6.0], "inside"),
                ([0.0, 0.0, 13.0], "outside"),
            ],
        )?;
    }
    for (name, entry, direction, steps, placement) in [
        (
            "blind-floor",
            [0.0; 3],
            [0.0, 0.0, 1.0],
            vec![BoreStep {
                radius: 5.0,
                end: BoreDepth::ToDepth(7.0),
            }],
            Placement3::IDENTITY,
        ),
        (
            "stepped-blind",
            [0.0; 3],
            [0.0, 0.0, 1.0],
            vec![
                BoreStep {
                    radius: 6.0,
                    end: BoreDepth::ToDepth(3.0),
                },
                BoreStep {
                    radius: 3.0,
                    end: BoreDepth::ToDepth(8.0),
                },
            ],
            Placement3::IDENTITY,
        ),
        (
            "counterbore-through",
            [0.0; 3],
            [0.0, 0.0, 1.0],
            vec![
                BoreStep {
                    radius: 6.0,
                    end: BoreDepth::ToDepth(3.0),
                },
                BoreStep {
                    radius: 3.0,
                    end: BoreDepth::Through,
                },
            ],
            Placement3::rotate_y_then_translate(0.4, 120.0, -80.0, 45.0),
        ),
        (
            "stepped-reversed",
            [0.0, 0.0, 20.0],
            [0.0, 0.0, -1.0],
            vec![
                BoreStep {
                    radius: 6.0,
                    end: BoreDepth::ToDepth(3.0),
                },
                BoreStep {
                    radius: 3.0,
                    end: BoreDepth::ToDepth(8.0),
                },
            ],
            Placement3::translate(-70.0, 25.0, 100.0),
        ),
    ] {
        let result = subtract_bore(&body, &BoreTool::new(entry, direction, &steps)?, &policy)?;
        let mut witnesses = vec![
            (local(entry, direction, [0.0, 0.0, 1.0]), "outside"),
            (local(entry, direction, [8.0, 0.0, 5.0]), "inside"),
        ];
        if steps.len() > 1 {
            witnesses.extend([
                (local(entry, direction, [4.5, 0.0, 2.0]), "outside"),
                (local(entry, direction, [4.5, 0.0, 4.0]), "inside"),
                (local(entry, direction, [0.0, 0.0, 6.0]), "outside"),
            ]);
        }
        if matches!(steps.last().unwrap().end, BoreDepth::ToDepth(_)) {
            let BoreDepth::ToDepth(depth) = steps.last().unwrap().end else {
                unreachable!()
            };
            witnesses.extend([
                (local(entry, direction, [0.0, 0.0, depth - 0.1]), "outside"),
                (local(entry, direction, [0.0, 0.0, depth + 0.1]), "inside"),
            ]);
        }
        save(
            &output,
            &mut fixtures,
            name,
            &result.body,
            placement,
            source.volume() - result.report.removed_volume,
            source.surface_area() + result.report.surface_area_delta,
            &witnesses,
        )?;
    }
    for (name, positions) in [
        (
            "joined-parallel-passages",
            vec![([-2.0, 0.0], 3.0), ([2.0, 0.0], 3.0)],
        ),
        (
            "joined-three-passages",
            vec![([-5.0, 0.0], 4.0), ([0.0, 0.0], 4.0), ([5.0, 0.0], 4.0)],
        ),
        (
            "joined-covered-arcs",
            vec![([-3.0, 0.0], 4.0), ([3.0, 0.0], 4.0), ([0.0, 5.0], 4.0)],
        ),
    ] {
        let tools = positions
            .iter()
            .map(|&(p, r)| CylinderTool::new([p[0], p[1], 0.0], [0.0, 0.0, 1.0], r))
            .collect::<Result<Vec<_>, _>>()?;
        let result = subtract_parallel_passages(&body, &tools, &policy)?;
        save(
            &output,
            &mut fixtures,
            name,
            &result.body,
            Placement3::IDENTITY,
            source.volume() - result.report.removed_volume,
            source.surface_area() + result.report.surface_area_delta,
            &[
                ([0.0, 0.0, 10.0], "outside"),
                ([-2.0, 0.0, 10.0], "outside"),
                ([0.0, -10.0, 10.0], "inside"),
                ([12.0, 0.0, 10.0], "inside"),
            ],
        )?;
    }
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_vec_pretty(&fixtures)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
fn local(entry: [f64; 3], direction: [f64; 3], point: [f64; 3]) -> [f64; 3] {
    [
        entry[0] + point[0],
        entry[1] + point[1],
        entry[2] + direction[2] * point[2],
    ]
}
fn save(
    output: &Path,
    fixtures: &mut Vec<Value>,
    name: &str,
    body: &Body,
    placement: Placement3,
    volume: f64,
    area: f64,
    witnesses: &[([f64; 3], &str)],
) -> Result<(), Box<dyn Error>> {
    let name = format!("native-{name}");
    let options = StepOptions {
        name: &name,
        ..Default::default()
    };
    let doc = export_body(body, placement, &options)?;
    let file = format!("{name}.step");
    std::fs::write(output.join(&file), &doc.text)?;
    let mut edges = BTreeSet::<EdgeId>::new();
    let mut vertices = BTreeSet::<VertexId>::new();
    for face in body.faces() {
        for l in &face.loops {
            for co in &body.loops()[l.index()].coedges {
                edges.insert(co.edge);
                let edge = &body.edges()[co.edge.index()];
                vertices.insert(edge.start);
                vertices.insert(edge.end);
            }
        }
    }
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for &id in &vertices {
        let p = placement.transform_point(body.vertices()[id.index()]);
        for i in 0..3 {
            bounds[0][i] = bounds[0][i].min(p[i]);
            bounds[1][i] = bounds[1][i].max(p[i]);
        }
    }
    let witnesses: Vec<_> = witnesses
        .iter()
        .map(|&(p, state)| json!({"point":placement.transform_point(p),"state":state}))
        .collect();
    fixtures.push(json!({"file":file,"product_name":name,"faces":body.faces().len(),"edges":doc.report.counts.edges,"vertices":doc.report.counts.vertices,"native_edges":edges.len(),"native_vertices":vertices.len(),"metres_per_unit":0.001,"native_volume":volume,"native_area":area,"bounds":bounds,"uncertainty":options.tolerance,"max_deviation":doc.report.max_deviation,"max_native_support_gap":doc.report.max_native_support_gap,"frame_roundoff_bound":doc.report.frame_roundoff_bound,"parameter_roundoff_bound":doc.report.parameter_roundoff_bound,"witnesses":witnesses}));
    Ok(())
}
