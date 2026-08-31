// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Exact topology invocations fed by the resolved basilica network.

use setout_generate::{InvocationKey, LinearDistribution, LinearFragment, distribute_linear};

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
