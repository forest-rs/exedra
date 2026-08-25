// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared analytic geometry for orthogonal secondary-roof crossings.

use exedra_math::{add, dot, scale};
use joiner::{
    Anchor, Applicability, ContactMeaning, ContactPatch, OrientedBox, Rejection, RejectionReason,
    RuleContext, RuleError, Vec3,
};

use crate::FitClass;
use crate::participants::ParticipantPair;
use crate::tool::world_to_local;

pub(super) const FRAME_EPSILON: f64 = 1.0e-9;

/// Finite plan overlap and bearing frame shared by both crossing rules.
pub(super) struct CrossingFootprint {
    pub(super) origin: Vec3,
    pub(super) tangents: [Vec3; 2],
    pub(super) normal: Vec3,
    pub(super) size: [f64; 2],
}

/// Derives the plan rectangle shared by the carried and carrier extents.
///
/// Member centrelines merely locate the relation. The actual bearing size is
/// the finite overlap of the two declared timber extents, so an endpoint
/// purlin that covers only half a principal cannot masquerade as a full
/// through trench.
pub(super) fn crossing_footprint(
    pair: &ParticipantPair<'_>,
) -> Result<CrossingFootprint, RuleError> {
    let carried = &pair.carried.extent;
    let carrier = &pair.carrier.extent;
    // Tangent 0 spans the carried section; tangent 1 follows the carried
    // member. The rule modules use this stable ordering for their distinct
    // end-relish checks.
    let tangents = [carried.axes[1], carried.axes[0]];
    let normal = carried.axes[2];
    if dot(normal, carrier.axes[2]) < 1.0 - FRAME_EPSILON
        || dot(tangents[0], carrier.axes[0]).abs() < 1.0 - FRAME_EPSILON
        || dot(tangents[1], carrier.axes[1]).abs() < 1.0 - FRAME_EPSILON
    {
        return Err(not_applicable(
            pair,
            "members do not form an aligned orthogonal roof crossing",
        ));
    }

    let mut minima = [0.0; 2];
    let mut maxima = [0.0; 2];
    for axis in 0..2 {
        let a = projection_interval(carried, tangents[axis]);
        let b = projection_interval(carrier, tangents[axis]);
        minima[axis] = a.0.max(b.0);
        maxima[axis] = a.1.min(b.1);
        if maxima[axis] - minima[axis] <= FRAME_EPSILON {
            return Err(not_applicable(
                pair,
                "member extents do not overlap in plan",
            ));
        }
        let node_coordinate = dot(pair.node.point, tangents[axis]);
        let centre = (minima[axis] + maxima[axis]) * 0.5;
        if (node_coordinate - centre).abs() > FRAME_EPSILON {
            return Err(not_applicable(
                pair,
                "joint node is not at the crossing footprint centre",
            ));
        }
    }

    let node_coordinates = [
        dot(pair.node.point, tangents[0]),
        dot(pair.node.point, tangents[1]),
    ];
    let origin = add(
        add(
            pair.node.point,
            scale(tangents[0], minima[0] - node_coordinates[0]),
        ),
        scale(tangents[1], minima[1] - node_coordinates[1]),
    );
    Ok(CrossingFootprint {
        origin,
        tangents,
        normal,
        size: [maxima[0] - minima[0], maxima[1] - minima[1]],
    })
}

pub(super) fn validate_complete_crossing(
    pair: &ParticipantPair<'_>,
    footprint: &CrossingFootprint,
) -> Result<(), RuleError> {
    // Both section widths must survive in full. A purlin ending on a
    // principal centreline, for example, could exceed a minimum-area check
    // while cutting only half of the claimed through trench.
    let expected = [pair.carried.extent.size[1], pair.carrier.extent.size[1]];
    if footprint
        .size
        .into_iter()
        .zip(expected)
        .any(|(actual, expected)| (actual - expected).abs() > FRAME_EPSILON)
    {
        return Err(not_applicable(
            pair,
            "joint needs the complete section width of both crossing timbers",
        ));
    }
    Ok(())
}

pub(super) fn validate_cut_depth(
    cut_depth: f64,
    minimum_remaining_depth: f64,
    receiver_depth: f64,
    cut_name: &'static str,
) -> Result<(), RuleError> {
    // The two authored values were positive, finite Lengths before lowering.
    // Only their relationship to the floating receiver depth remains to be
    // checked here.
    if cut_depth >= receiver_depth {
        return Err(RuleError::InvalidParameter { what: cut_name });
    }
    if receiver_depth - cut_depth < minimum_remaining_depth {
        return Err(RuleError::Degenerate {
            what: "cut leaves insufficient receiver depth",
        });
    }
    Ok(())
}

pub(super) fn validate_bearing_size(
    footprint: &CrossingFootprint,
    minimum: f64,
) -> Result<(), RuleError> {
    if footprint.size.into_iter().any(|size| size < minimum) {
        return Err(RuleError::Degenerate {
            what: "crossing bearing is smaller than the requested minimum",
        });
    }
    Ok(())
}

pub(super) fn bearing_contact(
    relation: &str,
    pair: &ParticipantPair<'_>,
    footprint: &CrossingFootprint,
    detail: &str,
    evidence: joiner::Evidence,
) -> ContactPatch {
    ContactPatch::new(
        &alloc::format!("contact-{relation}"),
        Anchor::new(
            &pair.carried.key,
            world_to_local(&pair.carried.extent, pair.node.point),
        ),
        Anchor::new(
            &pair.carrier.key,
            world_to_local(&pair.carrier.extent, pair.node.point),
        ),
        footprint.normal,
        footprint.tangents,
        ContactMeaning::Bearing,
        evidence,
    )
    .with_detail(detail)
}

pub(super) fn cutter_overrun(fit: FitClass) -> f64 {
    fit.allowance_meters().max(FRAME_EPSILON * 16.0)
}

pub(super) fn not_applicable(pair: &ParticipantPair<'_>, what: &'static str) -> RuleError {
    RuleError::NotApplicable(alloc::vec![Rejection::new(
        &pair.node.key,
        RejectionReason::Unsupported { what },
    )])
}

pub(super) fn unsuitable_from_error(ctx: &RuleContext<'_>, error: RuleError) -> Applicability {
    match error {
        RuleError::NotApplicable(rejections) => Applicability::Unsuitable(rejections),
        RuleError::InvalidParameter { what } | RuleError::Degenerate { what } => {
            Applicability::unsuitable(&ctx.relation().key, RejectionReason::Unsupported { what })
        }
        RuleError::Recipe(_) => Applicability::unsuitable(
            &ctx.relation().key,
            RejectionReason::Unsupported {
                what: "crossing tool could not be derived",
            },
        ),
        _ => Applicability::unsuitable(
            &ctx.relation().key,
            RejectionReason::Unsupported {
                what: "crossing geometry could not be derived",
            },
        ),
    }
}

fn projection_interval(extent: &OrientedBox, axis: Vec3) -> (f64, f64) {
    let centre = dot(extent.center(), axis);
    let radius = (0..3)
        .map(|index| extent.size[index] * 0.5 * dot(extent.axes[index], axis).abs())
        .sum::<f64>();
    (centre - radius, centre + radius)
}
