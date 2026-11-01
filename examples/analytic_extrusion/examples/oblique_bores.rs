// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Normal, moderate and steep native cylindrical bores with exact ellipse rims.

mod support;
use exedra::{
    analytic::{
        bore::{Bore, BorePolicy},
        extrusion::Extrusion,
        realize::RealizationPolicy,
    },
    constructive::{builders, ir::Placement3},
    curves::profile::{ProfilePolicy, SegTag},
};
use std::error::Error;
use support::{Exhibit, export};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "oblique-bores.glb".into());
    let mut exhibits = Vec::new();
    for (i, degrees) in [0.0_f64, 30.0, 60.0].into_iter().enumerate() {
        let theta = degrees.to_radians();
        let profile = builders::rect_centered(70.0, 50.0)?
            .tagged(SegTag(1))
            .to_profile(&ProfilePolicy::new(1e-8))?;
        let plate = Extrusion::new(profile, Placement3::IDENTITY, 20.0)?;
        // Center the complete oblique void through the slab's middle plane.
        let bore = Bore::new(
            [-10.0 * theta.tan(), 0.0, 0.0],
            [theta.sin(), 0.0, theta.cos()],
            6.0,
        )?
        .tagged(SegTag(10 + u32::try_from(i)?));
        let result = plate.with_through_bore(
            bore,
            &BorePolicy {
                separation: 0.1,
                geometry_gap: 1e-8,
                min_axis_dot: 0.1,
            },
        )?;
        let realized = result.body().realize_with_regions(
            result.frame(),
            &RealizationPolicy {
                tolerance: 0.025,
                geometry_gap: 1e-8,
                ..Default::default()
            },
            |face, _| u32::from(face == result.report().wall),
        )?;
        assert!(
            realized.report.within_tolerance(),
            "display accuracy: {:?}",
            realized.report
        );
        println!(
            "{}",
            serde_json::json!({"axis_degrees":degrees,"volume":result.volume(),"area":result.surface_area(),
            "native_faces":result.body().faces().len(),"native_edges":result.body().edges().len(),
            "rim_edges":result.report().rims.map(|e|e.0),"wall":result.report().wall.0,
            "minimum_material_clearance":result.report().min_clearance,"native_agreement":result.report().max_gap,
            "display_deviation":realized.report.max_deviation,"display_triangles":realized.mesh_faces.len()})
        );
        exhibits.push(Exhibit {
            name: match i {
                0 => "normal bore",
                1 => "30 degree exact bore",
                _ => "60 degree exact bore",
            },
            mesh: realized.mesh,
            placement: Placement3::translate((f64::from(u32::try_from(i)?) - 1.0) * 85.0, 0.0, 0.0),
            color: match i {
                0 => [0.18, 0.42, 0.66, 1.0],
                1 => [0.12, 0.52, 0.42, 1.0],
                _ => [0.75, 0.46, 0.24, 1.0],
            },
            grid: false,
            cut_regions: &[1],
        });
    }
    export(&path, "exact cylindrical bores and ellipse rims", exhibits)?;
    Ok(())
}
