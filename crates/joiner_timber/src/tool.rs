// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Constructive tool building at the rule-library boundary.

use exedra_constructive::builders;
use exedra_constructive::ir::{CapMode, NodeKind, Placement3, RecipeBuilder};
use exedra_constructive::offset::CornerPolicy;
use exedra_constructive::profile::Profile2;
use exedra_math::{dot, sub};
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
        world_frame_in_target(origin, axes, target),
    ))
}

/// Converts a world point into an orthonormal extent's local coordinates.
pub(crate) fn world_to_local(extent: &OrientedBox, point: Vec3) -> Vec3 {
    let delta = sub(point, extent.origin);
    [
        dot(delta, extent.axes[0]),
        dot(delta, extent.axes[1]),
        dot(delta, extent.axes[2]),
    ]
}

fn direction_to_local(extent: &OrientedBox, direction: Vec3) -> Vec3 {
    [
        dot(direction, extent.axes[0]),
        dot(direction, extent.axes[1]),
        dot(direction, extent.axes[2]),
    ]
}

/// An orthonormal frame in world coordinates, expressed relative to a target
/// extent. The transpose is the inverse for both right- and left-handed
/// orthonormal frames, so reflected extents need no special case here.
fn world_frame_in_target(origin: Vec3, axes: [Vec3; 3], target: &OrientedBox) -> Placement3 {
    let mut placement = Placement3::from_axes(
        direction_to_local(target, axes[0]),
        direction_to_local(target, axes[1]),
        direction_to_local(target, axes[2]),
        world_to_local(target, origin),
    );
    // Fingerprints distinguish -0.0 even though placements do not. Normalize
    // signed zero at this sole world-to-local lowering boundary.
    for value in placement.rows.iter_mut().flatten() {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    placement
}
