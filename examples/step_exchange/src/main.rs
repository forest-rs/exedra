// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Independently readable native STEP fixtures with exact reference measures.

use exedra_analytic::{
    End,
    extrusion::{Extrusion, Pocket},
    loft::{Loft, LoftPolicy},
};
use exedra_curves::profile::{Loop, Profile, ProfilePolicy, SegTag, Segment};
use exedra_math::Placement3;
use exedra_step::{LengthUnit, StepDocument, StepOptions, export_body};
use std::{error::Error, path::PathBuf};

mod periodic;
mod poles;
mod supports;

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
            block.clone(),
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-10.0, -8.0, 0.0], [10.0, 8.0, 6.0]],
        ),
        (
            "rotated-block",
            block,
            Placement3::from_axes(
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [10.0, 20.0, 30.0],
            ),
            LengthUnit::Millimetre,
            [[10.0, 12.0, 20.0], [16.0, 28.0, 40.0]],
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
        save(
            &output,
            &mut expected,
            name,
            &document,
            solid.volume(),
            solid.surface_area(),
            bounds,
        )?;
        println!(
            "{name}: {} faces, {} shared edges, {} entities, {} bytes",
            document.report.counts.faces,
            document.report.counts.edges,
            document.report.counts.entities,
            document.text.len()
        );
    }
    for (name, placement, units, bounds) in [
        (
            "oblique-bore",
            Placement3::IDENTITY,
            LengthUnit::Millimetre,
            [[-20.0, -15.0, 0.0], [20.0, 15.0, 10.0]],
        ),
        (
            "placed-oblique-bore",
            Placement3::from_axes(
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
                [1.0, 0.0, 0.0],
                [100.0, -30.0, 7.0],
            ),
            LengthUnit::Metre,
            [[100.0, -45.0, -13.0], [110.0, -15.0, 27.0]],
        ),
    ] {
        let plate = solid(rectangle(20.0, 15.0), vec![], 10.0);
        let bored = plate.with_through_bore(
            exedra_analytic::bore::Bore::new([0.0; 3], [0.5, 0.25, 1.0], 2.0)?.tagged(SegTag(91)),
            &exedra_analytic::bore::BorePolicy {
                separation: 1e-6,
                geometry_gap: 1e-9,
                min_axis_dot: 0.1,
            },
        )?;
        let document = export_body(
            bored.body(),
            placement,
            &StepOptions {
                name,
                units,
                timestamp: "2026-01-01T00:00:00Z",
                ..Default::default()
            },
        )?;
        save(
            &output,
            &mut expected,
            name,
            &document,
            bored.volume(),
            bored.surface_area(),
            bounds,
        )?;
        expected.last_mut().unwrap()["witnesses"] = serde_json::json!([
            {"point":placement.transform_point([2.5,1.25,5.0]),"state":"outside"},
            {"point":placement.transform_point([15.0,10.0,5.0]),"state":"inside"},
            {"point":placement.transform_point([0.0,0.0,-2.0]),"state":"outside"}
        ]);
    }
    let body = periodic::cylinder();
    let unicode_name = "Café \\ Ω ' 🌀";
    let document = export_body(
        &body,
        Placement3::IDENTITY,
        &StepOptions {
            name: unicode_name,
            ..StepOptions::default()
        },
    )?;
    save(
        &output,
        &mut expected,
        "periodic-cylinder",
        &document,
        20.0 * std::f64::consts::PI,
        28.0 * std::f64::consts::PI,
        [[-2.0, -2.0, 0.0], [2.0, 2.0, 5.0]],
    )?;
    expected
        .last_mut()
        .expect("periodic fixture was just saved")["product_name"] =
        serde_json::json!(unicode_name);
    let torus = periodic::torus();
    let document = export_body(
        &torus,
        Placement3::IDENTITY,
        &StepOptions {
            name: "double-periodic-torus",
            ..StepOptions::default()
        },
    )?;
    save(
        &output,
        &mut expected,
        "double-periodic-torus",
        &document,
        80.0 * std::f64::consts::PI.powi(2),
        80.0 * std::f64::consts::PI.powi(2),
        [[-12.0, -12.0, -2.0], [12.0, 12.0, 2.0]],
    )?;
    supports::export_cases(&output, &mut expected)?;
    export_lofts(&output, &mut expected)?;
    poles::export_cases(&output, &mut expected)?;
    poles::export_constructed(&output, &mut expected)?;
    poles::export_caps(&output, &mut expected)?;
    std::fs::write(
        output.join("expected.json"),
        serde_json::to_vec_pretty(&expected)?,
    )?;
    if std::env::args().any(|arg| arg == "--bench") {
        benchmark(&output)?;
    }
    Ok(())
}

