// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Builds one plate by hand and through constructive builders, measures it
//! exactly, realizes it, compiles the mesh, and diagnoses two refusals.

use analytic_extrusion::{PlateError, PlateSpec, compiled_profile, extrude, handwritten_profile};
use exedra::analytic::extrusion::{TessellateExtrusionError, TessellationPolicy};
use exedra::compile::{CompilePolicy, PartCompiler, PartInput, PartSource};
use exedra::curves::profile::{CircularSamplingError, ProfileError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = PlateSpec::default();
    let solid = extrude(handwritten_profile(&spec)?, &spec)?;
    assert_eq!(
        solid,
        extrude(compiled_profile(&spec)?, &spec)?,
        "both routes describe the same solid"
    );
    println!(
        "Native: {} faces, volume {:.6}, surface {:.6}, bounds {:?}",
        solid.faces().len(),
        solid.volume(),
        solid.surface_area(),
        solid.local_bounds()
    );

    let realized = solid.tessellate(&TessellationPolicy::default())?;
    println!(
        "Realized: {} mesh faces, deviation {:.2e} within {:.0e}: {}",
        realized.mesh.faces().count(),
        realized.report.max_deviation,
        realized.report.chord_tolerance,
        realized.report.within_tolerance()
    );

    // The mesh compiles as baked geometry; the realization report and
    // correspondence stay with the caller.
    let source = PartSource::Baked(realized.mesh.clone());
    let compiled = PartCompiler::new().compile_parts(
        &[PartInput::new("plate", &source)],
        &CompilePolicy::default(),
    )?;
    println!(
        "Compiled: {} triangles",
        compiled.parts()[0].triangle_count()
    );

    // A tolerance the per-arc budget cannot meet is refused, not coarsened.
    let strict = TessellationPolicy {
        chord_tolerance: 1e-6,
        max_segment_edges: 256,
        ..TessellationPolicy::default()
    };
    let required = match solid.tessellate(&strict) {
        Err(TessellateExtrusionError::Sampling {
            segment,
            error: error @ CircularSamplingError::ToleranceBudgetExceeded { required, .. },
        }) => {
            println!("Refused: {segment:?}: {error}; raising the budget");
            required
        }
        other => return Err(format!("expected a sampling refusal, got {other:?}").into()),
    };
    // The samples now meet the tolerance, but narrowing to f32 at this scale
    // moves vertices further; the report says so instead of claiming it.
    let fine = solid.tessellate(&TessellationPolicy {
        max_segment_edges: required,
        ..strict
    })?;
    println!(
        "Fine: deviation {:.2e} within {:.0e}: {}",
        fine.report.max_deviation,
        fine.report.chord_tolerance,
        fine.report.within_tolerance()
    );

    // A hole moved through the outline is named by its segments.
    match handwritten_profile(&PlateSpec {
        hole_x: 37.0,
        ..spec
    }) {
        Err(PlateError::Profile(ProfileError::Contact { first, second })) => {
            println!("Refused: {first:?} meets {second:?}");
        }
        other => return Err(format!("expected a contact, got {other:?}").into()),
    }
    Ok(())
}
