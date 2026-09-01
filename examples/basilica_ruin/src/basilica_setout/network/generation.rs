// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact topology invocations fed by the resolved basilica network.

use setout_generate::{
    InvocationKey, ItemOverride, LinearBayDistribution, LinearBayFragment, LinearDistribution,
    LinearFragment, distribute_linear, distribute_linear_bays,
};

use crate::PlanSection;

use super::BasilicaSetoutError;

pub(super) struct GeneratedArcades {
    pub(super) outer: LinearBayFragment,
    pub(super) west: LinearBayFragment,
    pub(super) east: LinearBayFragment,
}

pub(super) fn generate_arcade_bays(
    plan: &PlanSection,
) -> Result<GeneratedArcades, BasilicaSetoutError> {
    // Each fragment is local to the wall segment that consumes it. Keeping
    // east centers relative to the east segment avoids subtracting two large
    // lowered world coordinates merely to recover a small profile coordinate.
    let outer = generate_arcade_run(
        "basilica/arcades/outer",
        plan.length,
        plan.arcade_bays,
        plan.arcade_end_clearance,
    )?;
    let west = generate_arcade_run(
        "basilica/arcades/west-nave",
        plan.west_nave_length,
        plan.west_arcade_bays,
        plan.arcade_end_clearance,
    )?;
    let east = generate_arcade_run(
        "basilica/arcades/east-nave",
        plan.east_nave_length,
        plan.east_arcade_bays,
        plan.arcade_end_clearance,
    )?;
    Ok(GeneratedArcades { outer, west, east })
}

fn generate_arcade_run(
    invocation: &str,
    length: setout::Length,
    bays: setout::Count,
    end_clearance: setout::Length,
) -> Result<LinearBayFragment, BasilicaSetoutError> {
    let start = setout::Offset::ZERO
        .checked_add_length(end_clearance)
        .ok_or(BasilicaSetoutError::InvalidArcadeExtent)?;
    let end = setout::Offset::ZERO
        .checked_add_length(length)
        .and_then(|end| end.checked_sub_length(end_clearance))
        .ok_or(BasilicaSetoutError::InvalidArcadeExtent)?;
    if start.checked_positive_distance_to(end).is_none() {
        return Err(BasilicaSetoutError::InvalidArcadeExtent);
    }
    let invocation = InvocationKey::new(invocation)?;
    Ok(distribute_linear_bays(&LinearBayDistribution {
        invocation: &invocation,
        start,
        end,
        bays,
        overrides: &[],
    })?)
}

pub(super) fn generate_buttress_stations(
    plan: &PlanSection,
) -> Result<LinearFragment, BasilicaSetoutError> {
    // The network owns the two physical anchors and interval count. The
    // generator owns only their topology: stable item labels and exact
    // rational station coordinates between those resolved inputs.
    if plan
        .buttress_start
        .checked_positive_distance_to(plan.buttress_end)
        .is_none()
    {
        return Err(BasilicaSetoutError::InvalidButtressExtent);
    }
    let invocation = InvocationKey::new("basilica/aisle-buttresses")?;
    Ok(distribute_linear(&LinearDistribution {
        invocation: &invocation,
        start: plan.buttress_start,
        end: plan.buttress_end,
        intervals: plan.arcade_bays,
        overrides: &[],
    })?)
}

pub(super) fn generate_west_truss_stations(
    plan: &PlanSection,
) -> Result<LinearFragment, BasilicaSetoutError> {
    // The missing third station is evidence about this ruin, not a hole in an
    // ordinal loop. Targeting its semantic interior label keeps the absence
    // attached to that identity when the interval count changes; if a smaller
    // run no longer contains it, the generator reports an orphan instead of
    // suppressing an endpoint or a neighboring truss.
    if plan
        .nave_truss_west_start
        .checked_positive_distance_to(plan.nave_truss_west_end)
        .is_none()
    {
        return Err(BasilicaSetoutError::InvalidNaveTrussExtent);
    }
    let invocation = InvocationKey::new("basilica/nave-trusses/west")?;
    let omitted_ruin = ItemOverride::omit("interior/000002")?;
    Ok(distribute_linear(&LinearDistribution {
        invocation: &invocation,
        start: plan.nave_truss_west_start,
        end: plan.nave_truss_west_end,
        intervals: plan.nave_truss_bays,
        overrides: &[omitted_ruin],
    })?)
}
