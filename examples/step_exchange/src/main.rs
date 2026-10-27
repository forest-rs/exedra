// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independently readable native STEP fixtures with exact reference measures.

use exedra_analytic::{
    End,
    extrusion::{Extrusion, Pocket},
};
use exedra_math::Placement3;
use exedra_profile::{Loop, Profile, ProfilePolicy, SegTag, Segment};
use exedra_step::{LengthUnit, StepOptions, export_body};
use std::{error::Error, path::PathBuf};

const POLICY: ProfilePolicy = ProfilePolicy::new(1e-9);
fn rectangle(x: f64, y: f64) -> Loop {
    Loop::new(vec![
        Segment::line([x, -y]),
        Segment::line([x, y]),
        Segment::line([-x, y]),
        Segment::line([-x, -y]),
    ])
    .unwrap()
}
fn disk(x: f64, y: f64, r: f64) -> Loop {
    Loop::new(vec![
        Segment::arc([x - r, y], 1.0),
        Segment::arc([x + r, y], 1.0),
    ])
    .unwrap()
}
fn solid(outline: Loop, holes: Vec<Loop>, length: f64) -> Extrusion {
    Extrusion::new(
        Profile::new(outline, holes, &POLICY).unwrap(),
        Placement3::IDENTITY,
        length,
    )
    .unwrap()
}
fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "step-fixtures".into()),
    );
    std::fs::create_dir_all(&output)?;
    let block = solid(rectangle(10.0, 8.0), vec![], 6.0);
    let disk_solid = solid(disk(0.0, 0.0, 5.0), vec![], 6.0);
    let ring = solid(
        disk(0.0, 0.0, 5.0),
        vec![disk(0.0, 0.0, 2.0).reversed()],
        6.0,
    );
    let panel = solid(
        rectangle(10.0, 8.0),
        vec![disk(5.0, 0.0, 1.0).reversed()],
        6.0,
    )
    .with_pocket(
        Pocket::from_region(&disk(-3.0, 0.0, 2.0), End::End, 2.0).tagged(SegTag(71)),
        &POLICY,
    )?;
    let quarter = std::f64::consts::PI * 0.75;
    let sector = solid(
        Loop::new(vec![
            Segment::arc([0.0, -5.0], (quarter * 0.5).tan()),
            Segment::line([0.0, 0.0]),
            Segment::line([5.0, 0.0]),
        ])?,
        vec![],
        6.0,
    );
    let translation = Placement3::translate(100.0, -30.0, 7.0);
    let fixtures = [
        (
            "block",
            block,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-10.0, -8.0, 0.0], [10.0, 8.0, 6.0]],
        ),
        (
            "disk",
            disk_solid,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-5.0, -5.0, 0.0], [5.0, 5.0, 6.0]],
        ),
        (
            "ring",
            ring,
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-5.0, -5.0, 0.0], [5.0, 5.0, 6.0]],
        ),
        (
            "pocketed-panel",
            panel.clone(),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-10.0, -8.0, 0.0], [10.0, 8.0, 6.0]],
        ),
        (
            "translated-panel",
            panel,
            translation,
            LengthUnit::Millimetre,
            [[90.0, -38.0, 7.0], [110.0, -22.0, 13.0]],
        ),
        (
            "large-arc-sector",
            sector,
            Placement3::IDENTITY,
            LengthUnit::Metre,
            [[-5.0, -5.0, 0.0], [5.0, 5.0, 6.0]],
        ),
    ];
    let mut expected = Vec::new();
    for (name, solid, placement, units, bounds) in fixtures {
        let options = StepOptions {
            name,
            timestamp: "2026-01-01T00:00:00Z",
            units,
            ..StepOptions::default()
        };
        let document = export_body(solid.body(), placement, &options)?;
        let file = format!("{name}.step");
        std::fs::write(output.join(&file), &document.text)?;
        expected.push(serde_json::json!({"file":file,"native_volume":solid.volume(),"native_area":solid.surface_area(),"bounds":bounds,
            "metres_per_unit":units.metres_per_unit(),"faces":document.report.counts.faces,"edges":document.report.counts.edges,
            "vertices":document.report.counts.vertices,"max_deviation":document.report.max_deviation,"uncertainty":document.report.declared_uncertainty}));
        println!(
            "{name}: {} faces, {} shared edges, {} entities, {} bytes",
            document.report.counts.faces,
            document.report.counts.edges,
            document.report.counts.entities,
            document.text.len()
        );
    }
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_vec_pretty(&expected)?,
    )?;
    Ok(())
}
