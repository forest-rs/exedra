// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native multi-operation machining fixtures for the independent STEP oracle.
use exedra_analytic::{
    extrusion::Extrusion,
    machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder_with_roles},
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_step::{StepOptions, export_body};
use serde_json::json;
use std::{error::Error, path::PathBuf};
fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "native-machining-fixtures".into()),
    );
    std::fs::create_dir_all(&output)?;
    let circle = |r: f64| {
        Loop::new(vec![
            Segment::arc([-r, 0.0], 1.0),
            Segment::arc([r, 0.0], 1.0),
        ])
        .unwrap()
    };
    let rectangle = Loop::new(vec![
        Segment::line([30.0, -20.0]),
        Segment::line([30.0, 20.0]),
        Segment::line([-30.0, 20.0]),
        Segment::line([-30.0, -20.0]),
    ])?;
    let mut fixtures = Vec::new();
    for (name, outer, holes, tools, bounds, witnesses) in [
        (
            "native-hole-pattern",
            rectangle.clone(),
            vec![],
            vec![
                ([-15.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                ([15.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ],
            [[-30.0, -20.0, 0.0], [30.0, 20.0, 10.0]],
            json!([{"point":[0.0,0.0,5.0],"state":"outside"},{"point":[-15.0,0.0,5.0],"state":"outside"},{"point":[0.0,10.0,5.0],"state":"inside"}]),
        ),
        (
            "native-oblique-passage",
            rectangle,
            vec![],
            vec![([0.0, 0.0, 0.0], [0.3, 0.1, 1.0])],
            [[-30.0, -20.0, 0.0], [30.0, 20.0, 10.0]],
            json!([{"point":[1.5,0.5,5.0],"state":"outside"},{"point":[0.0,10.0,5.0],"state":"inside"}]),
        ),
        (
            "native-annular-stock",
            circle(30.0),
            vec![circle(10.0).reversed()],
            vec![([20.0, 0.0, 0.0], [0.0, 0.0, 1.0])],
            [[-30.0, -30.0, 0.0], [30.0, 30.0, 10.0]],
            json!([{"point":[20.0,0.0,5.0],"state":"outside"},{"point":[0.0,0.0,5.0],"state":"outside"},{"point":[0.0,20.0,5.0],"state":"inside"}]),
        ),
    ] {
        let source = Extrusion::new(
            Profile::new(outer, holes, &ProfilePolicy::new(1e-9))?,
            Placement3::IDENTITY,
            10.0,
        )?;
        let mut volume = source.volume();
        let mut area = source.surface_area();
        let mut body = source.body().clone();
        for (point, direction) in tools {
            let result = subtract_through_cylinder_with_roles(
                &body,
                &CylinderTool::new(point, direction, 2.0)?,
                &MachiningPolicy::default(),
                source.body().faces()[0].role,
                source.body().edges()[0].role,
            )?;
            volume -= result.report.removed_volume;
            area += result.report.surface_area_delta;
            body = result.body;
        }
        let options = StepOptions {
            name,
            ..Default::default()
        };
        let doc = export_body(&body, Placement3::IDENTITY, &options)?;
        let file = format!("{name}.step");
        std::fs::write(output.join(&file), &doc.text)?;
        fixtures.push(json!({"file":file,"product_name":name,"faces":body.faces().len(),"edges":body.edges().len(),"vertices":body.vertices().len(),"metres_per_unit":0.001,"native_volume":volume,"native_area":area,"bounds":bounds,"uncertainty":options.tolerance,"max_deviation":options.tolerance,"witnesses":witnesses}));
    }
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_string_pretty(&fixtures)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
