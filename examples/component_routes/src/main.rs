// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Builds one plate by hand and from complete tables, then changes a
//! dimension, selects the slot walls, and diagnoses a deliberate failure.

use component_routes::{PlateError, PlateSpec, SLOT, compile_plate, handwritten_plate};
use exedra::constructive::evaluate::evaluate;
use exedra::constructive::ir::ProfileId;
use exedra::constructive::profile::ProfileError;
use exedra::constructive::tessellate::{EvalPolicy, Feature};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = PlateSpec::default();
    let handwritten = handwritten_plate(&spec)?;
    let compiled = compile_plate(&spec)?;
    assert_eq!(
        handwritten.recipe_fingerprint(),
        compiled.recipe_fingerprint(),
        "the same tables in the same order freeze to the same recipe"
    );
    println!(
        "Both routes: recipe {:032x}",
        compiled.recipe_fingerprint().0
    );

    let wider = PlateSpec {
        width: 140.0,
        ..spec
    };
    println!(
        "Width 140: recipe {:032x}",
        compile_plate(&wider)?.recipe_fingerprint().0
    );

    // The slot is hole 1, so its walls are loop 2 of the plate body.
    let result = evaluate(&compiled, &EvalPolicy::default())?;
    let plate = &result.bodies[0];
    let profile = compiled.profile(ProfileId(0)).expect("plate profile");
    let slot = profile
        .holes()
        .iter()
        .position(|hole| hole.segs()[0].tag == Some(SLOT))
        .expect("tagged slot");
    let faces: usize = (0..profile.holes()[slot].segs().len())
        .map(|seg| {
            let feature = Feature::Wall {
                loop_index: 1 + u16::try_from(slot).expect("few holes"),
                seg: u32::try_from(seg).expect("few segments"),
            };
            plate.body.source_map.faces_for(feature).len()
        })
        .sum();
    println!(
        "Slot walls: {faces} faces on {:?}; fidelity {:?}",
        compiled.source_of(plate.node),
        result.report.fidelity_of(plate.node)
    );

    // Too small an end inset pushes the bolt hole through the outline.
    let mut adjusted = PlateSpec {
        end_inset: 2.0,
        ..spec
    };
    match compile_plate(&adjusted) {
        Err(PlateError::Profile(ProfileError::HoleOutsideOuter { hole })) => {
            println!("Refused: hole {hole} escapes the outline; restoring the inset");
            adjusted.end_inset = spec.end_inset;
        }
        other => return Err(format!("expected a refused cutout, got {other:?}").into()),
    }
    compile_plate(&adjusted)?;
    Ok(())
}
