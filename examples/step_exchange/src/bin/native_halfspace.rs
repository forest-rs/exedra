// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independent analytic halfspace STEP fixtures with closed-form measures.
use exedra_analytic::{
    extrusion::Extrusion,
    halfspace::{ClipPolicy, HalfSpace, intersect_half_space_with_roles},
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_step::{StepOptions, export_body};
use serde_json::json;
use std::{collections::BTreeSet, error::Error, path::PathBuf};
fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "native-halfspace-fixtures".into()),
    );
    std::fs::create_dir_all(&output)?;
    let outline = Loop::new(vec![
        Segment::line([10.0, -8.0]),
        Segment::line([10.0, 8.0]),
        Segment::line([-10.0, 8.0]),
        Segment::line([-10.0, -8.0]),
    ])?;
    let source = Extrusion::new(
        Profile::new(outline, vec![], &ProfilePolicy::new(1e-9))?,
        Placement3::IDENTITY,
        6.0,
    )?;
    let mut fixtures = Vec::new();
    for (name, point, normal, volume, area, bounds) in [
        (
            "native-axis-clip",
            [3.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            1248.0,
            764.0,
            [[-10.0, -8.0, 0.0], [3.0, 8.0, 6.0]],
        ),
        (
            "native-oblique-clip",
            [0.0; 3],
            [1.0, 1.0, 0.0],
            960.0,
            320.0 + 6.0 * (36.0 + 16.0 * 2.0_f64.sqrt()),
            [[-10.0, -8.0, 0.0], [8.0, 8.0, 6.0]],
        ),
        (
            "native-trihedral-clip",
            [0.0, 0.0, 3.0],
            [1.0, 1.0, 1.0],
            960.0,
            536.0 + 95.0 * 3.0_f64.sqrt(),
            [[-10.0, -8.0, 0.0], [10.0, 8.0, 6.0]],
        ),
    ] {
        let result = intersect_half_space_with_roles(
            source.body(),
            &HalfSpace::new(point, normal)?,
            &ClipPolicy::default(),
            source.body().faces()[0].role,
            source.body().edges()[0].role,
        )?;
        let body = result.body;
        let mut edges = BTreeSet::new();
        let mut vertices = BTreeSet::new();
        for face in body.faces() {
            for id in &face.loops {
                for co in &body.loops()[id.index()].coedges {
                    edges.insert(co.edge);
                    let edge = &body.edges()[co.edge.index()];
                    vertices.insert(edge.start);
                    vertices.insert(edge.end);
                }
            }
        }
        let options = StepOptions {
            name,
            ..Default::default()
        };
        let doc = export_body(&body, Placement3::IDENTITY, &options)?;
        let file = format!("{name}.step");
        std::fs::write(output.join(&file), doc.text)?;
        fixtures.push(json!({"file":file,"product_name":name,"faces":body.faces().len(),"edges":edges.len(),"vertices":vertices.len(),"metres_per_unit":0.001,"native_volume":volume,"native_area":area,"bounds":bounds,"uncertainty":options.tolerance,"max_deviation":doc.report.max_deviation,"max_native_support_gap":doc.report.max_native_support_gap,"frame_roundoff_bound":doc.report.frame_roundoff_bound,"parameter_roundoff_bound":doc.report.parameter_roundoff_bound,"witnesses":[{"point":[-9.0,0.0,3.0],"state":"inside"},{"point":[9.0,0.0,3.0],"state":"outside"}]}));
    }
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_string_pretty(&fixtures)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
