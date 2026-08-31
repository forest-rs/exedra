// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact topology invocations fed by the resolved basilica network.

use setout_generate::{
    InvocationKey, ItemOverride, LinearDistribution, LinearFragment, distribute_linear,
};

use crate::PlanSection;

use super::BasilicaSetoutError;

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
