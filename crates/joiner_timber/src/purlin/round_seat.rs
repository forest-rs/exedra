// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Flat seats cut into circular purlins.

use exedra_math::{Real, scale, sub};
use joiner::{
    Applicability, Observation, PartEdit, Rule, RuleContext, RuleError, RuleOutput, TransferEdge,
    TransferKind, TransferTarget,
};

use super::crossing::{
    CrossingFootprint, FRAME_EPSILON, bearing_contact, crossing_footprint, cutter_overrun,
    not_applicable, unsuitable_from_error, validate_bearing_size, validate_complete_crossing,
    validate_cut_depth,
};
use crate::length::default_millimeters;
use crate::participants::{ParticipantPair, resolve_pair};
use crate::tool::{nominal_rect, profile_tool_world};
use crate::{FitClass, Length};

/// Stable identity for a flat underside seat in a circular purlin.
pub const ROUND_PURLIN_SEAT_RULE_KEY: &str = "joiner_timber:round-purlin-seat@1";

/// Dimensions for one shallow seat in a circular purlin.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RoundPurlinSeatParams {
    /// Clearance at the two notch walls along the purlin; the bearing plane
    /// and its depth remain line-to-line.
    pub fit: FitClass,
    /// Height of the finished bearing plane above the purlin's nominal bottom.
    pub seat_depth: Length,
    /// Least uncut purlin depth above the seat, as a geometric safeguard.
    pub minimum_remaining_depth: Length,
    /// Least timber beyond either fit-expanded notch wall and the purlin end.
    pub minimum_end_relish: Length,
    /// Least bearing dimension along and across the purlin.
    pub minimum_bearing: Length,
}

impl Default for RoundPurlinSeatParams {
    fn default() -> Self {
        Self {
            fit: FitClass::CLOSE,
            seat_depth: default_millimeters(15),
            minimum_remaining_depth: default_millimeters(100),
            minimum_end_relish: default_millimeters(50),
            minimum_bearing: default_millimeters(60),
        }
    }
}

/// Cuts a flat underside seat where a round purlin crosses a rectangular support.
///
/// Participants have roles `round-purlin` and `purlin-support`. The purlin's
/// nominal part is a full circular extrusion along local X, with its circle
/// inscribed in the square Y/Z extent. The support has a full rectangular
/// section and crosses orthogonally with both local Z axes aligned. As with
/// other timber rules, custom recipes must honor these nominal section claims;
/// a bounding box alone cannot establish them. Verify the composed geometry
/// separately with [`joiner::contact_geometry`].
///
/// The relation node is at the crossing center, on the support's top face,
/// exactly [`RoundPurlinSeatParams::seat_depth`] above the purlin's bottom.
/// Setout owns those positions; this rule never moves either participant.
/// Only the purlin is cut. The contact rectangle spans the support width and
/// the circle chord at the seat depth, rather than the purlin's full diameter.
/// Seats at or beyond the section center are refused.
#[derive(Copy, Clone, Debug, Default)]
pub struct RoundPurlinSeatRule;

impl Rule for RoundPurlinSeatRule {
    type Params = RoundPurlinSeatParams;

    fn key(&self) -> &str {
        ROUND_PURLIN_SEAT_RULE_KEY
    }

    fn assess(&self, ctx: &RuleContext<'_>) -> Applicability {
        let pair = match resolve_pair(ctx, "round-purlin", "purlin-support") {
            Ok(pair) => pair,
            Err(rejection) => return Applicability::Unsuitable(alloc::vec![rejection]),
        };
        match seat_geometry(&pair) {
            Ok((footprint, _)) => Applicability::Suitable(alloc::vec![Observation::new(
                "round-purlin-seat",
                &pair.carried.key,
                &alloc::format!(
                    "flat bearing is {} m by {} m",
                    footprint.size[0],
                    footprint.size[1]
                ),
            )]),
            Err(error) => unsuitable_from_error(ctx, error),
        }
    }