fn benchmark(output: &std::path::Path) -> Result<(), Box<dyn Error>> {
    let mut report = Vec::new();
    for side in [0_u32, 2, 8, 16] {
        let holes = (0..side)
            .flat_map(|y| {
                (0..side).map(move |x| {
                    disk(f64::from(x) * 4.0 - 30.0, f64::from(y) * 4.0 - 30.0, 1.0).reversed()
                })
            })
            .collect();
        // Authoring/preparation and output-file I/O are deliberately outside timing.
        let solid = solid(rectangle(34.0, 34.0), holes, 6.0);
        let options = StepOptions::default();
        let warm = export_body(solid.body(), Placement3::IDENTITY, &options)?;
        let mut timings = Vec::new();
        for _ in 0..21 {
            let start = std::time::Instant::now();
            let document = std::hint::black_box(export_body(
                std::hint::black_box(solid.body()),
                Placement3::IDENTITY,
                &options,
            )?);
            timings.push(start.elapsed().as_secs_f64() * 1e6);
            assert_eq!(
                document.text.len(),
                warm.text.len(),
                "exported byte count must be deterministic during timing"
            );
        }
        timings.sort_by(f64::total_cmp);
        let value = serde_json::json!({"holes":side*side,"faces":warm.report.counts.faces,"entities":warm.report.counts.entities,"bytes":warm.text.len(),"median_export_us":timings[10],"min_export_us":timings[0],"iterations":timings.len()});
        println!("{value}");
        report.push(value);
    }
    std::fs::write(
        output.join("benchmark.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Lofts between squares and between circles: walls ruled between lines
/// and between arcs of equal sweep, written as B-spline surfaces, with
/// closed-form volumes and areas.
fn export_lofts(
    output: &std::path::Path,
    expected: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn Error>> {
    let policy = LoftPolicy::new(POLICY);
    let square = |h: f64| -> Result<Profile, Box<dyn Error>> {
        Ok(Profile::new(rectangle(h, h), vec![], &POLICY)?)
    };
    let circle = |r: f64| -> Result<Profile, Box<dyn Error>> {
        let bulge = std::f64::consts::FRAC_PI_8.tan();
        let ring = Loop::new(vec![
            Segment::arc([r, 0.0], bulge),
            Segment::arc([0.0, r], bulge),
            Segment::arc([-r, 0.0], bulge),
            Segment::arc([0.0, -r], bulge),
        ])?;
        Ok(Profile::new(ring, vec![], &POLICY)?)
    };
    let height = 20.0;
    let top = Placement3::translate(0.0, 0.0, height);
    // Frustum walls slope across 5 mm while rising 20 mm.
    let slant = f64::hypot(height, 5.0);
    let pi = std::f64::consts::PI;
    let lofts = [
        (
            "square-frustum-loft",
            Loft::from_profiles(
                [(Placement3::IDENTITY, square(10.0)?), (top, square(5.0)?)],
                Placement3::IDENTITY,
                &policy,
            )?,
            // h / 3 (A + a + sqrt(A a)), and two squares and four
            // trapezoids.
            height / 3.0 * (400.0 + 100.0 + 200.0),
            400.0 + 100.0 + 4.0 * 0.5 * (20.0 + 10.0) * slant,
        ),
        (
            "circular-frustum-loft",
            Loft::from_profiles(
                [(Placement3::IDENTITY, circle(10.0)?), (top, circle(5.0)?)],
                Placement3::IDENTITY,
                &policy,
            )?,
            pi * height / 3.0 * (100.0 + 50.0 + 25.0),
            pi * (100.0 + 25.0) + pi * (10.0 + 5.0) * slant,
        ),
    ];
    for (name, loft, volume, area) in lofts {
        let document = export_body(
            loft.body(),
            Placement3::IDENTITY,
            &StepOptions {
                name,
                timestamp: "2026-01-01T00:00:00Z",
                ..StepOptions::default()
            },
        )?;
        save(
            output,
            expected,
            name,
            &document,
            volume,
            area,
            [[-10.0, -10.0, 0.0], [10.0, 10.0, height]],
        )?;
        expected.last_mut().expect("loft fixture was just saved")["surface_counts"] =
            serde_json::json!({"plane": 2, "bspline": 4});
    }
    Ok(())
}

fn save(
    output: &std::path::Path,
    expected: &mut Vec<serde_json::Value>,
    name: &str,
    document: &StepDocument,
    volume: f64,
    area: f64,
    bounds: [[f64; 3]; 2],
) -> Result<(), Box<dyn Error>> {
    let file = format!("{name}.step");
    std::fs::write(output.join(&file), &document.text)?;
    expected.push(serde_json::json!({"file":file,"native_volume":volume,"native_area":area,"bounds":bounds,
        "metres_per_unit":document.report.units.metres_per_unit(),"faces":document.report.counts.faces,"edges":document.report.counts.edges,
        "vertices":document.report.counts.vertices,
        "serialized_edges":document.report.counts.edges,"serialized_vertices":document.report.counts.vertices,
        "serialized_coedges":document.report.counts.coedges,"serialized_loops":document.report.counts.loops,"max_deviation":document.report.max_deviation,"uncertainty":document.report.declared_uncertainty}));
    Ok(())
}
