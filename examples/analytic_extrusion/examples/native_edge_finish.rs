// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact selected axial chamfers and fillets with native transition provenance.

mod support;
use exedra::{
    Mesh,
    analytic::{
        End,
        extrusion::{Extrusion, Pocket, TessellationPolicy},
    },
    constructive::{builders, ir::Placement3},
    curves::profile::{CornerTreatment, ProfilePolicy, SegTag},
};
use std::error::Error;
use support::{Exhibit, export};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PROFILE: ProfilePolicy = ProfilePolicy::new(1e-7);
const BODY: SegTag = SegTag(1);
const HOLE: SegTag = SegTag(2);
const FINISH: SegTag = SegTag(3);
const RECESS: SegTag = SegTag(4);

fn finished(fillet: bool, hollow: bool) -> Result<Mesh> {
    let outline = builders::rect_centered(70.0, 50.0)?.tagged(BODY);
    let outline = if hollow {
        outline.with_cutouts_at(
            &builders::rect_centered(32.0, 22.0)?.tagged(HOLE),
            [(0.0, 0.0)],
        )?
    } else {
        outline
    };
    let mut original = Extrusion::from_profile(&outline, Placement3::IDENTITY, 35.0, &PROFILE)?;
    if !hollow {
        let recess = builders::slot_centered(36.0, 8.0)?.to_profile(&PROFILE)?;
        original = original.with_pocket(
            Pocket::from_region(recess.outer(), End::End, 12.0).tagged(RECESS),
            &PROFILE,
        )?;
    }
    let label = if hollow { HOLE } else { BODY };
    let treatment = if fillet {
        CornerTreatment::fillet(9.0)
    } else {
        CornerTreatment::chamfer(7.0)
    };
    let mut output = original.with_axial_corners(
        original.profile_axial_edges_tagged(label).map(|(id, _)| id),
        treatment.tagged(FINISH),
        &PROFILE,
    )?;
    println!(
        "{} selected edges → {} exact transition walls; volume {:.3}",
        output.transitions.len(),
        output.transitions.len(),
        output.solid.volume()
    );
    if hollow {
        output = output.solid.with_axial_corners(
            output
                .solid
                .profile_axial_edges_tagged(BODY)
                .map(|(id, _)| id),
            CornerTreatment::fillet(5.0).tagged(FINISH),
            &PROFILE,
        )?;
    }
    let solid = output.solid;
    let realized = solid.tessellate_with_regions(
        &TessellationPolicy {
            chord_tolerance: 0.03,
            ..TessellationPolicy::default()
        },
        |id, _| u32::from(solid.face_tag(id) == Some(FINISH)),
    )?;
    assert!(
        realized.report.within_tolerance(),
        "finished native tolerance"
    );
    Ok(realized.mesh)
}
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "native-edge-finishes.glb".to_owned());
    export(
        &path,
        "selected native axial finishes",
        vec![
            Exhibit {
                name: "selected chamfers with blind recess",
                mesh: finished(false, false)?,
                placement: Placement3::translate(-95.0, 0.0, 0.0),
                color: [0.18, 0.42, 0.66, 1.0],
                grid: false,
                cut_regions: &[1],
            },
            Exhibit {
                name: "selected tangent fillets with blind recess",
                mesh: finished(true, false)?,
                placement: Placement3::IDENTITY,
                color: [0.12, 0.52, 0.42, 1.0],
                grid: false,
                cut_regions: &[1],
            },
            Exhibit {
                name: "inner chamfers and outer fillets",
                mesh: finished(false, true)?,
                placement: Placement3::translate(95.0, 0.0, 0.0),
                color: [0.75, 0.59, 0.29, 1.0],
                grid: false,
                cut_regions: &[1],
            },
        ],
    )
}
