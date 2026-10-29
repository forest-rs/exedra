// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Retained native Boolean/lug-machining STEP fixtures for the independent oracle.
use exedra_analytic::{
    body::{Body, ValidationPolicy},
    extrusion::{EdgeRole, Extrusion, FaceRole},
    geometry::{Curve, Surface},
    machining::{CylinderTool, MachiningPolicy, subtract_through_cylinder},
    rectilinear::{BooleanOperation, RectilinearPolicy, boolean_rectilinear},
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, Segment};
use exedra_step::{StepOptions, export_body};
use serde_json::json;
use std::{error::Error, path::PathBuf};
fn block(min: [f64; 3], max: [f64; 3]) -> Body<FaceRole, EdgeRole> {
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
        max[2] - min[2],
    )
    .unwrap();
    let mut parts = source.body().clone().into_parts();
    for p in &mut parts.vertices {
        p[2] += min[2];
    }
    for edge in &mut parts.edges {
        let (curve, _) = edge.regular_mut().unwrap();
        let Curve::Line(line) = curve else {
            unreachable!()
        };
        let mut origin = line.origin();
        origin[2] += min[2];
        *curve = Curve::line(origin, line.direction()).unwrap();
    }
    for face in &mut parts.faces {
        let mut frame = face.surface.frame();
        frame.rows[2][3] += min[2];
        face.surface = Surface::plane(frame).unwrap();
    }
    Body::try_from_parts(parts, &ValidationPolicy { max_gap: 1e-9 }).unwrap()
}
fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "native-rectilinear-fixtures".into()),
    );
    std::fs::create_dir_all(&output)?;
    let base = block([-20.0, -15.0, 0.0], [20.0, 15.0, 10.0]);
    let lug = block([20.0, -5.0, 0.0], [35.0, 5.0, 10.0]);
    let union = boolean_rectilinear(
        &base,
        &lug,
        BooleanOperation::Union,
        &RectilinearPolicy::default(),
    )?;
    let drilled = subtract_through_cylinder(
        &union.body,
        &CylinderTool::new([27.5, 0.0, 0.0], [0.0, 0.0, 1.0], 2.0)?,
        &MachiningPolicy::default(),
    )?;
    let cutter = block([-5.0, -4.0, -2.0], [5.0, 4.0, 12.0]);
    let difference = boolean_rectilinear(
        &base,
        &cutter,
        BooleanOperation::Difference,
        &RectilinearPolicy::default(),
    )?;
    let a = block([-10.0, -8.0, 0.0], [10.0, 8.0, 6.0]);
    let b = block([-3.0, -5.0, -2.0], [12.0, 10.0, 4.0]);
    let intersection = boolean_rectilinear(
        &a,
        &b,
        BooleanOperation::Intersection,
        &RectilinearPolicy::default(),
    )?;
    let closed_cavity = boolean_rectilinear(
        &base,
        &block([-5.0, -4.0, 2.0], [5.0, 4.0, 8.0]),
        BooleanOperation::Difference,
        &RectilinearPolicy::default(),
    )?;
    let first_cavity = boolean_rectilinear(
        &union.body,
        &block([-12.0, -8.0, 2.0], [-6.0, -2.0, 8.0]),
        BooleanOperation::Difference,
        &RectilinearPolicy::default(),
    )?;
    let multiple_cavities = boolean_rectilinear(
        &first_cavity.body,
        &block([6.0, 2.0, 2.0], [12.0, 8.0, 8.0]),
        BooleanOperation::Difference,
        &RectilinearPolicy::default(),
    )?;
    let mut fixtures = Vec::new();
    for (name, body, volume, area, bounds, witnesses, shells) in [
        (
            "native-union-lug",
            union.body,
            13500.0,
            4400.0,
            [[-20.0, -15.0, 0.0], [35.0, 15.0, 10.0]],
            json!([{"point":[27.5,0.0,5.0],"state":"inside"},{"point":[27.5,10.0,5.0],"state":"outside"}]),
            1,
        ),
        (
            "native-machined-lug",
            drilled.body,
            13500.0 - drilled.report.removed_volume,
            4400.0 + drilled.report.surface_area_delta,
            [[-20.0, -15.0, 0.0], [35.0, 15.0, 10.0]],
            json!([{"point":[27.5,0.0,5.0],"state":"outside"},{"point":[25.0,0.0,5.0],"state":"inside"}]),
            1,
        ),
        (
            "native-box-passage",
            difference.body,
            11200.0,
            4000.0,
            [[-20.0, -15.0, 0.0], [20.0, 15.0, 10.0]],
            json!([{"point":[0.0,0.0,5.0],"state":"outside"},{"point":[10.0,0.0,5.0],"state":"inside"}]),
            1,
        ),
        (
            "native-solid-intersection",
            intersection.body,
            676.0,
            546.0,
            [[-3.0, -5.0, 0.0], [10.0, 8.0, 4.0]],
            json!([{"point":[0.0,0.0,2.0],"state":"inside"},{"point":[0.0,0.0,5.0],"state":"outside"}]),
            1,
        ),
        (
            "native-closed-cavity",
            closed_cavity.body,
            11520.0,
            4176.0,
            [[-20.0, -15.0, 0.0], [20.0, 15.0, 10.0]],
            json!([{"point":[0.0,0.0,5.0],"state":"outside"},{"point":[10.0,0.0,5.0],"state":"inside"}]),
            2,
        ),
        (
            "native-housing-two-cavities",
            multiple_cavities.body,
            13068.0,
            4832.0,
            [[-20.0, -15.0, 0.0], [35.0, 15.0, 10.0]],
            json!([{"point":[-9.0,-5.0,5.0],"state":"outside"},{"point":[9.0,5.0,5.0],"state":"outside"},{"point":[0.0,0.0,5.0],"state":"inside"},{"point":[27.5,0.0,5.0],"state":"inside"}]),
            3,
        ),
    ] {
        let options = StepOptions {
            name,
            ..Default::default()
        };
        let certification = if shells > 1 {
            let start = std::time::Instant::now();
            let certificate =
                exedra_analytic::rectilinear::validate_material(&body, &options.shell_validation)?;
            Some(
                json!({"elapsedNs":start.elapsed().as_nanos(),"cells":certificate.cells,
                "faceTests":certificate.face_tests,"planeVertices":certificate.plane_vertices,
                "linkBytes":certificate.link_bytes,"arrangementScratchBytes":certificate.arrangement_scratch_bytes}),
            )
        } else {
            None
        };
        let start = std::time::Instant::now();
        let doc = export_body(&body, Placement3::IDENTITY, &options)?;
        let export_ns = start.elapsed().as_nanos();
        assert_eq!(
            doc.report.counts.shells, shells,
            "each certified boundary shell must be preserved"
        );
        println!(
            "{name}: certificate={certification:?}, export_including_certificate_ns={export_ns}"
        );
        let file = format!("{name}.step");
        std::fs::write(output.join(&file), doc.text)?;
        fixtures.push(json!({"file":file,"product_name":name,"shells":shells,"certification":certification,"export_including_certificate_ns":export_ns,"faces":body.faces().len(),"edges":body.edges().len(),"vertices":body.vertices().len(),"metres_per_unit":0.001,"native_volume":volume,"native_area":area,"bounds":bounds,"uncertainty":options.tolerance,"max_deviation":doc.report.max_deviation,"max_native_support_gap":doc.report.max_native_support_gap,"frame_roundoff_bound":doc.report.frame_roundoff_bound,"parameter_roundoff_bound":doc.report.parameter_roundoff_bound,"witnesses":witnesses}));
    }
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_string_pretty(&fixtures)?,
    )?;
    println!("{}", output.display());
    Ok(())
}