    fn instantiate(
        &self,
        ctx: &RuleContext<'_>,
        params: &Self::Params,
    ) -> Result<RuleOutput, RuleError> {
        let pair = resolve_pair(ctx, "round-purlin", "purlin-support")
            .map_err(|rejection| RuleError::NotApplicable(alloc::vec![rejection]))?;
        let (footprint, depth) = seat_geometry(&pair)?;
        let purlin = &pair.carried.extent;
        if (depth - params.seat_depth.as_meters()).abs() > FRAME_EPSILON {
            return Err(RuleError::InvalidParameter {
                what: "authored support overlap does not match round-purlin seat depth",
            });
        }
        validate_cut_depth(
            depth,
            params.minimum_remaining_depth.as_meters(),
            purlin.size[2],
            "seat depth",
        )?;
        validate_bearing_size(&footprint, params.minimum_bearing.as_meters())?;
        let half_cut = footprint.size[1] * 0.5 + params.fit.allowance_meters();
        let station = purlin.local_point(pair.node.point)[0];
        if station - half_cut + FRAME_EPSILON < params.minimum_end_relish.as_meters()
            || purlin.size[0] - station - half_cut + FRAME_EPSILON
                < params.minimum_end_relish.as_meters()
        {
            return Err(RuleError::Degenerate {
                what: "round-purlin seat needs timber beyond both notch walls",
            });
        }

        // Span the full diameter so the cut opens through the underside. Its
        // top cap alone defines the bearing plane; clearance widens the notch
        // along the purlin and never sinks the finished seat below its support.
        let overrun = cutter_overrun(params.fit);
        let across_cut = purlin.size[1] + 2.0 * overrun;
        let profile = nominal_rect(across_cut, 2.0 * half_cut)?;
        let origin = sub(
            sub(
                sub(
                    pair.node.point,
                    scale(footprint.tangents[0], across_cut * 0.5),
                ),
                scale(footprint.tangents[1], half_cut),
            ),
            scale(footprint.normal, depth + overrun),
        );
        let relation = &ctx.relation().key;
        let evidence = ctx.relation().evidence.clone();
        let tool = profile_tool_world(
            &alloc::format!("{relation}-round-seat"),
            profile,
            depth + overrun,
            origin,
            [
                footprint.tangents[0],
                footprint.tangents[1],
                footprint.normal,
            ],
            purlin,
        )?;
        let contact = bearing_contact(
            relation,
            &pair,
            &footprint,
            "seated-round-purlin",
            evidence.clone(),
        )
        .with_footprint_meters(footprint.size)
        .with_minimum_overlap_meters([params.minimum_bearing.as_meters(); 2]);
        let mut output = RuleOutput::new();
        output
            .edit(PartEdit::remove(&pair.carried.key, tool, evidence))
            .contact(contact)
            .transfer(TransferEdge::new(
                &alloc::format!("load-{}-through-{relation}", pair.carried.key),
                &pair.carried.key,
                TransferTarget::element(&pair.carrier.key),
                TransferKind::Contact,
            ));
        Ok(output)
    }
}

fn seat_geometry(pair: &ParticipantPair<'_>) -> Result<(CrossingFootprint, f64), RuleError> {
    let purlin = &pair.carried.extent;
    if (purlin.size[1] - purlin.size[2]).abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "round-purlin section needs equal width and depth",
        ));
    }
    let mut footprint = crossing_footprint(pair)?;
    validate_complete_crossing(pair, &footprint)?;
    let depth = purlin.local_point(pair.node.point)[2];
    let support = &pair.carrier.extent;
    if (support.local_point(pair.node.point)[2] - support.size[2]).abs() > FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "joint node is not on the support upper face",
        ));
    }
    let radius = purlin.size[2] * 0.5;
    if depth <= FRAME_EPSILON || depth >= radius - FRAME_EPSILON {
        return Err(not_applicable(
            pair,
            "round-purlin seat must lie strictly below the circular section center",
        ));
    }
    footprint.size[0] = 2.0 * Real::sqrt(depth * (2.0 * radius - depth));
    footprint.origin = sub(
        sub(
            pair.node.point,
            scale(footprint.tangents[0], footprint.size[0] * 0.5),
        ),
        scale(footprint.tangents[1], footprint.size[1] * 0.5),
    );
    Ok((footprint, depth))
}

#[cfg(test)]
mod tests;
