// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive tool building at the rule-library boundary.

use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
use exedra_constructive::offset::CornerPolicy;
use exedra_constructive::profile::Profile2;
use joiner::{OrientedBox, RuleError, ToolSolid, Vec3};

use crate::FitClass;

pub(crate) fn nominal_rect(width: f64, depth: f64) -> Result<Profile2, RuleError> {
    builders::rect(width, depth).map_err(|_| RuleError::InvalidParameter {
        what: "nominal interface dimensions",
    })
}

pub(crate) fn receiving_profile(nominal: &Profile2, fit: FitClass) -> Result<Profile2, RuleError> {
    let clearance = fit.allowance_meters();
    nominal
        .offset(clearance, CornerPolicy::Miter { limit: 2.0 })
        .map_err(|_| RuleError::InvalidParameter {
            what: "offset receiving profile",
        })
}

/// Builds an extruded profile and maps its world-space frame into `target`.
pub(crate) fn profile_tool_world(
    key: &str,
    profile: Profile2,
    height: f64,
    origin: Vec3,
    axes: [Vec3; 3],
    target: &OrientedBox,
) -> Result<ToolSolid, RuleError> {
    // Rule dimensions are exact Lengths, but extrusion depth is normally a
    // derived sum or intersection distance involving floating-point extents.
    // Validate that geometry result at the last boundary before construction.
    if !(height.is_finite() && height > 0.0) {
        return Err(RuleError::InvalidParameter {
            what: "tool extrusion depth",
        });
    }
    let mut builder = RecipeBuilder::new();
    let source = builder.source_ref(&alloc::format!("joiner_timber:tool/{key}"));
    let profile = builder.add_profile(profile);
    let node = builder.with_source(source).add(NodeKind::Extrude {
        profile,
        placement: Placement3::IDENTITY,
        height,
        caps: CapMode::Both,
    })?;
    let recipe = builder.finish(node)?;
    Ok(ToolSolid::new(
        key,
        recipe,
        target.local_placement(Placement3::from_axes(axes[0], axes[1], axes[2], origin)),
    ))
}
